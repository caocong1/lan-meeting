//! Video streaming module
//! Handles capture → encode → send and receive → decode pipelines

use crate::capture::ScreenCapture;
use crate::decoder::{DecodedFrame, DecoderConfig, OutputFormat, VideoDecoder};
use crate::encoder::{EncoderConfig, EncoderPreset, FrameType};
use crate::network::protocol::{self, Message};
use crate::network::quic;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

/// Streaming errors
#[derive(Debug, thiserror::Error)]
pub enum StreamingError {
    #[error("Capture error: {0}")]
    CaptureError(String),
    #[error("Encoder error: {0}")]
    EncoderError(String),
    #[error("Decoder error: {0}")]
    DecoderError(String),
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Not streaming")]
    NotStreaming,
}

/// Configuration for streaming
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    pub fps: u32,
    pub quality: Quality,
    pub display_id: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum Quality {
    Auto,
    High,   // 8 Mbps
    Medium, // 4 Mbps
    Low,    // 2 Mbps
}

impl Quality {
    pub fn bitrate(&self) -> u32 {
        match self {
            Quality::Auto | Quality::High => 8_000_000,
            Quality::Medium => 4_000_000,
            Quality::Low => 2_000_000,
        }
    }
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            fps: 30,
            quality: Quality::Auto,
            display_id: 0,
        }
    }
}

/// Global streaming manager
static STREAMING_MANAGER: once_cell::sync::Lazy<Arc<RwLock<Option<StreamingManager>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(None)));

/// Get or create the streaming manager
pub fn get_streaming_manager() -> Arc<RwLock<Option<StreamingManager>>> {
    STREAMING_MANAGER.clone()
}

/// Streaming manager for the sending side
pub struct StreamingManager {
    is_streaming: Arc<AtomicBool>,
    frame_count: Arc<AtomicU32>,
    config: StreamingConfig,
    width: u32,
    height: u32,
    stop_tx: Option<mpsc::Sender<()>>,
}

impl StreamingManager {
    pub fn new() -> Self {
        Self {
            is_streaming: Arc::new(AtomicBool::new(false)),
            frame_count: Arc::new(AtomicU32::new(0)),
            config: StreamingConfig::default(),
            width: 0,
            height: 0,
            stop_tx: None,
        }
    }

    /// Start streaming (sync version - spawns background task)
    pub fn start_sync(
        &mut self,
        config: StreamingConfig,
        mut capture: Box<dyn ScreenCapture>,
    ) -> Result<(), StreamingError> {
        if self.is_streaming.load(Ordering::SeqCst) {
            return Ok(()); // Already streaming
        }

        log::info!("Starting streaming with config: {:?}", config);

        // Get display info
        let displays = capture
            .get_displays()
            .map_err(|e| StreamingError::CaptureError(e.to_string()))?;

        let display = displays
            .iter()
            .find(|d| d.id == config.display_id)
            .or_else(|| displays.first())
            .ok_or_else(|| StreamingError::CaptureError("No display found".to_string()))?;

        self.width = display.width;
        self.height = display.height;
        self.config = config.clone();

        // Start capture
        capture
            .start(config.display_id)
            .map_err(|e| StreamingError::CaptureError(e.to_string()))?;

        // Create encoder
        let mut encoder = crate::encoder::create_encoder()
            .map_err(|e| StreamingError::EncoderError(e.to_string()))?;

        let encoder_config = EncoderConfig {
            width: self.width,
            height: self.height,
            fps: config.fps,
            bitrate: config.quality.bitrate(),
            max_bitrate: config.quality.bitrate() * 2,
            keyframe_interval: config.fps, // 1 keyframe per second
            preset: EncoderPreset::UltraFast,
        };

        encoder
            .init(encoder_config)
            .map_err(|e| StreamingError::EncoderError(e.to_string()))?;

        log::info!(
            "Encoder initialized: {} ({}x{} @ {} fps)",
            encoder.info(),
            self.width,
            self.height,
            config.fps
        );

        // Create stop channel
        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        self.stop_tx = Some(stop_tx);

        // Set streaming flag
        self.is_streaming.store(true, Ordering::SeqCst);

        let is_streaming = self.is_streaming.clone();
        let frame_count = self.frame_count.clone();
        let fps = config.fps;
        let width = self.width;
        let height = self.height;

        // Spawn streaming task
        tokio::spawn(async move {
            // Send ScreenStart to all connected peers
            let start_msg = Message::ScreenStart {
                width,
                height,
                fps: fps as u8,
                codec: "h264".to_string(),
            };

            if let Ok(encoded) = protocol::encode(&start_msg) {
                let _ = quic::broadcast_message(&encoded).await;
            }

            let frame_interval = Duration::from_micros(1_000_000 / fps as u64);
            let mut last_frame_time = std::time::Instant::now();
            let mut sequence: u32 = 0;

            loop {
                // Check for stop signal
                if stop_rx.try_recv().is_ok() {
                    log::info!("Streaming stopped by request");
                    break;
                }

                if !is_streaming.load(Ordering::SeqCst) {
                    break;
                }

                // Frame rate limiting
                let elapsed = last_frame_time.elapsed();
                if elapsed < frame_interval {
                    tokio::time::sleep(frame_interval - elapsed).await;
                }
                last_frame_time = std::time::Instant::now();

                // Capture frame
                let frame = match capture.capture_frame() {
                    Ok(f) => f,
                    Err(e) => {
                        log::warn!("Capture error: {}", e);
                        continue;
                    }
                };

                // Get timestamp
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                // Encode frame
                let encoded = match encoder.encode(&frame.data, timestamp) {
                    Ok(e) => e,
                    Err(e) => {
                        log::warn!("Encode error: {}", e);
                        continue;
                    }
                };

                // Create ScreenFrame message
                let frame_msg = Message::ScreenFrame {
                    timestamp,
                    frame_type: match encoded.frame_type {
                        FrameType::KeyFrame => protocol::FrameType::KeyFrame,
                        FrameType::Delta => protocol::FrameType::DeltaFrame,
                    },
                    sequence,
                    data: encoded.data,
                };

                // Send to all connected peers
                if let Ok(encoded_msg) = protocol::encode(&frame_msg) {
                    let _ = quic::broadcast_message(&encoded_msg).await;
                }

                sequence = sequence.wrapping_add(1);
                frame_count.fetch_add(1, Ordering::Relaxed);
            }

            // Clean up
            let _ = capture.stop();
            is_streaming.store(false, Ordering::SeqCst);

            // Send ScreenStop to all peers
            let stop_msg = Message::ScreenStop;
            if let Ok(encoded) = protocol::encode(&stop_msg) {
                let _ = quic::broadcast_message(&encoded).await;
            }

            log::info!("Streaming task ended");
        });

        Ok(())
    }

    /// Stop streaming (sync version)
    pub fn stop_sync(&mut self) {
        log::info!("Stopping streaming");

        self.is_streaming.store(false, Ordering::SeqCst);

        // Send stop signal (non-blocking)
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.try_send(());
        }
    }

    /// Check if streaming
    pub fn is_streaming(&self) -> bool {
        self.is_streaming.load(Ordering::SeqCst)
    }

    /// Get frame count
    pub fn frame_count(&self) -> u32 {
        self.frame_count.load(Ordering::Relaxed)
    }

    /// Get current config
    pub fn config(&self) -> &StreamingConfig {
        &self.config
    }

    /// Get dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Viewer session for the receiving side
pub struct ViewerSession {
    peer_ip: String,
    decoder: Box<dyn VideoDecoder>,
    width: u32,
    height: u32,
    is_active: bool,
    frame_tx: mpsc::Sender<DecodedFrame>,
}

impl ViewerSession {
    pub fn new(peer_ip: String, frame_tx: mpsc::Sender<DecodedFrame>) -> Result<Self, StreamingError> {
        let decoder = crate::decoder::create_decoder()
            .map_err(|e| StreamingError::DecoderError(e.to_string()))?;

        Ok(Self {
            peer_ip,
            decoder,
            width: 0,
            height: 0,
            is_active: false,
            frame_tx,
        })
    }

    /// Handle ScreenStart message
    pub fn handle_screen_start(
        &mut self,
        width: u32,
        height: u32,
        _fps: u8,
        _codec: &str,
    ) -> Result<(), StreamingError> {
        log::info!(
            "Viewer session started: {}x{} from {}",
            width,
            height,
            self.peer_ip
        );

        self.width = width;
        self.height = height;

        let config = DecoderConfig {
            width,
            height,
            output_format: OutputFormat::BGRA,
        };

        self.decoder
            .init(config)
            .map_err(|e| StreamingError::DecoderError(e.to_string()))?;

        self.is_active = true;
        Ok(())
    }

    /// Handle ScreenFrame message
    pub async fn handle_screen_frame(
        &mut self,
        timestamp: u64,
        data: &[u8],
    ) -> Result<(), StreamingError> {
        if !self.is_active {
            return Err(StreamingError::NotStreaming);
        }

        // Decode frame
        if let Some(frame) = self
            .decoder
            .decode(data, timestamp)
            .map_err(|e| StreamingError::DecoderError(e.to_string()))?
        {
            // Send to renderer
            let _ = self.frame_tx.send(frame).await;
        }

        Ok(())
    }

    /// Handle ScreenStop message
    pub fn handle_screen_stop(&mut self) {
        log::info!("Viewer session stopped for {}", self.peer_ip);
        self.is_active = false;
    }

    /// Check if active
    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// Get dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Global viewer sessions
static VIEWER_SESSIONS: once_cell::sync::Lazy<Arc<RwLock<HashMap<String, ViewerSession>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Get viewer sessions
pub fn get_viewer_sessions() -> Arc<RwLock<HashMap<String, ViewerSession>>> {
    VIEWER_SESSIONS.clone()
}

/// Create a viewer session for a peer
pub fn create_viewer_session(
    peer_ip: String,
    frame_tx: mpsc::Sender<DecodedFrame>,
) -> Result<(), StreamingError> {
    let session = ViewerSession::new(peer_ip.clone(), frame_tx)?;
    VIEWER_SESSIONS.write().insert(peer_ip, session);
    Ok(())
}

/// Remove a viewer session
pub fn remove_viewer_session(peer_ip: &str) {
    VIEWER_SESSIONS.write().remove(peer_ip);
}

/// Request screen stream from a peer
pub async fn request_screen_stream(peer_ip: &str, display_id: u32) -> Result<(), StreamingError> {
    let request_msg = Message::ScreenRequest {
        display_id,
        preferred_fps: 30,
        preferred_quality: 80,
    };

    let encoded = protocol::encode(&request_msg)
        .map_err(|e| StreamingError::NetworkError(e.to_string()))?;

    quic::send_to_peer(peer_ip, &encoded)
        .await
        .map_err(|e| StreamingError::NetworkError(e.to_string()))?;

    Ok(())
}

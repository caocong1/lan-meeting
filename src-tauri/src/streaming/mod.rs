//! Video streaming module
//! Handles capture → encode → send and receive → decode pipelines

use crate::capture::ScreenCapture;
use crate::decoder::{DecoderConfig, OutputFormat, VideoDecoder};
use crate::diagnostics;
use crate::encoder::scaler::FrameScaler;
use crate::encoder::{EncoderConfig, EncoderPreset, FrameType};
use crate::network::protocol::{self, Message};
use crate::network::quic;
use crate::renderer::{RenderFrame, RenderWindow, RenderWindowHandle};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

const DEFAULT_TARGET_WIDTH: u32 = 2048;
const DEFAULT_TARGET_HEIGHT: u32 = 1280;
const SOFTWARE_HIGH_RES_FPS_CAP: u32 = 15;
const HIGH_RES_PIXEL_THRESHOLD: u32 = 1920 * 1080;
const MAX_VIEWER_FRAME_AGE_MS: u64 = 1_500;

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
            Quality::Auto | Quality::High => 12_000_000,
            Quality::Medium => 6_000_000,
            Quality::Low => 3_000_000,
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

        // Start capture
        capture
            .start(config.display_id)
            .map_err(|e| StreamingError::CaptureError(e.to_string()))?;

        // Downscale before encoding so all encoders see a predictable, decoder-friendly size.
        // A 2K-ish 16:10 target keeps high-DPI desktop text more readable than
        // 1080p without pushing the software encoder into multi-second lag.
        let pre_scaler = FrameScaler::new_with_target(
            self.width,
            self.height,
            DEFAULT_TARGET_WIDTH,
            DEFAULT_TARGET_HEIGHT,
        );
        let target_width = pre_scaler.dst_width;
        let target_height = pre_scaler.dst_height;

        // Create encoder
        let mut encoder = crate::encoder::create_encoder()
            .map_err(|e| StreamingError::EncoderError(e.to_string()))?;

        let encoder_info = encoder.info().to_string();
        let mut effective_fps = config.fps.max(1);
        let target_pixels = target_width.saturating_mul(target_height);
        if encoder_info.contains("Software")
            && target_pixels > HIGH_RES_PIXEL_THRESHOLD
            && effective_fps > SOFTWARE_HIGH_RES_FPS_CAP
        {
            log::info!(
                "Capping software high-resolution stream fps: requested={} effective={}",
                effective_fps,
                SOFTWARE_HIGH_RES_FPS_CAP
            );
            effective_fps = SOFTWARE_HIGH_RES_FPS_CAP;
        }

        let encoder_config = EncoderConfig {
            width: target_width,
            height: target_height,
            fps: effective_fps,
            bitrate: config.quality.bitrate(),
            max_bitrate: config.quality.bitrate() * 2,
            keyframe_interval: effective_fps, // 1 keyframe per second
            preset: EncoderPreset::UltraFast,
        };

        encoder
            .init(encoder_config)
            .map_err(|e| StreamingError::EncoderError(e.to_string()))?;

        // Get actual encoding dimensions (may be scaled for OpenH264)
        let (encode_width, encode_height) = encoder
            .get_dimensions()
            .unwrap_or((self.width, self.height));

        log::info!(
            "Encoder initialized: {} ({}x{} -> {}x{} @ {} fps)",
            encoder.info(),
            self.width,
            self.height,
            encode_width,
            encode_height,
            effective_fps
        );

        // Expose the encoded dimensions to request-time ScreenStart responses.
        self.width = encode_width;
        self.height = encode_height;
        let mut effective_config = config.clone();
        effective_config.fps = effective_fps;
        self.config = effective_config.clone();
        clear_active_viewers();

        // Create stop channel
        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        self.stop_tx = Some(stop_tx);

        // Set streaming flag
        self.is_streaming.store(true, Ordering::SeqCst);

        let is_streaming = self.is_streaming.clone();
        let frame_count = self.frame_count.clone();
        let fps = effective_config.fps;
        // Use encoded dimensions (may be scaled for OpenH264)
        let width = encode_width;
        let height = encode_height;

        // Spawn streaming task
        tokio::spawn(async move {
            // Send ScreenStart to all connected peers via control streams
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
            let mut last_waiting_log = std::time::Instant::now() - Duration::from_secs(5);
            let mut sequence: u32 = 0;
            let mut last_viewer_epoch = VIEWER_REQUEST_EPOCH.load(Ordering::SeqCst);
            let mut last_raw_frame: Option<Vec<u8>> = None;
            let mut timeout_count: u32 = 0;

            // Frame send bookkeeping. The receiver accepts frame streams
            // independently from long-lived control streams.
            let mut peer_streams: HashMap<String, quinn::SendStream> =
                HashMap::new();

            loop {
                // Check for stop signal
                if stop_rx.try_recv().is_ok() {
                    log::info!("Streaming stopped by request");
                    break;
                }

                if !is_streaming.load(Ordering::SeqCst) {
                    break;
                }

                let active_count = active_viewer_count();
                if active_count == 0 {
                    if last_waiting_log.elapsed() >= Duration::from_secs(5) {
                        log::info!("Streaming is armed, waiting for a viewer request");
                        last_waiting_log = std::time::Instant::now();
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    last_frame_time = std::time::Instant::now();
                    continue;
                }

                let viewer_epoch = VIEWER_REQUEST_EPOCH.load(Ordering::SeqCst);
                if viewer_epoch != last_viewer_epoch {
                    encoder.request_keyframe();
                    last_viewer_epoch = viewer_epoch;
                    log::info!("Viewer request changed; forcing next stream frame to keyframe");
                }

                // Frame rate limiting
                let elapsed = last_frame_time.elapsed();
                if elapsed < frame_interval {
                    tokio::time::sleep(frame_interval - elapsed).await;
                }
                last_frame_time = std::time::Instant::now();

                // Capture frame. On Windows, DXGI returns WAIT_TIMEOUT when the
                // desktop is static. Re-encode the last raw frame so a newly
                // opened viewer still receives an initial keyframe.
                let frame_data = match capture.capture_frame() {
                    Ok(frame) => {
                        diagnostics::record_sharer_capture_frame();
                        timeout_count = 0;
                        last_raw_frame = Some(frame.data);
                        last_raw_frame.as_ref().expect("last frame was just stored")
                    }
                    Err(e) => {
                        if e.to_string().contains("Frame timeout") {
                            timeout_count = timeout_count.saturating_add(1);
                            if let Some(frame) = last_raw_frame.as_ref() {
                                if timeout_count == 1 || timeout_count % 150 == 0 {
                                    log::debug!(
                                        "Capture timed out; reusing last raw frame ({} timeouts)",
                                        timeout_count
                                    );
                                }
                                frame
                            } else {
                                if timeout_count == 1 || timeout_count % 30 == 0 {
                                    log::warn!(
                                        "Capture timed out and no previous frame is available yet"
                                    );
                                }
                                continue;
                            }
                        } else {
                            timeout_count = 0;
                            diagnostics::record_sharer_capture_error(&e.to_string());
                            log::warn!("Capture error: {}", e);
                            continue;
                        }
                    }
                };

                // Get timestamp
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                if sequence > 0 && sequence % fps == 0 {
                    encoder.request_keyframe();
                    log::debug!("Forcing periodic keyframe at stream frame {}", sequence);
                }

                // Encode frame
                let scaled_frame = pre_scaler.scale(frame_data);
                let encoded = match encoder.encode(&scaled_frame, timestamp) {
                    Ok(e) => e,
                    Err(e) => {
                        log::warn!("Encode error: {}", e);
                        continue;
                    }
                };
                if encoded.data.is_empty() {
                    diagnostics::record_sharer_empty_packet(sequence);
                    if sequence < 5 || sequence % 100 == 0 {
                        log::debug!("Encoder returned empty packet at frame {}", sequence);
                    }
                    continue;
                }
                diagnostics::record_sharer_encoded_packet(
                    sequence,
                    encoded.size,
                    encoded.frame_type == FrameType::KeyFrame,
                );

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

                // Send to all connected peers using frame streams
                let mut sent_count = 0usize;
                if let Ok(encoded_msg) = protocol::encode(&frame_msg) {
                    sent_count =
                        broadcast_frame(sequence, encoded.size, &encoded_msg, &mut peer_streams)
                            .await;
                }
                if sequence < 5 || sequence % 50 == 0 {
                    log::info!(
                        "Streaming frame {}: {} bytes ({:?}) sent to {} peers",
                        sequence,
                        encoded.size,
                        encoded.frame_type,
                        sent_count
                    );
                }

                sequence = sequence.wrapping_add(1);
                frame_count.fetch_add(1, Ordering::Relaxed);
            }

            peer_streams.clear();

            let _ = capture.stop();
            is_streaming.store(false, Ordering::SeqCst);
            clear_active_viewers();

            // Send ScreenStop to all peers via control streams
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
        clear_active_viewers();

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
/// Uses native wgpu window for efficient GPU rendering
pub struct ViewerSession {
    peer_ip: String,
    peer_name: String,
    decoder: Box<dyn VideoDecoder>,
    window_handle: Option<RenderWindowHandle>,
    width: u32,
    height: u32,
    is_active: bool,
    frame_count: u32,
    dropping_until_keyframe: bool,
    stale_drop_count: u32,
}

impl ViewerSession {
    pub fn new(peer_ip: String, peer_name: String) -> Result<Self, StreamingError> {
        let decoder = crate::decoder::create_decoder()
            .map_err(|e| StreamingError::DecoderError(e.to_string()))?;

        Ok(Self {
            peer_ip,
            peer_name,
            decoder,
            window_handle: None,
            width: 0,
            height: 0,
            is_active: false,
            frame_count: 0,
            dropping_until_keyframe: false,
            stale_drop_count: 0,
        })
    }

    /// Handle ScreenStart message - creates native render window
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

        // Initialize decoder with BGRA output for direct GPU upload
        log::debug!("Initializing decoder for {}x{} BGRA output", width, height);
        let config = DecoderConfig {
            width,
            height,
            output_format: OutputFormat::BGRA,
        };

        self.decoder.init(config).map_err(|e| {
            log::error!("Decoder init failed: {}", e);
            StreamingError::DecoderError(e.to_string())
        })?;
        log::debug!("Decoder initialized successfully");

        // Create native render window
        let title = format!("{} 的屏幕 ({})", self.peer_name, self.peer_ip);
        log::debug!(
            "Creating native render window: '{}' ({}x{})",
            title,
            width,
            height
        );
        let window_handle = RenderWindow::create(&title, width, height).map_err(|e| {
            log::error!("RenderWindow::create failed: {}", e);
            StreamingError::DecoderError(format!("Failed to create window: {}", e))
        })?;

        self.window_handle = Some(window_handle);
        self.is_active = true;
        self.frame_count = 0;
        self.dropping_until_keyframe = false;
        self.stale_drop_count = 0;

        log::info!("Native render window created for {}", self.peer_ip);
        Ok(())
    }

    /// Handle ScreenFrame message - decode and render to native window
    pub fn handle_screen_frame(
        &mut self,
        sequence: u32,
        frame_type: protocol::FrameType,
        timestamp: u64,
        data: &[u8],
    ) -> Result<(), StreamingError> {
        if !self.is_active {
            return Err(StreamingError::NotStreaming);
        }

        // Check if window is still open
        if let Some(ref handle) = self.window_handle {
            if !handle.is_open() {
                log::info!("Render window closed by user");
                self.is_active = false;
                return Err(StreamingError::NotStreaming);
            }
        }

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(timestamp);
        let frame_age_ms = now_ms.saturating_sub(timestamp);
        let is_keyframe = matches!(frame_type, protocol::FrameType::KeyFrame);

        if frame_age_ms > MAX_VIEWER_FRAME_AGE_MS && !is_keyframe {
            self.dropping_until_keyframe = true;
            self.stale_drop_count = self.stale_drop_count.saturating_add(1);
            if self.stale_drop_count <= 3 || sequence % 50 == 0 {
                log::warn!(
                    "Dropping stale screen delta frame {} from {} (age={}ms, dropped={})",
                    sequence,
                    self.peer_ip,
                    frame_age_ms,
                    self.stale_drop_count
                );
            }
            return Ok(());
        }

        if self.dropping_until_keyframe && !is_keyframe {
            self.stale_drop_count = self.stale_drop_count.saturating_add(1);
            if self.stale_drop_count <= 3 || sequence % 50 == 0 {
                log::warn!(
                    "Dropping delta frame {} while waiting for keyframe from {} (age={}ms, dropped={})",
                    sequence,
                    self.peer_ip,
                    frame_age_ms,
                    self.stale_drop_count
                );
            }
            return Ok(());
        }

        if is_keyframe {
            self.dropping_until_keyframe = false;
        }

        if sequence < 5 || sequence % 50 == 0 {
            log::info!(
                "Viewer frame {} from {} age={}ms bytes={} keyframe={}",
                sequence,
                self.peer_ip,
                frame_age_ms,
                data.len(),
                is_keyframe
            );
        }

        // Decode frame
        let decoded = match self.decoder.decode(data, timestamp) {
            Ok(decoded) => decoded,
            Err(e) => {
                diagnostics::record_viewer_decode_error(sequence, &e.to_string());
                return Err(StreamingError::DecoderError(e.to_string()));
            }
        };

        if let Some(decoded) = decoded {
            diagnostics::record_viewer_decoded_frame(sequence);
            // Convert DecodedFrame to RenderFrame based on data type
            let render_frame = if let Some(cpu_data) = decoded.cpu_data() {
                match decoded.format {
                    OutputFormat::BGRA => {
                        RenderFrame::from_bgra(decoded.width, decoded.height, cpu_data.to_vec())
                    }
                    OutputFormat::YUV420 => RenderFrame::from_yuv420(
                        decoded.width,
                        decoded.height,
                        cpu_data.to_vec(),
                        decoded.strides().unwrap_or([
                            decoded.width as usize,
                            decoded.width as usize / 2,
                            decoded.width as usize / 2,
                        ]),
                    ),
                }
            } else {
                // GPU texture path - not yet implemented
                // TODO: When vk-video updates to wgpu 28, add zero-copy texture rendering
                log::warn!("GPU texture frames not yet supported");
                return Ok(());
            };

            // Send to native window for GPU rendering
            if let Some(ref handle) = self.window_handle {
                if let Err(e) = handle.render_frame(render_frame) {
                    diagnostics::record_viewer_render_failure(sequence, &e.to_string());
                    log::warn!("Failed to render frame: {}", e);
                } else {
                    diagnostics::record_viewer_render_success(sequence);
                }
            }

            self.frame_count += 1;
        } else {
            diagnostics::record_viewer_decoder_none(sequence);
        }

        Ok(())
    }

    /// Handle ScreenStop message
    pub fn handle_screen_stop(&mut self) {
        log::info!("Viewer session stopped for {}", self.peer_ip);
        self.is_active = false;

        // Close the render window
        if let Some(ref handle) = self.window_handle {
            handle.close();
        }
    }

    /// Close the viewer session
    pub fn close(&mut self) {
        self.is_active = false;
        if let Some(ref handle) = self.window_handle {
            handle.close();
        }
        self.window_handle = None;
    }

    /// Check if active
    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// Check if window is open
    pub fn is_window_open(&self) -> bool {
        self.window_handle
            .as_ref()
            .map(|h| h.is_open())
            .unwrap_or(false)
    }

    /// Get dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Get frame count
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }
}

/// Global viewer sessions
static VIEWER_SESSIONS: once_cell::sync::Lazy<Arc<RwLock<HashMap<String, ViewerSession>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

static PENDING_SCREEN_STARTS: once_cell::sync::Lazy<
    Arc<RwLock<HashMap<String, (u32, u32, u8, String)>>>,
> = once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

static ACTIVE_VIEWERS: once_cell::sync::Lazy<Arc<RwLock<HashSet<String>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(HashSet::new())));

static VIEWER_REQUEST_EPOCH: AtomicU32 = AtomicU32::new(0);

/// Get viewer sessions
pub fn get_viewer_sessions() -> Arc<RwLock<HashMap<String, ViewerSession>>> {
    VIEWER_SESSIONS.clone()
}

/// Mark a peer as actively watching this screen.
pub fn add_active_viewer(peer_ip: String) {
    VIEWER_REQUEST_EPOCH.fetch_add(1, Ordering::SeqCst);

    let mut viewers = ACTIVE_VIEWERS.write();
    if viewers.insert(peer_ip.clone()) {
        log::info!("Activated screen viewer {}", peer_ip);
    } else {
        log::info!("Refreshed screen viewer {}", peer_ip);
    }
}

/// Remove a peer from the active watching set.
pub fn remove_active_viewer(peer_ip: &str) {
    if ACTIVE_VIEWERS.write().remove(peer_ip) {
        log::info!("Removed active screen viewer {}", peer_ip);
    }
}

/// Clear all active watching state.
pub fn clear_active_viewers() {
    let mut viewers = ACTIVE_VIEWERS.write();
    if !viewers.is_empty() {
        log::info!("Clearing {} active screen viewers", viewers.len());
        viewers.clear();
    }
}

fn active_viewer_count() -> usize {
    ACTIVE_VIEWERS.read().len()
}

fn is_active_viewer(peer_ip: &str) -> bool {
    ACTIVE_VIEWERS.read().contains(peer_ip)
}

/// Store ScreenStart metadata when it arrives before the user opens a viewer.
pub fn store_pending_screen_start(
    peer_ip: String,
    width: u32,
    height: u32,
    fps: u8,
    codec: String,
) {
    PENDING_SCREEN_STARTS
        .write()
        .insert(peer_ip, (width, height, fps, codec));
}

/// Create a viewer session for a peer.
///
/// Returns true when a cached ScreenStart was applied and the native window was
/// opened immediately.
pub fn create_viewer_session(peer_ip: String, peer_name: String) -> Result<bool, StreamingError> {
    let mut session = ViewerSession::new(peer_ip.clone(), peer_name)?;
    let mut started_immediately = false;

    if let Some((width, height, fps, codec)) = PENDING_SCREEN_STARTS.write().remove(&peer_ip) {
        log::info!("Applying pending ScreenStart for {}", peer_ip);
        session.handle_screen_start(width, height, fps, &codec)?;
        started_immediately = true;
    }

    let mut sessions = VIEWER_SESSIONS.write();
    if let Some(mut previous) = sessions.remove(&peer_ip) {
        previous.close();
    }
    sessions.insert(peer_ip, session);
    Ok(started_immediately)
}

/// Remove a viewer session
pub fn remove_viewer_session(peer_ip: &str) {
    let mut sessions = VIEWER_SESSIONS.write();
    if let Some(mut session) = sessions.remove(peer_ip) {
        session.close();
    }
}

/// Request screen stream from a peer
pub async fn request_screen_stream(peer_ip: &str, display_id: u32) -> Result<(), StreamingError> {
    let request_msg = Message::ScreenRequest {
        display_id,
        preferred_fps: 30,
        preferred_quality: 80,
    };

    let encoded =
        protocol::encode(&request_msg).map_err(|e| StreamingError::NetworkError(e.to_string()))?;

    quic::send_to_peer(peer_ip, &encoded)
        .await
        .map_err(|e| StreamingError::NetworkError(e.to_string()))?;

    Ok(())
}

/// Send frame data to active viewers on persistent unidirectional streams.
///
/// Each viewer gets one long-lived uni stream that is reused for every frame.
/// Uni streams are accepted by the dedicated uni accept task in
/// `handle_incoming_connection`, which already handles ScreenFrame messages.
async fn broadcast_frame(
    sequence: u32,
    payload_bytes: usize,
    data: &[u8],
    peer_streams: &mut HashMap<String, quinn::SendStream>,
) -> usize {
    let connections = quic::get_all_connections();
    let mut sent_count = 0usize;
    let mut active_keys = HashSet::new();

    for conn in &connections {
        if !conn.is_alive() {
            continue;
        }

        let peer_ip = conn.remote_addr().ip().to_string();
        if !is_active_viewer(&peer_ip) {
            continue;
        }

        let key = conn.remote_addr().to_string();
        active_keys.insert(key.clone());

        // Remove the stream from the map so we can use it without borrowing conflicts
        let mut stream = match peer_streams.remove(&key) {
            Some(s) => s,
            None => match conn.open_uni_stream().await {
                Ok(s) => {
                    diagnostics::record_frame_stream_opened(&key);
                    log::info!("Opened persistent frame uni stream to {}", key);
                    s
                }
                Err(e) => {
                    diagnostics::record_frame_stream_open_failed(&key, &e.to_string());
                    log::warn!("Failed to open frame uni stream to {}: {}", key, e);
                    continue;
                }
            },
        };

        let send_started = std::time::Instant::now();
        match quic::send_uni_framed(&mut stream, data).await {
            Ok(()) => {
                diagnostics::record_frame_send_success(
                    &key,
                    sequence,
                    payload_bytes,
                    data.len(),
                    send_started.elapsed(),
                );
                sent_count += 1;
                // Put the stream back for reuse
                peer_streams.insert(key, stream);
            }
            Err(e) => {
                diagnostics::record_frame_send_failure(&key, sequence, &e.to_string());
                log::warn!(
                    "Failed to send frame {} to {}: {} (stream will be recreated)",
                    sequence,
                    key,
                    e
                );
                // Stream is dropped — will be recreated next frame
            }
        }
    }

    // Clean up streams for disconnected/stopped viewers
    let stale_keys: Vec<String> = peer_streams
        .keys()
        .filter(|key| !active_keys.contains(*key))
        .cloned()
        .collect();
    for key in stale_keys {
        if let Some(mut stream) = peer_streams.remove(&key) {
            let _ = stream.finish();
        }
    }

    sent_count
}

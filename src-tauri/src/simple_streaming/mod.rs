//! Simple streaming module - minimal screen sharing pipeline
//!
//! Bypasses all complex encoder/decoder selection and optimization.
//! Uses OpenH264 only, single QUIC stream for all messages.
//! Designed to verify basic capture→encode→transmit→decode→render works.

use crate::capture::{self, ScreenCapture};
use crate::decoder::software::SoftwareDecoder;
use crate::decoder::{DecoderConfig, OutputFormat, VideoDecoder};
use crate::encoder::scaler::FrameScaler;
use crate::encoder::{self, EncoderConfig, EncoderPreset, VideoEncoder};
use crate::network::quic::{self, QuicStream};
use crate::renderer::{RenderFrame, RenderWindow, RenderWindowHandle};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

/// Simple message types sent on the persistent stream
const MSG_TYPE_START: u8 = 0x01;
const MSG_TYPE_FRAME: u8 = 0x02;
const MSG_TYPE_STOP: u8 = 0x03;
const MSG_TYPE_RESOLUTION_REQUEST: u8 = 0x04; // viewer → sharer

/// Hardcoded FPS for simplicity
const SIMPLE_FPS: u32 = 30;

/// Target encode resolution (downscale from capture resolution)
const SIMPLE_TARGET_WIDTH: u32 = 1280;
const SIMPLE_TARGET_HEIGHT: u32 = 720;

/// Resolution option for toolbar
#[derive(Debug, Clone, Copy)]
pub struct ResolutionOption {
    pub label: &'static str,
    pub target_width: u32,
    pub target_height: u32,
}

/// Bitrate option for toolbar
#[derive(Debug, Clone, Copy)]
pub struct BitrateOption {
    pub label: &'static str,
    pub bitrate: u32,
}

/// Available resolution options (independent of bitrate)
pub const RESOLUTION_OPTIONS: [ResolutionOption; 4] = [
    ResolutionOption { label: "720p",     target_width: 1280, target_height: 720 },
    ResolutionOption { label: "1080p",    target_width: 1920, target_height: 1080 },
    ResolutionOption { label: "1440p",    target_width: 2560, target_height: 1440 },
    ResolutionOption { label: "Original", target_width: 3840, target_height: 2160 },
];

/// Available bitrate options (independent of resolution)
pub const BITRATE_OPTIONS: [BitrateOption; 4] = [
    BitrateOption { label: "2 Mbps",  bitrate: 2_000_000 },
    BitrateOption { label: "4 Mbps",  bitrate: 4_000_000 },
    BitrateOption { label: "8 Mbps",  bitrate: 8_000_000 },
    BitrateOption { label: "12 Mbps", bitrate: 12_000_000 },
];

// ===== Global state =====

static SIMPLE_SHARER_ACTIVE: once_cell::sync::Lazy<Arc<AtomicBool>> =
    once_cell::sync::Lazy::new(|| Arc::new(AtomicBool::new(false)));

static SIMPLE_STOP_TX: once_cell::sync::Lazy<RwLock<Option<mpsc::Sender<()>>>> =
    once_cell::sync::Lazy::new(|| RwLock::new(None));

/// Check if simple sharer is active
pub fn is_simple_sharing() -> bool {
    SIMPLE_SHARER_ACTIVE.load(Ordering::SeqCst)
}

// ===== Sender side =====

/// Start simple sharing - begins capture and waits for viewer requests
pub fn start_sharing(display_id: u32) -> Result<(), String> {
    if SIMPLE_SHARER_ACTIVE.load(Ordering::SeqCst) {
        log::info!("[SIMPLE] Already sharing, ignoring start request");
        return Ok(());
    }

    log::info!("[SIMPLE] === Starting simple sharing for display {} ===", display_id);

    // Create capture
    let mut capture = capture::create_capture()
        .map_err(|e| format!("[SIMPLE] Failed to create capture: {}", e))?;
    log::info!("[SIMPLE] Capture created successfully");

    // Get display info
    let displays = capture.get_displays()
        .map_err(|e| format!("[SIMPLE] Failed to get displays: {}", e))?;
    log::info!("[SIMPLE] Found {} displays", displays.len());

    let display = displays.iter()
        .find(|d| d.id == display_id)
        .or_else(|| displays.first())
        .ok_or_else(|| "[SIMPLE] No display found".to_string())?;

    let width = display.width;
    let height = display.height;
    log::info!("[SIMPLE] Display: {} ({}x{})", display.name, width, height);

    // Start capture
    capture.start(display_id)
        .map_err(|e| format!("[SIMPLE] Failed to start capture: {}", e))?;
    log::info!("[SIMPLE] Capture started");

    // Create pre-encoder downscaler: capture resolution → target resolution
    let pre_scaler = FrameScaler::new_with_target(width, height, SIMPLE_TARGET_WIDTH, SIMPLE_TARGET_HEIGHT);
    let encode_width = pre_scaler.dst_width;
    let encode_height = pre_scaler.dst_height;
    log::info!("[SIMPLE] Pre-scaler: {}x{} -> {}x{}", width, height, encode_width, encode_height);

    // Create encoder - try hardware first, fall back to software
    let mut encoder = encoder::create_encoder()
        .map_err(|e| format!("[SIMPLE] Failed to create encoder: {}", e))?;
    log::info!("[SIMPLE] Using encoder: {}", encoder.info());

    let encoder_config = EncoderConfig {
        width: encode_width,
        height: encode_height,
        fps: SIMPLE_FPS,
        bitrate: 2_000_000, // 2 Mbps
        max_bitrate: 4_000_000,
        keyframe_interval: SIMPLE_FPS, // 1 keyframe per second
        preset: EncoderPreset::UltraFast,
    };

    encoder.init(encoder_config)
        .map_err(|e| format!("[SIMPLE] Failed to init encoder: {}", e))?;

    log::info!("[SIMPLE] Encoder initialized: {}x{} -> {}x{} @ {} fps",
        width, height, encode_width, encode_height, SIMPLE_FPS);

    // Create stop channel
    let (stop_tx, stop_rx) = mpsc::channel::<()>(1);
    *SIMPLE_STOP_TX.write() = Some(stop_tx);
    SIMPLE_SHARER_ACTIVE.store(true, Ordering::SeqCst);

    log::info!("[SIMPLE] Sharer is now active, waiting for viewer requests");

    // Broadcast that we're sharing (using existing protocol)
    let active = SIMPLE_SHARER_ACTIVE.clone();
    let _ = std::thread::Builder::new()
        .name("simple-sharer-state".to_string())
        .spawn(move || {
            // Store capture/encoder for use when viewer requests come in
            // We put them in a global so handle_simple_request can access them
            let mut state = SHARER_STATE.write();
            *state = Some(SharerState {
                capture,
                pre_scaler,
                encoder,
                encode_width,
                encode_height,
                stop_rx,
                active,
            });
            log::info!("[SIMPLE] Sharer state stored, ready for viewer requests");
        });

    Ok(())
}

/// Internal sharer state
struct SharerState {
    capture: Box<dyn ScreenCapture>,
    pre_scaler: FrameScaler,
    encoder: Box<dyn VideoEncoder>,
    encode_width: u32,
    encode_height: u32,
    stop_rx: mpsc::Receiver<()>,
    active: Arc<AtomicBool>,
}

// Safety: SharerState is only accessed from one thread at a time
unsafe impl Send for SharerState {}
unsafe impl Sync for SharerState {}

static SHARER_STATE: once_cell::sync::Lazy<RwLock<Option<SharerState>>> =
    once_cell::sync::Lazy::new(|| RwLock::new(None));

/// Handle a SimpleScreenRequest from a viewer - starts streaming to them
pub async fn handle_viewer_request(peer_ip: &str) {
    log::info!("[SIMPLE] === Received viewer request from {} ===", peer_ip);

    if !SIMPLE_SHARER_ACTIVE.load(Ordering::SeqCst) {
        log::warn!("[SIMPLE] Not sharing, ignoring viewer request from {}", peer_ip);
        return;
    }

    // Find connection to the viewer
    let conn = match quic::find_connection(peer_ip) {
        Some(c) => c,
        None => {
            log::error!("[SIMPLE] No connection found for viewer {}", peer_ip);
            return;
        }
    };

    // Open a persistent stream to the viewer
    let mut stream = match conn.open_bi_stream().await {
        Ok(s) => s,
        Err(e) => {
            log::error!("[SIMPLE] Failed to open stream to {}: {}", peer_ip, e);
            return;
        }
    };
    log::info!("[SIMPLE] Opened persistent stream to viewer {}", peer_ip);

    // Take the sharer state
    let state_opt = SHARER_STATE.write().take();
    let Some(mut state) = state_opt else {
        log::error!("[SIMPLE] Sharer state not available");
        return;
    };

    // Send ScreenStart as the FIRST message on this stream
    let start_data = encode_start_message(state.encode_width, state.encode_height);
    if let Err(e) = stream.send_framed(&start_data).await {
        log::error!("[SIMPLE] Failed to send ScreenStart: {}", e);
        return;
    }
    log::info!("[SIMPLE] Sent ScreenStart ({}x{}) to {}", state.encode_width, state.encode_height, peer_ip);

    // Now stream frames on the SAME stream
    let frame_interval = Duration::from_micros(1_000_000 / SIMPLE_FPS as u64);
    let mut sequence: u32 = 0;
    let mut last_frame_time = std::time::Instant::now();

    log::info!("[SIMPLE] Starting frame streaming loop at {} fps", SIMPLE_FPS);

    loop {
        // Check stop signal
        if state.stop_rx.try_recv().is_ok() || !state.active.load(Ordering::SeqCst) {
            log::info!("[SIMPLE] Stop signal received, ending stream");
            break;
        }

        // Check for resolution change request from viewer (non-blocking)
        match stream.try_recv_framed().await {
            Ok(Some(req_data)) if req_data.len() >= 13 && req_data[0] == MSG_TYPE_RESOLUTION_REQUEST => {
                let new_target_w = u32::from_be_bytes([req_data[1], req_data[2], req_data[3], req_data[4]]);
                let new_target_h = u32::from_be_bytes([req_data[5], req_data[6], req_data[7], req_data[8]]);
                let bitrate = u32::from_be_bytes([req_data[9], req_data[10], req_data[11], req_data[12]]);
                log::info!("[SIMPLE] Resolution change requested: {}x{} @ {} bps", new_target_w, new_target_h, bitrate);

                // Reconfigure scaler
                let src_w = state.pre_scaler.src_width;
                let src_h = state.pre_scaler.src_height;
                state.pre_scaler = FrameScaler::new_with_target(src_w, src_h, new_target_w, new_target_h);
                let new_encode_w = state.pre_scaler.dst_width;
                let new_encode_h = state.pre_scaler.dst_height;

                // Recreate encoder with new dimensions
                match encoder::create_encoder() {
                    Ok(mut new_encoder) => {
                        let enc_config = EncoderConfig {
                            width: new_encode_w,
                            height: new_encode_h,
                            fps: SIMPLE_FPS,
                            bitrate,
                            max_bitrate: bitrate * 2,
                            keyframe_interval: SIMPLE_FPS,
                            preset: EncoderPreset::UltraFast,
                        };
                        if let Err(e) = new_encoder.init(enc_config) {
                            log::error!("[SIMPLE] Failed to reinit encoder: {}", e);
                        } else {
                            state.encoder = new_encoder;
                            state.encode_width = new_encode_w;
                            state.encode_height = new_encode_h;
                            log::info!("[SIMPLE] Encoder reconfigured: {}x{} @ {} bps", new_encode_w, new_encode_h, bitrate);

                            // Send new START message so viewer reinits decoder
                            let start_data = encode_start_message(new_encode_w, new_encode_h);
                            if let Err(e) = stream.send_framed(&start_data).await {
                                log::error!("[SIMPLE] Failed to send new ScreenStart: {}", e);
                                break;
                            }
                            log::info!("[SIMPLE] Sent new ScreenStart ({}x{}) after resolution change", new_encode_w, new_encode_h);
                            sequence = 0;
                        }
                    }
                    Err(e) => {
                        log::error!("[SIMPLE] Failed to create new encoder: {}", e);
                    }
                }
            }
            Ok(Some(_)) => {} // unknown message from viewer, ignore
            Ok(None) => {} // no message ready
            Err(e) => {
                log::debug!("[SIMPLE] Error reading from viewer: {}", e);
            }
        }

        // Frame rate limiting
        let elapsed = last_frame_time.elapsed();
        if elapsed < frame_interval {
            tokio::time::sleep(frame_interval - elapsed).await;
        }
        last_frame_time = std::time::Instant::now();

        // Capture + scale + encode in block_in_place to avoid blocking tokio worker
        let capture_result = tokio::task::block_in_place(|| {
            let t0 = std::time::Instant::now();

            let frame = match state.capture.capture_frame() {
                Ok(f) => f,
                Err(e) => {
                    return Err(format!("Capture: {}", e));
                }
            };
            let t_capture = t0.elapsed();

            // Downscale before encoding (e.g. 3456x2160 → 1280x720)
            let scaled_data = state.pre_scaler.scale(&frame.data);
            let t_scale = t0.elapsed();

            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            let encoded = match state.encoder.encode(&scaled_data, timestamp) {
                Ok(e) => e,
                Err(e) => {
                    return Err(format!("Encode: {}", e));
                }
            };
            let t_encode = t0.elapsed();

            if sequence < 10 || sequence % 50 == 0 {
                log::info!("[SIMPLE] Frame {} timing: capture={:.1}ms scale={:.1}ms encode={:.1}ms total={:.1}ms",
                    sequence,
                    t_capture.as_secs_f64() * 1000.0,
                    (t_scale - t_capture).as_secs_f64() * 1000.0,
                    (t_encode - t_scale).as_secs_f64() * 1000.0,
                    t_encode.as_secs_f64() * 1000.0,
                );
            }

            Ok((timestamp, encoded))
        });

        let (timestamp, encoded) = match capture_result {
            Ok(r) => r,
            Err(e) => {
                if sequence < 10 || sequence % 50 == 0 {
                    log::warn!("[SIMPLE] Frame {} error: {}", sequence, e);
                }
                continue;
            }
        };

        // Skip empty frames (encoder buffering, e.g. B-frame reordering)
        if encoded.data.is_empty() {
            sequence += 1;
            continue;
        }

        if sequence < 10 || sequence % 50 == 0 {
            log::info!("[SIMPLE] Frame {} encoded: {} bytes, type={:?}",
                sequence, encoded.data.len(), encoded.frame_type);
        }

        // Send frame on the same persistent stream
        let frame_data = encode_frame_message(timestamp, &encoded.data);
        if let Err(e) = stream.send_framed(&frame_data).await {
            log::info!("[SIMPLE] Viewer disconnected (send failed at frame {}): {}", sequence, e);
            break;
        }

        if sequence < 10 {
            log::info!("[SIMPLE] Frame {} sent ({} bytes on wire)", sequence, frame_data.len());
        }

        sequence += 1;
    }

    // Send stop message
    let stop_data = encode_stop_message();
    let _ = stream.send_framed(&stop_data).await;
    let _ = stream.finish().await;

    let _ = state.capture.stop();
    SIMPLE_SHARER_ACTIVE.store(false, Ordering::SeqCst);
    log::info!("[SIMPLE] Streaming ended after {} frames", sequence);
}

/// Stop simple sharing
pub fn stop_sharing() {
    log::info!("[SIMPLE] Stopping simple sharing");
    SIMPLE_SHARER_ACTIVE.store(false, Ordering::SeqCst);
    if let Some(tx) = SIMPLE_STOP_TX.write().take() {
        let _ = tx.try_send(());
    }
    // Clean up sharer state
    let _ = SHARER_STATE.write().take();
}

// ===== Receiver side =====

/// Handle an incoming stream that carries simple streaming data.
/// Called when we detect a simple stream message on an accepted bi-stream.
pub async fn handle_simple_stream(stream: &mut QuicStream, peer_ip: &str) {
    use crate::renderer::WindowEvent;

    log::info!("[SIMPLE] === Handling simple stream from {} ===", peer_ip);

    let mut decoder: Option<SoftwareDecoder> = None;
    let mut window_handle: Option<RenderWindowHandle> = None;
    let mut frame_count: u32 = 0;

    loop {
        // Poll window events (resolution requests, close) between frame receives
        if let Some(ref handle) = window_handle {
            while let Some(event) = handle.try_recv_event() {
                match event {
                    WindowEvent::ResolutionRequested(target_w, target_h, bitrate) => {
                        log::info!("[SIMPLE] Viewer requesting resolution {}x{} @ {} bps", target_w, target_h, bitrate);
                        let req = encode_resolution_request(target_w, target_h, bitrate);
                        if let Err(e) = stream.send_framed(&req).await {
                            log::error!("[SIMPLE] Failed to send resolution request: {}", e);
                        }
                    }
                    WindowEvent::CloseRequested => {
                        log::info!("[SIMPLE] Window close requested by user");
                        break;
                    }
                    _ => {}
                }
            }
            if !handle.is_open() {
                log::info!("[SIMPLE] Render window closed by user");
                break;
            }
        }

        // Receive next framed message with timeout to allow event polling
        let data = match tokio::time::timeout(
            Duration::from_millis(100),
            stream.recv_framed(),
        ).await {
            Ok(Ok(d)) => d,
            Ok(Err(e)) => {
                log::info!("[SIMPLE] Stream closed from {}: {}", peer_ip, e);
                break;
            }
            Err(_) => continue, // timeout, loop back to poll events
        };

        if data.is_empty() {
            log::warn!("[SIMPLE] Empty message received from {}", peer_ip);
            continue;
        }

        let msg_type = data[0];

        match msg_type {
            MSG_TYPE_START => {
                if data.len() < 9 {
                    log::error!("[SIMPLE] ScreenStart message too short: {} bytes", data.len());
                    continue;
                }

                let width = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
                let height = u32::from_be_bytes([data[5], data[6], data[7], data[8]]);

                log::info!("[SIMPLE] Received ScreenStart: {}x{} from {}", width, height, peer_ip);

                // Init decoder (always reinit on START - handles resolution changes)
                let mut dec = match SoftwareDecoder::new() {
                    Ok(d) => d,
                    Err(e) => {
                        log::error!("[SIMPLE] Failed to create decoder: {}", e);
                        break;
                    }
                };

                let config = DecoderConfig {
                    width,
                    height,
                    output_format: OutputFormat::YUV420,
                };

                if let Err(e) = dec.init(config) {
                    log::error!("[SIMPLE] Failed to init decoder: {}", e);
                    break;
                }
                log::info!("[SIMPLE] Decoder (re)initialized for {}x{} (output=YUV420)", width, height);

                // Only create window if not already open (resolution changes keep existing window)
                if window_handle.is_none() {
                    let title = format!("[Simple] {} screen", peer_ip);
                    match RenderWindow::create(&title, width, height) {
                        Ok(handle) => {
                            log::info!("[SIMPLE] Render window created: {}x{}", width, height);
                            window_handle = Some(handle);
                        }
                        Err(e) => {
                            log::error!("[SIMPLE] Failed to create render window: {}", e);
                            break;
                        }
                    }
                }

                decoder = Some(dec);
                frame_count = 0;
            }

            MSG_TYPE_FRAME => {
                // Collect this frame + drain any pending frames from the stream
                let mut pending_frames = vec![data];
                loop {
                    match tokio::time::timeout(Duration::ZERO, stream.recv_framed()).await {
                        Ok(Ok(next)) if !next.is_empty() && next[0] == MSG_TYPE_FRAME => {
                            pending_frames.push(next);
                        }
                        Ok(Ok(next)) if !next.is_empty() && next[0] == MSG_TYPE_STOP => {
                            log::info!("[SIMPLE] Received Stop message from {}", peer_ip);
                            // Process remaining frames then exit
                            pending_frames.push(next);
                            break;
                        }
                        _ => break,
                    }
                }

                let skipped = if pending_frames.len() > 1 { pending_frames.len() - 1 } else { 0 };
                if skipped > 0 {
                    log::info!("[SIMPLE] Skipped {} stale frames, processing latest", skipped);
                }

                // Check window is still open
                if let Some(ref handle) = window_handle {
                    if !handle.is_open() {
                        log::info!("[SIMPLE] Render window closed by user");
                        break;
                    }
                } else {
                    log::warn!("[SIMPLE] Frame received but no window (missing ScreenStart?)");
                    continue;
                }

                let Some(ref mut dec) = decoder else {
                    log::warn!("[SIMPLE] Frame received but no decoder (missing ScreenStart?)");
                    continue;
                };

                // Decode ALL frames (H.264 P-frames need sequential decode), render only the last
                for (i, fdata) in pending_frames.iter().enumerate() {
                    if fdata[0] == MSG_TYPE_STOP {
                        break;
                    }
                    if fdata.len() < 13 {
                        continue;
                    }

                    let timestamp = u64::from_be_bytes([
                        fdata[1], fdata[2], fdata[3], fdata[4],
                        fdata[5], fdata[6], fdata[7], fdata[8],
                    ]);
                    let frame_len = u32::from_be_bytes([fdata[9], fdata[10], fdata[11], fdata[12]]) as usize;

                    if fdata.len() < 13 + frame_len {
                        continue;
                    }

                    let encoded_data = &fdata[13..13 + frame_len];
                    let is_last = i == pending_frames.len() - 1
                        || (i + 1 < pending_frames.len() && pending_frames[i + 1][0] == MSG_TYPE_STOP);

                    match dec.decode(encoded_data, timestamp) {
                        Ok(Some(decoded)) => {
                            frame_count += 1;
                            // Only render the latest frame
                            if is_last {
                                if let Some(cpu_data) = decoded.cpu_data() {
                                    let strides = decoded.strides().unwrap_or([
                                        decoded.width as usize,
                                        decoded.width as usize / 2,
                                        decoded.width as usize / 2,
                                    ]);
                                    let render_frame = RenderFrame::from_yuv420(
                                        decoded.width,
                                        decoded.height,
                                        cpu_data.to_vec(),
                                        strides,
                                    );
                                    if let Some(ref handle) = window_handle {
                                        if let Err(e) = handle.render_frame(render_frame) {
                                            if frame_count % 100 == 0 {
                                                log::warn!("[SIMPLE] Render error: {}", e);
                                            }
                                        }
                                    }
                                }
                                if frame_count == 1 || frame_count % 50 == 0 {
                                    log::info!("[SIMPLE] Frame {} decoded and rendered", frame_count);
                                }
                            }
                        }
                        Ok(None) => {
                            if frame_count == 0 {
                                log::debug!("[SIMPLE] Decoder buffering (no output yet)");
                            }
                        }
                        Err(e) => {
                            if frame_count % 100 == 0 {
                                log::warn!("[SIMPLE] Decode error at frame {}: {}", frame_count, e);
                            }
                        }
                    }
                }

                // If we drained a STOP message, exit
                if pending_frames.last().map(|f| f[0]) == Some(MSG_TYPE_STOP) {
                    break;
                }
            }

            MSG_TYPE_STOP => {
                log::info!("[SIMPLE] Received Stop message from {}", peer_ip);
                break;
            }

            _ => {
                log::warn!("[SIMPLE] Unknown message type: 0x{:02x}", msg_type);
            }
        }
    }

    // Cleanup: stop receiving so sharer's send_framed fails immediately
    stream.stop_receiving();
    log::info!("[SIMPLE] Stream stopped, notifying sharer");

    if let Some(ref handle) = window_handle {
        handle.close();
    }
    log::info!("[SIMPLE] Simple stream handler ended, {} frames rendered", frame_count);
}

// ===== Message encoding =====

fn encode_start_message(width: u32, height: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(9);
    data.push(MSG_TYPE_START);
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data
}

fn encode_frame_message(timestamp: u64, frame_data: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(13 + frame_data.len());
    data.push(MSG_TYPE_FRAME);
    data.extend_from_slice(&timestamp.to_be_bytes());
    data.extend_from_slice(&(frame_data.len() as u32).to_be_bytes());
    data.extend_from_slice(frame_data);
    data
}

fn encode_stop_message() -> Vec<u8> {
    vec![MSG_TYPE_STOP]
}

fn encode_resolution_request(target_width: u32, target_height: u32, bitrate: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(13);
    data.push(MSG_TYPE_RESOLUTION_REQUEST);
    data.extend_from_slice(&target_width.to_be_bytes());
    data.extend_from_slice(&target_height.to_be_bytes());
    data.extend_from_slice(&bitrate.to_be_bytes());
    data
}

/// Public wrapper for encoding resolution request (used by lib.rs)
pub fn encode_resolution_request_msg(target_width: u32, target_height: u32, bitrate: u32) -> Vec<u8> {
    encode_resolution_request(target_width, target_height, bitrate)
}

/// Check if a framed message is a simple streaming message
/// (first byte after recv_framed is one of our message types)
pub fn is_simple_message(data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }
    matches!(data[0], MSG_TYPE_START | MSG_TYPE_FRAME | MSG_TYPE_STOP | MSG_TYPE_RESOLUTION_REQUEST)
}

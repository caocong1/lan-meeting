// LAN Meeting - High-performance screen sharing tool
// Main library entry point

pub mod capture;
pub mod chat;
pub mod commands;
pub mod decoder;
pub mod encoder;
pub mod input;
pub mod network;
pub mod renderer;
pub mod simple_streaming;
pub mod streaming;
pub mod transfer;

use network::quic::QuicEndpoint;
use once_cell::sync::OnceCell;
use std::sync::Arc;
use tauri::Emitter;

/// Global QUIC endpoint
pub static QUIC_ENDPOINT: OnceCell<Arc<QuicEndpoint>> = OnceCell::new();

/// Global Tauri app handle for emitting events
pub static APP_HANDLE: OnceCell<tauri::AppHandle> = OnceCell::new();

/// Get the global QUIC endpoint
pub fn get_quic_endpoint() -> Option<&'static Arc<QuicEndpoint>> {
    QUIC_ENDPOINT.get()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Install aws-lc-rs as the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    tauri::Builder::default()
        .setup(|app| {
            // Initialize logging
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Debug)
                        .build(),
                )?;
            }

            // Initialize dialog plugin
            app.handle().plugin(tauri_plugin_dialog::init())?;

            // Store app handle globally for emitting events
            let _ = APP_HANDLE.set(app.handle().clone());

            // Note: QUIC and mDNS are now started via start_service command
            log::info!("LAN Meeting started (service not yet enabled)");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_displays,
            commands::start_capture,
            commands::stop_capture,
            commands::check_screen_permission,
            commands::request_screen_permission,
            commands::get_devices,
            commands::add_manual_device,
            commands::connect_to_device,
            commands::disconnect,
            commands::get_self_info,
            commands::send_chat_message,
            commands::get_chat_messages,
            commands::check_input_permission,
            commands::request_input_permission,
            commands::offer_file,
            commands::accept_file_transfer,
            commands::reject_file_transfer,
            commands::cancel_file_transfer,
            commands::get_file_transfers,
            commands::get_active_file_transfers,
            commands::get_file_transfer,
            commands::get_download_directory,
            // Service commands
            commands::start_service,
            commands::stop_service,
            commands::is_service_running,
            // Settings commands
            commands::get_settings,
            commands::save_settings,
            // Sharing commands
            commands::broadcast_sharing_status,
            commands::open_viewer_window,
            commands::request_control,
            commands::request_screen_stream,
            commands::stop_viewing_stream,
            // Simple streaming commands
            commands::simple_start_sharing,
            commands::simple_request_stream,
            commands::simple_stop_sharing,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Handle incoming QUIC connection
pub async fn handle_incoming_connection(conn: Arc<network::quic::QuicConnection>) {
    use network::protocol::MessageCodec;

    log::info!("Handling connection from {}", conn.remote_addr());

    // Accept bidirectional streams for control messages
    loop {
        match conn.accept_bi_stream().await {
            Ok(mut stream) => {
                let conn_clone = conn.clone();
                tokio::spawn(async move {
                    // Read first message to detect if this is a simple stream
                    let first_data = match stream.recv_framed().await {
                        Ok(d) => d,
                        Err(e) => {
                            log::debug!("Stream closed on first read: {}", e);
                            return;
                        }
                    };

                    // Check if this is a simple streaming message
                    if simple_streaming::is_simple_message(&first_data) {
                        let peer_ip = conn_clone.remote_addr().ip().to_string();
                        log::info!("[SIMPLE] Detected simple stream from {}", peer_ip);

                        // Handle the first message manually, then pass to handler
                        // We need to re-process the first message since we already consumed it
                        // Create a wrapper that first yields the already-read data
                        handle_simple_stream_with_first(&first_data, &mut stream, &peer_ip).await;
                        return;
                    }

                    // Normal protocol message path
                    let mut codec = MessageCodec::new();
                    codec.feed(&first_data);

                    // Process messages from the first read
                    while let Ok(Some(msg)) = codec.decode() {
                        if let Err(e) = handle_message(&msg, &mut stream, &conn_clone).await {
                            log::error!("Failed to handle message: {}", e);
                        }
                    }

                    // Handle subsequent stream messages
                    loop {
                        match stream.recv_framed().await {
                            Ok(data) => {
                                codec.feed(&data);

                                // Process all complete messages
                                while let Ok(Some(msg)) = codec.decode() {
                                    if let Err(e) = handle_message(&msg, &mut stream, &conn_clone).await {
                                        log::error!("Failed to handle message: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                log::debug!("Stream closed: {}", e);
                                break;
                            }
                        }
                    }
                });
            }
            Err(e) => {
                log::debug!("Connection closed: {}", e);
                break;
            }
        }
    }

    // Connection ended - clean up the device associated with this peer
    let peer_ip = conn.remote_addr().ip().to_string();
    log::info!("Peer disconnected: {}, cleaning up device", peer_ip);
    let devices = network::discovery::get_devices();
    for device in &devices {
        if device.ip == peer_ip {
            log::info!("Removing disconnected device '{}' (ip={})", device.name, device.ip);
            network::discovery::remove_device(&device.id);
            if let Some(app) = APP_HANDLE.get() {
                let _ = app.emit("device-removed", &device.id);
            }
        }
    }
    // Also clean up the QUIC connection entry
    network::quic::remove_connection_by_ip(&peer_ip);
}

/// Handle a protocol message
async fn handle_message(
    msg: &network::protocol::Message,
    stream: &mut network::quic::QuicStream,
    _conn: &Arc<network::quic::QuicConnection>,
) -> Result<(), network::NetworkError> {
    use network::protocol::{self, Message};

    match msg {
        Message::Handshake {
            device_id,
            name,
            version,
            capabilities,
        } => {
            log::info!(
                "Received handshake from {} ({}) v{}, capabilities: {:?}",
                name,
                device_id,
                version,
                capabilities
            );

            // Add the remote device to our device list
            let remote_addr = _conn.remote_addr();
            let remote_device = network::discovery::DiscoveredDevice {
                id: device_id.clone(),
                name: name.clone(),
                ip: remote_addr.ip().to_string(),
                port: network::quic::DEFAULT_PORT, // Use default port, not ephemeral source port
                status: network::discovery::DeviceStatus::Online,
                last_seen: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                is_sharing: false,
            };
            network::discovery::add_device(remote_device.clone());
            log::info!("Added {} ({}) to device list", name, remote_addr.ip());

            // Emit event to frontend to notify about the new connection
            if let Some(handle) = APP_HANDLE.get() {
                #[derive(serde::Serialize, Clone)]
                struct ConnectionEvent {
                    device_id: String,
                    device_name: String,
                    ip: String,
                }
                let _ = handle.emit("connection-received", ConnectionEvent {
                    device_id: device_id.clone(),
                    device_name: name.clone(),
                    ip: remote_addr.ip().to_string(),
                });

                // Also emit device-discovered so the device list updates
                let _ = handle.emit("device-discovered", &remote_device);
            }

            // Send handshake acknowledgment
            let our_id = network::discovery::get_our_device_id();
            let our_name = hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "Unknown".to_string());

            let ack = protocol::create_handshake_ack(&our_id, &our_name, true, None);
            let encoded = protocol::encode(&ack)?;
            stream.send_framed(&encoded).await?;

            log::info!("Handshake accepted from {}, sent acknowledgment", name);
        }

        Message::HandshakeAck {
            device_id,
            name,
            accepted,
            reason,
            ..
        } => {
            if *accepted {
                log::info!("Handshake accepted by {} ({})", name, device_id);
            } else {
                log::warn!(
                    "Handshake rejected by {} ({}): {:?}",
                    name,
                    device_id,
                    reason
                );
            }
        }

        Message::Heartbeat { timestamp } => {
            // Respond with heartbeat ack
            let ack = protocol::create_heartbeat_ack(*timestamp);
            let encoded = protocol::encode(&ack)?;
            stream.send_framed(&encoded).await?;
        }

        Message::HeartbeatAck { latency_ms, .. } => {
            log::debug!("Heartbeat latency: {}ms", latency_ms);
        }

        Message::Disconnect { reason } => {
            log::info!("Peer disconnected: {}", reason);
        }

        Message::ChatMessage {
            from,
            content,
            timestamp,
        } => {
            log::info!("[{}] {}: {}", timestamp, from, content);
            // Store the message
            chat::receive_message(from, from, content, *timestamp);

            // Emit event to frontend
            if let Some(handle) = APP_HANDLE.get() {
                let msg = chat::get_chat_manager()
                    .get_messages()
                    .into_iter()
                    .last();
                if let Some(msg) = msg {
                    let _ = handle.emit("chat-message", msg);
                }
            }
        }

        // Screen sharing messages
        Message::ScreenOffer { displays } => {
            let remote_ip = _conn.remote_addr().ip().to_string();
            let is_sharing = !displays.is_empty();

            log::info!(
                "Received screen offer from {}: {} displays (sharing: {})",
                remote_ip,
                displays.len(),
                is_sharing
            );

            // Update device sharing status
            if let Some(device_id) = network::discovery::update_device_sharing_by_ip(&remote_ip, is_sharing) {
                // Emit event to frontend
                if let Some(handle) = APP_HANDLE.get() {
                    #[derive(serde::Serialize, Clone)]
                    struct SharingStatusEvent {
                        device_id: String,
                        is_sharing: bool,
                    }
                    let _ = handle.emit("sharing-status-changed", SharingStatusEvent {
                        device_id,
                        is_sharing,
                    });
                }
            }
        }

        Message::ScreenRequest { display_id, preferred_fps, preferred_quality } => {
            let remote_ip = _conn.remote_addr().ip().to_string();
            log::info!(
                "Received screen request from {}: display={}, fps={}, quality={}",
                remote_ip,
                display_id,
                preferred_fps,
                preferred_quality
            );

            // Check if we are sharing
            let manager = streaming::get_streaming_manager();
            let is_streaming = manager.read().as_ref().map(|m| m.is_streaming()).unwrap_or(false);

            if is_streaming {
                // Send ScreenStart response via a NEW stream (not the request stream)
                // The request stream is already finished/dropped by the sender,
                // so we must use send_to_peer to open a fresh stream
                let (width, height) = manager.read().as_ref().map(|m| m.dimensions()).unwrap_or((1920, 1080));
                let fps = manager.read().as_ref().map(|m| m.config().fps).unwrap_or(30);

                let start_msg = network::protocol::Message::ScreenStart {
                    width,
                    height,
                    fps: fps as u8,
                    codec: "h264".to_string(),
                };

                if let Ok(encoded) = network::protocol::encode(&start_msg) {
                    if let Err(e) = network::quic::send_to_peer(&remote_ip, &encoded).await {
                        log::error!("Failed to send ScreenStart to {}: {}", remote_ip, e);
                    } else {
                        log::info!("Sent ScreenStart to {} ({}x{} @ {}fps)", remote_ip, width, height, fps);
                    }
                }
            } else {
                log::warn!("Received ScreenRequest from {} but we are not streaming", remote_ip);
            }
        }

        Message::ScreenStart { width, height, fps, codec } => {
            let remote_ip = _conn.remote_addr().ip().to_string();
            log::info!(
                "Received screen start from {}: {}x{} @ {} fps, codec={}",
                remote_ip,
                width,
                height,
                fps,
                codec
            );

            // Initialize viewer session and create native render window
            let sessions = streaming::get_viewer_sessions();
            if let Some(session) = sessions.write().get_mut(&remote_ip) {
                match session.handle_screen_start(*width, *height, *fps, codec) {
                    Ok(_) => {
                        log::info!("Native viewer window created for {}", remote_ip);
                    }
                    Err(e) => {
                        log::error!("Failed to start viewer session: {}", e);
                    }
                }
            } else {
                log::warn!("No viewer session found for {}", remote_ip);
            }
        }

        Message::ScreenFrame { timestamp, frame_type: _, sequence, data } => {
            let remote_ip = _conn.remote_addr().ip().to_string();

            // Decode and render frame in native window (no Tauri event overhead)
            let sessions = streaming::get_viewer_sessions();
            let mut sessions_guard = sessions.write();

            if let Some(session) = sessions_guard.get_mut(&remote_ip) {
                if session.is_active() {
                    // Decode and render directly to native wgpu window
                    if let Err(e) = session.handle_screen_frame(*timestamp, data) {
                        // Only log occasional errors to avoid spam
                        if *sequence % 100 == 0 {
                            log::warn!("Frame {} decode error: {}", sequence, e);
                        }
                    }
                }
            }

            // Drop lock before any other operations
            drop(sessions_guard);
        }

        Message::ScreenStop => {
            let remote_ip = _conn.remote_addr().ip().to_string();
            log::info!("Received screen stop from {}", remote_ip);

            // Stop viewer session (closes native window)
            let sessions = streaming::get_viewer_sessions();
            if let Some(session) = sessions.write().get_mut(&remote_ip) {
                session.handle_screen_stop();
            }
        }

        // Simple streaming request (minimal pipeline)
        Message::SimpleScreenRequest { display_id } => {
            let remote_ip = _conn.remote_addr().ip().to_string();
            log::info!("[SIMPLE] Received SimpleScreenRequest from {} (display={})", remote_ip, display_id);

            // Handle in a background task - this will open a persistent stream and stream frames
            let peer_ip = remote_ip.clone();
            tokio::spawn(async move {
                simple_streaming::handle_viewer_request(&peer_ip).await;
            });
        }

        // Remote control messages will be handled in Phase 6
        Message::ControlRequest { .. }
        | Message::ControlGrant { .. }
        | Message::ControlRevoke
        | Message::InputEvent { .. } => {
            log::debug!("Remote control message received (not yet implemented)");
        }

        // File transfer messages
        Message::FileOffer {
            file_id,
            name,
            size,
            checksum,
        } => {
            log::info!(
                "Received file offer: {} ({} bytes, checksum: {})",
                name,
                size,
                checksum
            );

            // Create FileInfo and register incoming transfer
            let info = transfer::FileInfo {
                id: file_id.clone(),
                name: name.clone(),
                size: *size,
                checksum: checksum.clone(),
                mime_type: None,
            };

            // Get peer ID from connection
            let peer_id = _conn.remote_addr().to_string();
            let transfer_record = transfer::get_transfer_manager().receive_offer(info, &peer_id);

            // Emit event to frontend to show file offer UI
            if let Some(handle) = APP_HANDLE.get() {
                let _ = handle.emit("file-offer", &transfer_record);
            }
            log::info!("File offer registered, waiting for user acceptance");
        }

        Message::FileAccept { file_id } => {
            log::info!("File transfer accepted: {}", file_id);

            // Start sending file chunks
            if let Some(transfer) = transfer::get_transfer_manager().get_transfer(file_id) {
                if transfer.direction == transfer::TransferDirection::Outgoing {
                    // Update transfer status
                    let manager = transfer::get_transfer_manager();
                    if let Some(mut t) = manager.get_transfer(file_id) {
                        t.start();
                    }

                    // TODO: Start sending chunks in a separate task
                    log::info!("Starting to send file chunks for {}", file_id);
                }
            }
        }

        Message::FileReject { file_id } => {
            log::info!("File transfer rejected: {}", file_id);
            let _ = transfer::get_transfer_manager().cancel_transfer(file_id);
        }

        Message::FileChunk {
            file_id,
            offset,
            data,
        } => {
            log::debug!(
                "Received file chunk: {} offset={} size={}",
                file_id,
                offset,
                data.len()
            );

            // Write chunk to file
            match transfer::get_transfer_manager().write_chunk(file_id, *offset, data) {
                Ok(bytes) => {
                    log::debug!("File {} progress: {} bytes", file_id, bytes);

                    // Emit progress event to frontend
                    if let Some(handle) = APP_HANDLE.get() {
                        if let Some(transfer) = transfer::get_transfer_manager().get_transfer(file_id) {
                            #[derive(serde::Serialize, Clone)]
                            struct ProgressEvent {
                                file_id: String,
                                progress: f32,
                                bytes: u64,
                            }
                            let _ = handle.emit("file-progress", ProgressEvent {
                                file_id: file_id.clone(),
                                progress: transfer.progress,
                                bytes,
                            });
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to write chunk: {}", e);
                }
            }
        }

        Message::FileComplete { file_id } => {
            log::info!("File transfer complete: {}", file_id);

            // Finalize the transfer
            match transfer::get_transfer_manager().complete_transfer(file_id) {
                Ok(_) => {
                    log::info!("File {} verified and saved", file_id);

                    // Emit completion event to frontend
                    if let Some(handle) = APP_HANDLE.get() {
                        #[derive(serde::Serialize, Clone)]
                        struct CompleteEvent {
                            file_id: String,
                            success: bool,
                        }
                        let _ = handle.emit("file-complete", CompleteEvent {
                            file_id: file_id.clone(),
                            success: true,
                        });
                    }
                }
                Err(e) => {
                    log::error!("Failed to complete transfer: {}", e);

                    // Emit failure event to frontend
                    if let Some(handle) = APP_HANDLE.get() {
                        #[derive(serde::Serialize, Clone)]
                        struct CompleteEvent {
                            file_id: String,
                            success: bool,
                        }
                        let _ = handle.emit("file-complete", CompleteEvent {
                            file_id: file_id.clone(),
                            success: false,
                        });
                    }
                }
            }
        }

        Message::FileCancel { file_id } => {
            log::info!("File transfer cancelled: {}", file_id);
            let _ = transfer::get_transfer_manager().cancel_transfer(file_id);

            // Emit cancel event to frontend
            if let Some(handle) = APP_HANDLE.get() {
                #[derive(serde::Serialize, Clone)]
                struct CancelEvent {
                    file_id: String,
                }
                let _ = handle.emit("file-cancelled", CancelEvent {
                    file_id: file_id.clone(),
                });
            }
        }
    }

    Ok(())
}

/// Handle a simple stream where the first message was already consumed
async fn handle_simple_stream_with_first(
    first_data: &[u8],
    stream: &mut network::quic::QuicStream,
    peer_ip: &str,
) {
    log::info!("[SIMPLE] === Handling simple stream from {} ===", peer_ip);

    let mut decoder: Option<crate::decoder::software::SoftwareDecoder> = None;
    let mut window_handle: Option<crate::renderer::RenderWindowHandle> = None;
    let mut frame_count: u32 = 0;

    // Process the first message
    process_simple_message(first_data, peer_ip, &mut decoder, &mut window_handle, &mut frame_count);

    // Send initial resolution request based on saved settings (if window was just created)
    if window_handle.is_some() {
        let (res_idx, br_idx) = crate::commands::get_default_streaming_indices();
        if res_idx != 0 || br_idx != 0 {
            let res_opts = &crate::simple_streaming::RESOLUTION_OPTIONS;
            let br_opts = &crate::simple_streaming::BITRATE_OPTIONS;
            if let (Some(res), Some(br)) = (
                res_opts.get(res_idx.min(res_opts.len() - 1)),
                br_opts.get(br_idx.min(br_opts.len() - 1)),
            ) {
                log::info!("[SIMPLE] Sending initial resolution request: {} + {}", res.label, br.label);
                let req = crate::simple_streaming::encode_resolution_request_msg(
                    res.target_width, res.target_height, br.bitrate,
                );
                if let Err(e) = stream.send_framed(&req).await {
                    log::error!("[SIMPLE] Failed to send initial resolution request: {}", e);
                }
            }
        }
    }

    // Continue reading from stream
    log::info!("[SIMPLE] Entering frame receive loop from {}", peer_ip);
    loop {
        // Poll window events (resolution requests)
        if let Some(ref handle) = window_handle {
            while let Some(event) = handle.try_recv_event() {
                if let crate::renderer::WindowEvent::ResolutionRequested(target_w, target_h, bitrate) = event {
                    log::info!("[SIMPLE] Viewer requesting resolution {}x{} @ {} bps", target_w, target_h, bitrate);
                    let req = crate::simple_streaming::encode_resolution_request_msg(target_w, target_h, bitrate);
                    if let Err(e) = stream.send_framed(&req).await {
                        log::error!("[SIMPLE] Failed to send resolution request: {}", e);
                    }
                }
            }
            if !handle.is_open() {
                log::info!("[SIMPLE] Render window closed by user");
                break;
            }
        }

        let data = match tokio::time::timeout(
            std::time::Duration::from_millis(100),
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
            log::warn!("[SIMPLE] Empty data received from {}", peer_ip);
            continue;
        }

        let msg_type = data[0];
        if frame_count < 10 || frame_count % 50 == 0 {
            log::info!("[SIMPLE] Received msg type=0x{:02x}, {} bytes from {} (frame_count={})",
                msg_type, data.len(), peer_ip, frame_count);
        }

        if msg_type == 0x03 {
            // MSG_TYPE_STOP
            log::info!("[SIMPLE] Received Stop message from {}", peer_ip);
            break;
        }

        process_simple_message(&data, peer_ip, &mut decoder, &mut window_handle, &mut frame_count);
    }

    // Cleanup
    if let Some(handle) = window_handle.as_ref() {
        handle.close();
    }
    log::info!("[SIMPLE] Simple stream handler ended, {} frames rendered", frame_count);
}

/// Process a single simple streaming message
fn process_simple_message(
    data: &[u8],
    peer_ip: &str,
    decoder: &mut Option<crate::decoder::software::SoftwareDecoder>,
    window_handle: &mut Option<crate::renderer::RenderWindowHandle>,
    frame_count: &mut u32,
) {
    use crate::decoder::software::SoftwareDecoder;
    use crate::decoder::{DecoderConfig, OutputFormat, VideoDecoder};
    use crate::renderer::{RenderFrame, RenderWindow};

    if data.is_empty() {
        return;
    }

    let msg_type = data[0];

    match msg_type {
        0x01 => {
            // MSG_TYPE_START
            if data.len() < 9 {
                log::error!("[SIMPLE] ScreenStart message too short: {} bytes", data.len());
                return;
            }

            let width = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
            let height = u32::from_be_bytes([data[5], data[6], data[7], data[8]]);

            log::info!("[SIMPLE] Received ScreenStart: {}x{} from {}", width, height, peer_ip);

            // Init decoder
            let mut dec = match SoftwareDecoder::new() {
                Ok(d) => d,
                Err(e) => {
                    log::error!("[SIMPLE] Failed to create decoder: {}", e);
                    return;
                }
            };

            let config = DecoderConfig {
                width,
                height,
                output_format: OutputFormat::BGRA,
            };

            if let Err(e) = dec.init(config) {
                log::error!("[SIMPLE] Failed to init decoder: {}", e);
                return;
            }
            log::info!("[SIMPLE] Decoder (re)initialized for {}x{}", width, height);

            // Only create window if not already open (resolution changes keep existing window)
            if window_handle.is_none() {
                let title = format!("[Simple] {} screen", peer_ip);
                match RenderWindow::create(&title, width, height) {
                    Ok(handle) => {
                        log::info!("[SIMPLE] Render window created: {}x{}", width, height);
                        *window_handle = Some(handle);
                    }
                    Err(e) => {
                        log::error!("[SIMPLE] Failed to create render window: {}", e);
                        return;
                    }
                }
            }

            *decoder = Some(dec);
            *frame_count = 0;
        }

        0x02 => {
            // MSG_TYPE_FRAME
            if data.len() < 13 {
                log::warn!("[SIMPLE] Frame message too short: {} bytes", data.len());
                return;
            }

            let timestamp = u64::from_be_bytes([
                data[1], data[2], data[3], data[4],
                data[5], data[6], data[7], data[8],
            ]);
            let frame_len = u32::from_be_bytes([data[9], data[10], data[11], data[12]]) as usize;

            if data.len() < 13 + frame_len {
                log::warn!("[SIMPLE] Frame data truncated: expected {} bytes, got {}",
                    13 + frame_len, data.len());
                return;
            }

            let frame_data = &data[13..13 + frame_len];

            // Check window is still open
            match window_handle.as_ref() {
                Some(handle) => {
                    if !handle.is_open() {
                        log::info!("[SIMPLE] Render window closed by user");
                        return;
                    }
                }
                None => {
                    if *frame_count == 0 {
                        log::warn!("[SIMPLE] Frame received but no window (missing ScreenStart?)");
                    }
                    return;
                }
            }

            // Decode
            let Some(dec) = decoder.as_mut() else {
                if *frame_count == 0 {
                    log::warn!("[SIMPLE] Frame received but no decoder");
                }
                return;
            };

            match dec.decode(frame_data, timestamp) {
                Ok(Some(decoded)) => {
                    if let Some(cpu_data) = decoded.cpu_data() {
                        let render_frame = RenderFrame::from_bgra(
                            decoded.width,
                            decoded.height,
                            cpu_data.to_vec(),
                        );

                        if let Some(handle) = window_handle.as_ref() {
                            if let Err(e) = handle.render_frame(render_frame) {
                                if *frame_count % 100 == 0 {
                                    log::warn!("[SIMPLE] Render error: {}", e);
                                }
                            }
                        }
                    }

                    *frame_count += 1;
                    if *frame_count == 1 || *frame_count % 50 == 0 {
                        log::info!("[SIMPLE] Frame {} decoded and rendered", frame_count);
                    }
                }
                Ok(None) => {
                    if *frame_count == 0 {
                        log::debug!("[SIMPLE] Decoder buffering (no output yet)");
                    }
                }
                Err(e) => {
                    if *frame_count % 100 == 0 {
                        log::warn!("[SIMPLE] Decode error at frame {}: {}", frame_count, e);
                    }
                }
            }
        }

        _ => {
            log::warn!("[SIMPLE] Unknown message type: 0x{:02x}", msg_type);
        }
    }
}

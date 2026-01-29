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
                    let mut codec = MessageCodec::new();

                    // Handle stream messages
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
                // Send ScreenStart response
                let (width, height) = manager.read().as_ref().map(|m| m.dimensions()).unwrap_or((1920, 1080));
                let fps = manager.read().as_ref().map(|m| m.config().fps).unwrap_or(30);

                let start_msg = network::protocol::Message::ScreenStart {
                    width,
                    height,
                    fps: fps as u8,
                    codec: "h264".to_string(),
                };

                if let Ok(encoded) = network::protocol::encode(&start_msg) {
                    let _ = stream.send_framed(&encoded).await;
                }
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

            // Initialize viewer session
            let sessions = streaming::get_viewer_sessions();
            if let Some(session) = sessions.write().get_mut(&remote_ip) {
                if let Err(e) = session.handle_screen_start(*width, *height, *fps, codec) {
                    log::error!("Failed to start viewer session: {}", e);
                }
            }

            // Emit event to frontend
            if let Some(handle) = APP_HANDLE.get() {
                #[derive(serde::Serialize, Clone)]
                struct ScreenStartEvent {
                    peer_ip: String,
                    width: u32,
                    height: u32,
                    fps: u8,
                    codec: String,
                }
                let _ = handle.emit("screen-start", ScreenStartEvent {
                    peer_ip: remote_ip,
                    width: *width,
                    height: *height,
                    fps: *fps,
                    codec: codec.clone(),
                });
            }
        }

        Message::ScreenFrame { timestamp, frame_type, sequence, data } => {
            let remote_ip = _conn.remote_addr().ip().to_string();

            // Handle frame in viewer session (without holding lock across await)
            let sessions = streaming::get_viewer_sessions();
            let session_active = {
                let sessions_read = sessions.read();
                sessions_read.get(&remote_ip).map(|s| s.is_active()).unwrap_or(false)
            };

            if session_active {
                // Note: For now we skip the async decode since we're sending raw H264 data
                // A proper implementation would decode in a separate task
            }

            // Emit frame event to frontend (with base64 encoded data for simplicity)
            // In production, we'd use a more efficient method like shared memory
            if let Some(handle) = APP_HANDLE.get() {
                #[derive(serde::Serialize, Clone)]
                struct ScreenFrameEvent {
                    peer_ip: String,
                    timestamp: u64,
                    frame_type: String,
                    sequence: u32,
                    data: String, // Base64 encoded
                }

                let frame_type_str = match frame_type {
                    network::protocol::FrameType::KeyFrame => "keyframe",
                    network::protocol::FrameType::DeltaFrame => "delta",
                };

                if session_active {
                    // For now, emit the raw H264 data encoded as base64
                    // The frontend will need to decode this
                    use base64::{Engine, engine::general_purpose::STANDARD};
                    let _ = handle.emit("screen-frame", ScreenFrameEvent {
                        peer_ip: remote_ip,
                        timestamp: *timestamp,
                        frame_type: frame_type_str.to_string(),
                        sequence: *sequence,
                        data: STANDARD.encode(data),
                    });
                }
            }
        }

        Message::ScreenStop => {
            let remote_ip = _conn.remote_addr().ip().to_string();
            log::info!("Received screen stop from {}", remote_ip);

            // Stop viewer session
            let sessions = streaming::get_viewer_sessions();
            if let Some(session) = sessions.write().get_mut(&remote_ip) {
                session.handle_screen_stop();
            }

            // Emit event to frontend
            if let Some(handle) = APP_HANDLE.get() {
                #[derive(serde::Serialize, Clone)]
                struct ScreenStopEvent {
                    peer_ip: String,
                }
                let _ = handle.emit("screen-stop", ScreenStopEvent {
                    peer_ip: remote_ip,
                });
            }
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

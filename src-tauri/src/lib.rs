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
pub mod transfer;

use network::quic::{QuicConfig, QuicEndpoint};
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

            // Initialize network discovery
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = network::discovery::start_discovery(handle).await {
                    log::error!("Failed to start mDNS discovery: {}", e);
                }
            });

            // Initialize QUIC endpoint
            tauri::async_runtime::spawn(async move {
                match QuicEndpoint::new(QuicConfig::default()).await {
                    Ok(endpoint) => {
                        let endpoint = Arc::new(endpoint);
                        log::info!("QUIC endpoint initialized on {}", endpoint.local_addr());

                        // Store globally
                        let _ = QUIC_ENDPOINT.set(endpoint.clone());

                        // Start accepting connections
                        endpoint.start_server(|conn| {
                            log::info!("Incoming connection from {}", conn.remote_addr());
                            // Handle incoming connection in a separate task
                            tokio::spawn(async move {
                                handle_incoming_connection(conn).await;
                            });
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to initialize QUIC endpoint: {}", e);
                    }
                }
            });

            log::info!("LAN Meeting started");
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Handle incoming QUIC connection
async fn handle_incoming_connection(conn: Arc<network::quic::QuicConnection>) {
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

            // Send handshake acknowledgment
            let our_id = network::discovery::get_our_device_id();
            let our_name = hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "Unknown".to_string());

            let ack = protocol::create_handshake_ack(&our_id, &our_name, true, None);
            let encoded = protocol::encode(&ack)?;
            stream.send_framed(&encoded).await?;

            log::info!("Handshake accepted, sent acknowledgment");
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

        // Screen sharing messages will be handled in Phase 3
        Message::ScreenOffer { .. }
        | Message::ScreenRequest { .. }
        | Message::ScreenStart { .. }
        | Message::ScreenFrame { .. }
        | Message::ScreenStop => {
            log::debug!("Screen sharing message received (not yet implemented)");
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

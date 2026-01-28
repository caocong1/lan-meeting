//! Tauri commands - Frontend API

use crate::capture::ScreenCapture;
use crate::network::discovery::{self, DeviceStatus, DiscoveredDevice};
use crate::network::quic;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Display information for screen capture
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub primary: bool,
}

/// Global screen capture instance
static CAPTURE: once_cell::sync::Lazy<Mutex<Option<Box<dyn ScreenCapture>>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

/// Initialize the capture module
fn ensure_capture_initialized() -> Result<(), String> {
    let mut capture = CAPTURE.lock();
    if capture.is_none() {
        *capture = Some(
            crate::capture::create_capture()
                .map_err(|e| format!("Failed to initialize capture: {}", e))?,
        );
    }
    Ok(())
}

/// Get available displays for screen capture
#[tauri::command]
pub async fn get_displays() -> Result<Vec<DisplayInfo>, String> {
    ensure_capture_initialized()?;

    let mut capture = CAPTURE.lock();
    let capture = capture
        .as_mut()
        .ok_or_else(|| "Capture not initialized".to_string())?;

    let displays = capture
        .get_displays()
        .map_err(|e| format!("Failed to get displays: {}", e))?;

    Ok(displays
        .into_iter()
        .map(|d| DisplayInfo {
            id: d.id,
            name: d.name,
            width: d.width,
            height: d.height,
            scale_factor: d.scale_factor,
            primary: d.primary,
        })
        .collect())
}

/// Start screen capture for a specific display
#[tauri::command]
pub async fn start_capture(display_id: u32) -> Result<(), String> {
    log::info!("Starting capture for display {}", display_id);

    ensure_capture_initialized()?;

    let mut capture = CAPTURE.lock();
    let capture = capture
        .as_mut()
        .ok_or_else(|| "Capture not initialized".to_string())?;

    capture
        .start(display_id)
        .map_err(|e| format!("Failed to start capture: {}", e))?;

    log::info!("Screen capture started for display {}", display_id);
    Ok(())
}

/// Stop screen capture
#[tauri::command]
pub async fn stop_capture() -> Result<(), String> {
    log::info!("Stopping capture");

    let mut capture = CAPTURE.lock();
    if let Some(capture) = capture.as_mut() {
        capture
            .stop()
            .map_err(|e| format!("Failed to stop capture: {}", e))?;
    }

    log::info!("Screen capture stopped");
    Ok(())
}

/// Check if screen recording permission is granted (macOS)
#[tauri::command]
pub fn check_screen_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::capture::macos::MacOSCapture::has_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Request screen recording permission (macOS)
#[tauri::command]
pub fn request_screen_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::capture::macos::MacOSCapture::request_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Get discovered devices on the network
#[tauri::command]
pub async fn get_devices() -> Result<Vec<DiscoveredDevice>, String> {
    Ok(discovery::get_devices())
}

/// Add a device manually by IP address
#[tauri::command]
pub async fn add_manual_device(ip: String) -> Result<DiscoveredDevice, String> {
    log::info!("Adding manual device: {}", ip);
    discovery::add_manual_device(ip, 19876)
        .await
        .map_err(|e| e.to_string())
}

/// Connect to a remote device
#[tauri::command]
pub async fn connect_to_device(device_id: String) -> Result<(), String> {
    use crate::network::protocol;

    log::info!("Connecting to device {}", device_id);

    // Get device info
    let device = discovery::get_devices()
        .into_iter()
        .find(|d| d.id == device_id)
        .ok_or_else(|| format!("Device not found: {}", device_id))?;

    // Parse address
    let addr: SocketAddr = format!("{}:{}", device.ip, device.port)
        .parse()
        .map_err(|e| format!("Invalid address: {}", e))?;

    // Get QUIC endpoint
    let endpoint = crate::get_quic_endpoint()
        .ok_or_else(|| "QUIC endpoint not initialized".to_string())?;

    // Connect to device
    let conn = endpoint
        .connect(addr)
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    log::info!("Connected to {} at {}", device.name, conn.remote_addr());

    // Update device status
    discovery::update_device_status(&device_id, DeviceStatus::Busy);

    // Open a control stream and send handshake
    let mut stream = conn
        .open_bi_stream()
        .await
        .map_err(|e| format!("Failed to open stream: {}", e))?;

    // Create and send proper protocol handshake
    let our_id = discovery::get_our_device_id();
    let our_name = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "Unknown".to_string());

    let handshake = protocol::create_handshake(&our_id, &our_name);
    let encoded = protocol::encode(&handshake)
        .map_err(|e| format!("Failed to encode handshake: {}", e))?;

    stream
        .send_framed(&encoded)
        .await
        .map_err(|e| format!("Failed to send handshake: {}", e))?;

    log::info!("Handshake sent to {}", device.name);

    // Wait for handshake acknowledgment
    let response = stream
        .recv_framed()
        .await
        .map_err(|e| format!("Failed to receive handshake ack: {}", e))?;

    let ack = protocol::decode(&response)
        .map_err(|e| format!("Failed to decode handshake ack: {}", e))?;

    match ack {
        protocol::Message::HandshakeAck { accepted, reason, name, .. } => {
            if accepted {
                log::info!("Connection accepted by {}", name);
                Ok(())
            } else {
                let err_msg = reason.unwrap_or_else(|| "Unknown reason".to_string());
                log::warn!("Connection rejected by {}: {}", name, err_msg);
                Err(format!("Connection rejected: {}", err_msg))
            }
        }
        _ => Err("Unexpected response to handshake".to_string()),
    }
}

/// Disconnect from current session
#[tauri::command]
pub async fn disconnect(device_id: Option<String>) -> Result<(), String> {
    log::info!("Disconnecting from {:?}", device_id);

    if let Some(id) = &device_id {
        // Get device to find connection ID
        if let Some(device) = discovery::get_devices().into_iter().find(|d| d.id == *id) {
            let conn_id = format!("{}:{}", device.ip, device.port);

            // Close and remove connection
            if let Some(conn) = quic::get_connection(&conn_id) {
                conn.close();
            }
            quic::remove_connection(&conn_id);

            // Update device status
            discovery::update_device_status(id, DeviceStatus::Online);
        }
    } else {
        // Disconnect all - get all connection IDs first to avoid holding lock
        let conn_ids: Vec<String> = quic::CONNECTIONS.read().keys().cloned().collect();

        for conn_id in conn_ids {
            if let Some(conn) = quic::get_connection(&conn_id) {
                conn.close();
            }
            quic::remove_connection(&conn_id);
        }
    }

    Ok(())
}

/// Get our own device info
#[tauri::command]
pub fn get_self_info() -> Result<SelfInfo, String> {
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "Unknown".to_string());

    Ok(SelfInfo {
        id: discovery::get_our_device_id().to_string(),
        name: hostname,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfInfo {
    pub id: String,
    pub name: String,
}

// ===== Chat commands =====

/// Send a chat message
#[tauri::command]
pub async fn send_chat_message(content: String) -> Result<crate::chat::ChatMessage, String> {
    use crate::network::protocol;

    let self_info = get_self_info()?;
    let message = crate::chat::send_message(&content, &self_info.id, &self_info.name);

    // Send to connected peers via QUIC
    let chat_msg = protocol::Message::ChatMessage {
        from: self_info.name.clone(),
        content: content.clone(),
        timestamp: message.timestamp,
    };

    if let Ok(encoded) = protocol::encode(&chat_msg) {
        let results = quic::broadcast_message(&encoded).await;
        let success_count = results.iter().filter(|r| r.is_ok()).count();
        log::info!("Chat message broadcast to {} peers", success_count);
    }

    Ok(message)
}

/// Get chat message history
#[tauri::command]
pub fn get_chat_messages() -> Vec<crate::chat::ChatMessage> {
    crate::chat::get_chat_manager().get_messages()
}

// ===== Input permission commands =====

/// Check if input control permission is granted
#[tauri::command]
pub fn check_input_permission() -> bool {
    crate::input::has_permission()
}

/// Request input control permission
#[tauri::command]
pub fn request_input_permission() -> bool {
    crate::input::request_permission()
}

// ===== File transfer commands =====

use crate::transfer::{self, FileTransfer};
use std::path::Path;

/// Offer a file for transfer to a peer
#[tauri::command]
pub async fn offer_file(file_path: String, peer_id: String) -> Result<FileTransfer, String> {
    use crate::network::protocol;

    log::info!("Offering file {} to {}", file_path, peer_id);

    let path = Path::new(&file_path);
    let transfer = transfer::get_transfer_manager()
        .offer_file(path, &peer_id)
        .map_err(|e| e.to_string())?;

    // Send FileOffer message to peer via QUIC
    let offer_msg = protocol::Message::FileOffer {
        file_id: transfer.info.id.clone(),
        name: transfer.info.name.clone(),
        size: transfer.info.size,
        checksum: transfer.info.checksum.clone(),
    };

    if let Ok(encoded) = protocol::encode(&offer_msg) {
        if let Err(e) = quic::send_to_peer(&peer_id, &encoded).await {
            log::warn!("Failed to send file offer to peer: {}", e);
        }
    }

    log::info!("File offer created: {} ({} bytes)", transfer.info.name, transfer.info.size);

    Ok(transfer)
}

/// Accept an incoming file transfer
#[tauri::command]
pub async fn accept_file_transfer(file_id: String, dest_path: Option<String>) -> Result<(), String> {
    use crate::network::protocol;

    log::info!("Accepting file transfer: {}", file_id);

    // Get peer_id before accepting
    let peer_id = transfer::get_transfer_manager()
        .get_transfer(&file_id)
        .map(|t| t.peer_id.clone())
        .ok_or_else(|| "Transfer not found".to_string())?;

    let dest = dest_path.as_ref().map(|p| Path::new(p));
    transfer::get_transfer_manager()
        .accept_transfer(&file_id, dest)
        .map_err(|e| e.to_string())?;

    // Send FileAccept message to peer via QUIC
    let accept_msg = protocol::Message::FileAccept {
        file_id: file_id.clone(),
    };

    if let Ok(encoded) = protocol::encode(&accept_msg) {
        if let Err(e) = quic::send_to_peer(&peer_id, &encoded).await {
            log::warn!("Failed to send file accept to peer: {}", e);
        }
    }

    log::info!("File transfer accepted: {}", file_id);

    Ok(())
}

/// Reject an incoming file transfer
#[tauri::command]
pub async fn reject_file_transfer(file_id: String) -> Result<(), String> {
    use crate::network::protocol;

    log::info!("Rejecting file transfer: {}", file_id);

    // Get peer_id before rejecting
    let peer_id = transfer::get_transfer_manager()
        .get_transfer(&file_id)
        .map(|t| t.peer_id.clone());

    transfer::get_transfer_manager()
        .reject_transfer(&file_id)
        .map_err(|e| e.to_string())?;

    // Send FileReject message to peer via QUIC
    if let Some(peer_id) = peer_id {
        let reject_msg = protocol::Message::FileReject {
            file_id: file_id.clone(),
        };

        if let Ok(encoded) = protocol::encode(&reject_msg) {
            if let Err(e) = quic::send_to_peer(&peer_id, &encoded).await {
                log::warn!("Failed to send file reject to peer: {}", e);
            }
        }
    }

    log::info!("File transfer rejected: {}", file_id);

    Ok(())
}

/// Cancel a file transfer
#[tauri::command]
pub async fn cancel_file_transfer(file_id: String) -> Result<(), String> {
    use crate::network::protocol;

    log::info!("Cancelling file transfer: {}", file_id);

    // Get peer_id before cancelling
    let peer_id = transfer::get_transfer_manager()
        .get_transfer(&file_id)
        .map(|t| t.peer_id.clone());

    transfer::get_transfer_manager()
        .cancel_transfer(&file_id)
        .map_err(|e| e.to_string())?;

    // Send FileCancel message to peer via QUIC
    if let Some(peer_id) = peer_id {
        let cancel_msg = protocol::Message::FileCancel {
            file_id: file_id.clone(),
        };

        if let Ok(encoded) = protocol::encode(&cancel_msg) {
            if let Err(e) = quic::send_to_peer(&peer_id, &encoded).await {
                log::warn!("Failed to send file cancel to peer: {}", e);
            }
        }
    }

    log::info!("File transfer cancelled: {}", file_id);

    Ok(())
}

/// Get all file transfers
#[tauri::command]
pub fn get_file_transfers() -> Vec<FileTransfer> {
    transfer::get_transfer_manager().get_all_transfers()
}

/// Get active file transfers
#[tauri::command]
pub fn get_active_file_transfers() -> Vec<FileTransfer> {
    transfer::get_transfer_manager().get_active_transfers()
}

/// Get a specific file transfer
#[tauri::command]
pub fn get_file_transfer(file_id: String) -> Option<FileTransfer> {
    transfer::get_transfer_manager().get_transfer(&file_id)
}

/// Get the default download directory
#[tauri::command]
pub fn get_download_directory() -> String {
    transfer::get_transfer_manager()
        .download_dir()
        .to_string_lossy()
        .to_string()
}

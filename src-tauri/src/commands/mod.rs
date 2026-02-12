//! Tauri commands - Frontend API

use crate::capture::ScreenCapture;
use crate::network::discovery::{self, DeviceStatus, DiscoveredDevice};
use crate::network::quic;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tauri::Manager;

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
    // Clean up dead connections and their devices before returning
    crate::network::quic::cleanup_dead_connections();
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

    // Get local IP address
    let ip = get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());

    Ok(SelfInfo {
        id: discovery::get_our_device_id().to_string(),
        name: hostname,
        ip,
    })
}

/// Check if an IPv4 address is a real private LAN IP
/// (not a VPN/proxy virtual interface like 198.18.0.0/15)
pub fn is_real_lan_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 198.18.0.0/15 — Surge, ClashX and similar proxy tools
            if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
                return false;
            }
            // 100.64.0.0/10 — CGNAT / Tailscale / some VPNs
            if octets[0] == 100 && (octets[1] & 0xC0) == 64 {
                return false;
            }
            // Accept standard private ranges
            // 10.0.0.0/8
            if octets[0] == 10 {
                return true;
            }
            // 172.16.0.0/12
            if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                return true;
            }
            // 192.168.0.0/16
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }
            // 169.254.0.0/16 — link-local (better than nothing)
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
            false
        }
        std::net::IpAddr::V6(_) => false,
    }
}

/// Get local IP address, preferring real LAN IPs over VPN interfaces
fn get_local_ip() -> Option<String> {
    use std::net::UdpSocket;

    // Try multiple targets to get IPs from different routing paths
    let targets = ["8.8.8.8:80", "192.168.1.1:80", "10.0.0.1:80"];
    let mut candidates = Vec::new();

    for target in &targets {
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
            if socket.connect(target).is_ok() {
                if let Ok(addr) = socket.local_addr() {
                    let ip = addr.ip();
                    if !ip.is_loopback() && !candidates.contains(&ip) {
                        candidates.push(ip);
                    }
                }
            }
        }
    }

    // Prefer real LAN IPs over VPN IPs
    if let Some(lan_ip) = candidates.iter().find(|ip| is_real_lan_ip(ip)) {
        return Some(lan_ip.to_string());
    }

    // Fall back to any non-loopback IP
    candidates.first().map(|ip| ip.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfInfo {
    pub id: String,
    pub name: String,
    pub ip: String,
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

// ===== Service commands =====

use crate::network::quic::{QuicConfig, QuicEndpoint};
use std::sync::Arc;

/// Service state
static SERVICE_RUNNING: once_cell::sync::Lazy<parking_lot::RwLock<bool>> =
    once_cell::sync::Lazy::new(|| parking_lot::RwLock::new(false));

/// Start the network service (mDNS discovery + QUIC server)
#[tauri::command]
pub async fn start_service(app_handle: tauri::AppHandle) -> Result<(), String> {
    if *SERVICE_RUNNING.read() {
        return Ok(()); // Already running
    }

    log::info!("Starting network service");

    // Start mDNS discovery
    let handle = app_handle.clone();
    tokio::spawn(async move {
        if let Err(e) = discovery::start_discovery(handle).await {
            log::error!("Failed to start mDNS discovery: {}", e);
        }
    });

    // Start QUIC endpoint
    match QuicEndpoint::new(QuicConfig::default()).await {
        Ok(endpoint) => {
            let endpoint = Arc::new(endpoint);
            log::info!("QUIC endpoint initialized on {}", endpoint.local_addr());

            // Store globally
            let _ = crate::QUIC_ENDPOINT.set(endpoint.clone());

            // Start accepting connections
            endpoint.start_server(|conn| {
                log::info!("Incoming connection from {}", conn.remote_addr());
                tokio::spawn(async move {
                    crate::handle_incoming_connection(conn).await;
                });
            });
        }
        Err(e) => {
            log::error!("Failed to initialize QUIC endpoint: {}", e);
            return Err(format!("Failed to start QUIC: {}", e));
        }
    }

    *SERVICE_RUNNING.write() = true;
    log::info!("Network service started");

    Ok(())
}

/// Stop the network service
#[tauri::command]
pub async fn stop_service() -> Result<(), String> {
    log::info!("Stopping network service");

    // Disconnect all peers
    disconnect(None).await?;

    // Clear device list
    discovery::clear_devices();

    *SERVICE_RUNNING.write() = false;

    Ok(())
}

/// Check if service is running
#[tauri::command]
pub fn is_service_running() -> bool {
    *SERVICE_RUNNING.read()
}

// ===== Settings commands =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub device_name: String,
    pub quality: String,
    pub fps: u32,
    /// Default resolution index for viewer toolbar (0=720p, 1=1080p, 2=1440p, 3=Original)
    #[serde(default)]
    pub default_resolution: u32,
    /// Default bitrate index for viewer toolbar (0=2M, 1=4M, 2=8M, 3=12M)
    #[serde(default)]
    pub default_bitrate: u32,
}

/// Global settings
static SETTINGS: once_cell::sync::Lazy<parking_lot::RwLock<AppSettings>> =
    once_cell::sync::Lazy::new(|| {
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "Unknown".to_string());

        parking_lot::RwLock::new(AppSettings {
            device_name: hostname,
            quality: "auto".to_string(),
            fps: 30,
            default_resolution: 1, // 1080p
            default_bitrate: 1,    // 4 Mbps
        })
    });

/// Get current settings
#[tauri::command]
pub fn get_settings() -> AppSettings {
    SETTINGS.read().clone()
}

/// Save settings
#[tauri::command]
pub fn save_settings(settings: AppSettings) -> Result<(), String> {
    log::info!("Saving settings: {:?}", settings);
    *SETTINGS.write() = settings;
    Ok(())
}

/// Get default resolution and bitrate indices for viewer toolbar
pub fn get_default_streaming_indices() -> (usize, usize) {
    let s = SETTINGS.read();
    (s.default_resolution as usize, s.default_bitrate as usize)
}

// ===== Sharing status commands =====

/// Sharing state
static IS_SHARING: once_cell::sync::Lazy<parking_lot::RwLock<bool>> =
    once_cell::sync::Lazy::new(|| parking_lot::RwLock::new(false));

/// Broadcast sharing status to all connected peers
#[tauri::command]
pub async fn broadcast_sharing_status(is_sharing: bool, display_id: Option<u32>) -> Result<(), String> {
    use crate::network::protocol;
    use crate::streaming::{get_streaming_manager, StreamingConfig, Quality, StreamingManager};

    log::info!("Broadcasting sharing status: {} (display: {:?})", is_sharing, display_id);

    *IS_SHARING.write() = is_sharing;

    // Start or stop streaming
    if is_sharing {
        // Start streaming
        let capture = crate::capture::create_capture()
            .map_err(|e| format!("Failed to create capture: {}", e))?;

        let settings = SETTINGS.read().clone();
        let config = StreamingConfig {
            fps: settings.fps,
            quality: match settings.quality.as_str() {
                "high" => Quality::High,
                "medium" => Quality::Medium,
                "low" => Quality::Low,
                _ => Quality::Auto,
            },
            display_id: display_id.unwrap_or(0),
        };

        // Initialize manager if needed (sync operation)
        {
            let manager_arc = get_streaming_manager();
            let mut manager = manager_arc.write();
            if manager.is_none() {
                *manager = Some(StreamingManager::new());
            }
        }

        // Start streaming (spawns a background task, doesn't need to hold lock long)
        let manager_arc = get_streaming_manager();
        let start_result = {
            let mut manager = manager_arc.write();
            if let Some(ref mut m) = *manager {
                Some(m.start_sync(config, capture))
            } else {
                None
            }
        };

        if let Some(result) = start_result {
            result.map_err(|e| format!("Failed to start streaming: {}", e))?;
        }
    } else {
        // Stop streaming (sync operation)
        let manager_arc = get_streaming_manager();
        let mut manager = manager_arc.write();
        if let Some(ref mut m) = *manager {
            m.stop_sync();
        }
    }

    // Create sharing status message
    let msg = protocol::Message::ScreenOffer {
        displays: if is_sharing {
            // Get current displays
            match get_displays().await {
                Ok(displays) => displays
                    .into_iter()
                    .map(|d| protocol::DisplayInfo {
                        id: d.id,
                        name: d.name,
                        width: d.width,
                        height: d.height,
                        primary: d.primary,
                    })
                    .collect(),
                Err(_) => vec![],
            }
        } else {
            vec![]
        },
    };

    if let Ok(encoded) = protocol::encode(&msg) {
        let results = quic::broadcast_message(&encoded).await;
        let success_count = results.iter().filter(|r| r.is_ok()).count();
        log::info!("Sharing status broadcast to {} peers", success_count);
    }

    Ok(())
}

/// Request screen stream from a peer (creates native render window)
#[tauri::command]
pub async fn request_screen_stream(peer_ip: String, peer_name: String) -> Result<(), String> {
    use crate::streaming;

    log::info!("Requesting screen stream from {} ({})", peer_name, peer_ip);

    // Ensure we have an active QUIC connection to this peer
    ensure_peer_connection(&peer_ip).await?;

    // Create viewer session (native window will be created on ScreenStart)
    streaming::create_viewer_session(peer_ip.clone(), peer_name)
        .map_err(|e| format!("Failed to create viewer session: {}", e))?;

    // Send request to peer
    streaming::request_screen_stream(&peer_ip, 0)
        .await
        .map_err(|e| format!("Failed to request stream: {}", e))?;

    Ok(())
}

/// Ensure there is an active QUIC connection to the peer, reconnecting if needed
async fn ensure_peer_connection(peer_ip: &str) -> Result<(), String> {
    use crate::network::discovery;

    // Check if we already have a live connection
    if let Some(conn) = quic::find_connection(peer_ip) {
        if conn.is_alive() {
            log::debug!("Existing connection to {} is alive", peer_ip);
            return Ok(());
        }
        log::warn!("Connection to {} is dead, will reconnect", peer_ip);
        quic::remove_connection_by_ip(peer_ip);
    }

    log::info!("No active connection to {}, establishing...", peer_ip);

    // Find the device to get port info
    let port = discovery::get_devices()
        .into_iter()
        .find(|d| d.ip == peer_ip)
        .map(|d| d.port)
        .unwrap_or(quic::DEFAULT_PORT);

    let addr: SocketAddr = format!("{}:{}", peer_ip, port)
        .parse()
        .map_err(|e| format!("Invalid address: {}", e))?;

    // Get QUIC endpoint
    let endpoint = crate::get_quic_endpoint()
        .ok_or_else(|| "QUIC endpoint not initialized - start service first".to_string())?;

    // Connect with timeout
    let conn = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        endpoint.connect(addr),
    )
    .await
    .map_err(|_| format!("Connection to {} timed out", peer_ip))?
    .map_err(|e| format!("Failed to connect to {}: {}", peer_ip, e))?;

    log::info!("Connected to {} at {}", peer_ip, conn.remote_addr());

    // Send handshake
    let our_id = discovery::get_our_device_id();
    let our_name = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "Unknown".to_string());

    let handshake = crate::network::protocol::create_handshake(&our_id, &our_name);
    let encoded = crate::network::protocol::encode(&handshake)
        .map_err(|e| format!("Failed to encode handshake: {}", e))?;

    let mut stream = conn
        .open_bi_stream()
        .await
        .map_err(|e| format!("Failed to open handshake stream: {}", e))?;

    stream
        .send_framed(&encoded)
        .await
        .map_err(|e| format!("Failed to send handshake: {}", e))?;

    // Wait for handshake ack
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.recv_framed(),
    )
    .await
    .map_err(|_| "Handshake ack timed out".to_string())?
    .map_err(|e| format!("Failed to receive handshake ack: {}", e))?;

    let ack = crate::network::protocol::decode(&response)
        .map_err(|e| format!("Failed to decode handshake ack: {}", e))?;

    match ack {
        crate::network::protocol::Message::HandshakeAck { accepted, reason, name, .. } => {
            if !accepted {
                return Err(format!("Connection rejected: {}", reason.unwrap_or_default()));
            }
            log::info!("Reconnected and handshake accepted by {}", name);
        }
        _ => return Err("Unexpected handshake response".to_string()),
    }

    // Start listening for incoming messages on this connection
    let conn_clone = conn.clone();
    tokio::spawn(async move {
        crate::handle_incoming_connection(conn_clone).await;
    });

    Ok(())
}

/// Stop viewing a screen stream
#[tauri::command]
pub fn stop_viewing_stream(peer_ip: String) -> Result<(), String> {
    use crate::streaming;

    log::info!("Stopping stream viewer for {}", peer_ip);
    streaming::remove_viewer_session(&peer_ip);
    Ok(())
}

/// Open viewer window to watch a peer's screen
#[tauri::command]
pub async fn open_viewer_window(
    app_handle: tauri::AppHandle,
    peer_id: String,
    peer_name: String,
    peer_ip: String,
) -> Result<(), String> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    log::info!("Opening viewer window for {} ({})", peer_name, peer_ip);

    // Create unique window label
    let window_label = format!("viewer-{}", peer_id.replace(".", "-").replace(":", "-"));

    // Check if window already exists
    if let Some(window) = app_handle.get_webview_window(&window_label) {
        log::info!("Viewer window already exists, focusing it");
        let _ = window.set_focus();
        return Ok(());
    }

    // Create viewer URL with query parameters
    let viewer_url = format!(
        "/viewer.html?peer_id={}&peer_name={}&peer_ip={}",
        urlencoding::encode(&peer_id),
        urlencoding::encode(&peer_name),
        urlencoding::encode(&peer_ip)
    );

    // Create new window
    let _window = WebviewWindowBuilder::new(
        &app_handle,
        &window_label,
        WebviewUrl::App(viewer_url.into()),
    )
    .title(format!("{} 的屏幕", peer_name))
    .inner_size(1280.0, 720.0)
    .min_inner_size(640.0, 480.0)
    .resizable(true)
    .center()
    .build()
    .map_err(|e| format!("Failed to create viewer window: {}", e))?;

    log::info!("Viewer window created: {}", window_label);

    Ok(())
}

/// Request control of a peer's screen
#[tauri::command]
pub async fn request_control(peer_id: String) -> Result<(), String> {
    use crate::network::protocol;

    log::info!("Requesting control of {}", peer_id);

    let self_info = get_self_info()?;
    let msg = protocol::Message::ControlRequest {
        from_user: self_info.name,
    };

    if let Ok(encoded) = protocol::encode(&msg) {
        quic::send_to_peer(&peer_id, &encoded)
            .await
            .map_err(|e| format!("Failed to send control request: {}", e))?;
    }

    Ok(())
}

// ===== Simple streaming commands (minimal pipeline for debugging) =====

/// Start simple screen sharing (OpenH264 only, no optimizations)
#[tauri::command]
pub async fn simple_start_sharing(display_id: u32) -> Result<(), String> {
    log::info!("[SIMPLE] Command: simple_start_sharing(display_id={})", display_id);
    crate::simple_streaming::start_sharing(display_id)
}

/// Request simple screen stream from a peer
#[tauri::command]
pub async fn simple_request_stream(peer_ip: String) -> Result<(), String> {
    use crate::network::protocol;

    log::info!("[SIMPLE] Command: simple_request_stream(peer_ip={})", peer_ip);

    // Ensure connection
    ensure_peer_connection(&peer_ip).await?;

    // Send SimpleScreenRequest to the sharer
    let msg = protocol::Message::SimpleScreenRequest { display_id: 0 };
    let encoded = protocol::encode(&msg)
        .map_err(|e| format!("[SIMPLE] Failed to encode request: {}", e))?;

    quic::send_to_peer(&peer_ip, &encoded)
        .await
        .map_err(|e| format!("[SIMPLE] Failed to send request to {}: {}", peer_ip, e))?;

    log::info!("[SIMPLE] Request sent to {}", peer_ip);
    Ok(())
}

/// Stop simple screen sharing
#[tauri::command]
pub async fn simple_stop_sharing() -> Result<(), String> {
    log::info!("[SIMPLE] Command: simple_stop_sharing");
    crate::simple_streaming::stop_sharing();
    Ok(())
}

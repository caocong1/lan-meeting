//! mDNS service discovery
//! Automatically find other LAN Meeting instances on the network

use super::NetworkError;
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

const SERVICE_TYPE: &str = "_lan-meeting._udp.local.";
const SERVICE_PORT: u16 = 19876;

/// Discovered device information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub status: DeviceStatus,
    pub last_seen: u64,
    #[serde(default)]
    pub is_sharing: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    Online,
    Busy,
    Offline,
}

/// Global device registry
pub static DEVICES: once_cell::sync::Lazy<Arc<RwLock<HashMap<String, DiscoveredDevice>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Our own device ID
static OUR_DEVICE_ID: once_cell::sync::Lazy<String> =
    once_cell::sync::Lazy::new(|| uuid::Uuid::new_v4().to_string());

/// mDNS service daemon handle
static MDNS_DAEMON: once_cell::sync::Lazy<Option<ServiceDaemon>> =
    once_cell::sync::Lazy::new(|| match ServiceDaemon::new() {
        Ok(daemon) => {
            log::info!("mDNS daemon created successfully");
            Some(daemon)
        }
        Err(e) => {
            log::error!("Failed to create mDNS daemon: {}", e);
            None
        }
    });

/// Get our device ID
pub fn get_our_device_id() -> &'static str {
    &OUR_DEVICE_ID
}

/// Get current timestamp in milliseconds
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Start mDNS discovery and service advertisement
pub async fn start_discovery(app: AppHandle) -> Result<(), NetworkError> {
    log::info!("Starting mDNS discovery on {}", SERVICE_TYPE);

    let daemon = MDNS_DAEMON
        .as_ref()
        .ok_or_else(|| NetworkError::DiscoveryError("Failed to create mDNS daemon".to_string()))?;

    // Register our service
    register_service(daemon)?;

    // Start browsing for other services
    browse_services(daemon, app)?;

    Ok(())
}

/// Register our service on the network
fn register_service(daemon: &ServiceDaemon) -> Result<(), NetworkError> {
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "Unknown".to_string());

    let device_id = get_our_device_id();
    let instance_name = format!("{}-{}", hostname, &device_id[..8]);

    log::info!(
        "Registering mDNS service: {} on port {}",
        instance_name,
        SERVICE_PORT
    );

    // Create service info with properties
    let mut properties = HashMap::new();
    properties.insert("id".to_string(), device_id.to_string());
    properties.insert("name".to_string(), hostname.clone());
    properties.insert("version".to_string(), env!("CARGO_PKG_VERSION").to_string());

    // Collect our real LAN IPs to register with mDNS
    let lan_ips: Vec<String> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .iter()
        .filter(|iface| !iface.is_loopback())
        .filter(|iface| iface.ip().is_ipv4())
        .filter(|iface| crate::commands::is_real_lan_ip(&iface.ip()))
        .map(|iface| iface.ip().to_string())
        .collect();

    let ip_str = if lan_ips.is_empty() {
        log::warn!("No real LAN IPs found, using addr_auto for mDNS");
        String::new()
    } else {
        log::info!("Registering mDNS with IPs: {:?}", lan_ips);
        lan_ips.join(",")
    };

    let service_info = ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name,
        &format!("{}.local.", hostname),
        &ip_str,
        SERVICE_PORT,
        properties,
    )
    .map_err(|e| NetworkError::DiscoveryError(format!("Failed to create service info: {}", e)))?
    // Enable automatic address management so the service stays updated
    // when network interfaces change (e.g., VPN connect/disconnect)
    .enable_addr_auto();

    daemon
        .register(service_info)
        .map_err(|e| NetworkError::DiscoveryError(format!("Failed to register service: {}", e)))?;

    log::info!("mDNS service registered successfully");
    Ok(())
}

/// Browse for other services on the network
fn browse_services(daemon: &ServiceDaemon, app: AppHandle) -> Result<(), NetworkError> {
    log::info!("Browsing for LAN Meeting services...");

    let receiver = daemon.browse(SERVICE_TYPE).map_err(|e| {
        NetworkError::DiscoveryError(format!("Failed to start browsing: {}", e))
    })?;

    // Spawn a task to handle service events
    std::thread::spawn(move || {
        loop {
            match receiver.recv() {
                Ok(event) => {
                    handle_service_event(event, &app);
                }
                Err(e) => {
                    log::error!("mDNS browse error: {}", e);
                    break;
                }
            }
        }
    });

    Ok(())
}

/// Handle mDNS service events
fn handle_service_event(event: ServiceEvent, app: &AppHandle) {
    match event {
        ServiceEvent::ServiceResolved(info) => {
            // Skip our own service
            if let Some(prop) = info.txt_properties.get("id") {
                if prop.val_str() == get_our_device_id() {
                    return;
                }
            }

            // Log all addresses for debugging VPN/virtual interface issues
            let all_addrs: Vec<String> = info.addresses.iter().map(|a| a.to_ip_addr().to_string()).collect();
            log::debug!("mDNS resolved addresses: {:?}", all_addrs);

            let device = extract_device_info(&info);
            if let Some(device) = device {
                log::info!("Discovered device: {} ({})", device.name, device.ip);
                add_device(device.clone());

                // Notify frontend
                let _ = app.emit("device-discovered", &device);
            }
        }
        ServiceEvent::ServiceRemoved(_type, fullname) => {
            // Extract device ID from fullname
            if let Some(device) = find_device_by_fullname(&fullname) {
                log::info!("Device removed: {} ({})", device.name, device.ip);
                remove_device(&device.id);

                // Notify frontend
                let _ = app.emit("device-removed", &device.id);
            }
        }
        ServiceEvent::SearchStarted(_) => {
            log::debug!("mDNS search started");
        }
        ServiceEvent::SearchStopped(_) => {
            log::debug!("mDNS search stopped");
        }
        _ => {}
    }
}

/// Extract device info from ResolvedService
fn extract_device_info(info: &ResolvedService) -> Option<DiscoveredDevice> {
    let id = info.txt_properties.get("id")?.val_str().to_string();
    let name = info
        .txt_properties
        .get("name")
        .map(|prop| prop.val_str().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    // Collect all IPv4 addresses from the resolved service
    let ipv4_addrs: Vec<std::net::IpAddr> = info
        .addresses
        .iter()
        .map(|scoped_ip| scoped_ip.to_ip_addr())
        .filter(|ip| ip.is_ipv4() && !ip.is_loopback())
        .collect();

    // Priority: 1) same subnet as us, 2) real LAN IP, 3) any IPv4
    let our_subnets = crate::commands::get_local_subnets();
    let ip = ipv4_addrs
        .iter()
        .find(|ip| crate::commands::is_same_subnet(ip, &our_subnets))
        .or_else(|| ipv4_addrs.iter().find(|ip| crate::commands::is_real_lan_ip(ip)))
        .or_else(|| ipv4_addrs.first())
        .map(|ip| ip.to_string())?;

    let port = info.port;

    Some(DiscoveredDevice {
        id,
        name,
        ip,
        port,
        status: DeviceStatus::Online,
        last_seen: now_ms(),
        is_sharing: false,
    })
}

/// Find device by mDNS fullname
fn find_device_by_fullname(fullname: &str) -> Option<DiscoveredDevice> {
    let devices = DEVICES.read();
    // The fullname contains the instance name, try to match
    devices.values().find(|d| fullname.contains(&d.id[..8])).cloned()
}

/// Get all discovered devices
pub fn get_devices() -> Vec<DiscoveredDevice> {
    DEVICES.read().values().cloned().collect()
}

/// Add or update a device
pub fn add_device(device: DiscoveredDevice) {
    let mut devices = DEVICES.write();
    devices.insert(device.id.clone(), device);
}

/// Remove a device
pub fn remove_device(id: &str) {
    let mut devices = DEVICES.write();
    devices.remove(id);
}

/// Clear all devices
pub fn clear_devices() {
    let mut devices = DEVICES.write();
    devices.clear();
}

/// Update device status
pub fn update_device_status(id: &str, status: DeviceStatus) {
    let mut devices = DEVICES.write();
    if let Some(device) = devices.get_mut(id) {
        device.status = status;
        device.last_seen = now_ms();
    }
}

/// Update device sharing status
pub fn update_device_sharing(id: &str, is_sharing: bool) {
    let mut devices = DEVICES.write();
    if let Some(device) = devices.get_mut(id) {
        device.is_sharing = is_sharing;
        device.last_seen = now_ms();
    }
}

/// Update device sharing status by IP
pub fn update_device_sharing_by_ip(ip: &str, is_sharing: bool) -> Option<String> {
    let mut devices = DEVICES.write();
    for device in devices.values_mut() {
        if device.ip == ip {
            device.is_sharing = is_sharing;
            device.last_seen = now_ms();
            return Some(device.id.clone());
        }
    }
    None
}

/// Manually add a device by IP address
/// This will attempt to connect and exchange handshake to verify the device
pub async fn add_manual_device(ip: String, port: u16) -> Result<DiscoveredDevice, NetworkError> {
    use super::protocol;
    use std::net::SocketAddr;
    use std::time::Duration;

    let addr: SocketAddr = format!("{}:{}", ip, port)
        .parse()
        .map_err(|e| NetworkError::ConnectionFailed(format!("Invalid address: {}", e)))?;

    // Try to connect with a timeout to verify the device is reachable
    let endpoint = crate::get_quic_endpoint()
        .ok_or_else(|| NetworkError::ConnectionFailed("请先开启服务".to_string()))?;

    // Attempt connection with timeout
    let connect_future = endpoint.connect(addr);
    let timeout_duration = Duration::from_secs(5);

    let conn = match tokio::time::timeout(timeout_duration, connect_future).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(e)) => {
            log::warn!("Failed to connect to manual device {}: {}", ip, e);
            return Err(NetworkError::ConnectionFailed(format!(
                "连接失败: 对方可能未开启服务"
            )));
        }
        Err(_) => {
            log::warn!("Connection timeout to manual device {}", ip);
            return Err(NetworkError::ConnectionFailed(
                "连接超时: 对方可能未开启服务".to_string(),
            ));
        }
    };

    // Send handshake to get device info
    let our_id = get_our_device_id();
    let our_name = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "Unknown".to_string());

    let handshake = protocol::create_handshake(&our_id, &our_name);
    let encoded = protocol::encode(&handshake)?;

    let mut stream = conn.open_bi_stream().await?;
    stream.send_framed(&encoded).await?;

    // Wait for handshake ack with timeout
    let recv_future = stream.recv_framed();
    let response = match tokio::time::timeout(Duration::from_secs(5), recv_future).await {
        Ok(Ok(data)) => data,
        Ok(Err(e)) => {
            return Err(NetworkError::ConnectionFailed(format!(
                "握手失败: {}", e
            )));
        }
        Err(_) => {
            return Err(NetworkError::ConnectionFailed(
                "握手超时".to_string(),
            ));
        }
    };

    // Parse handshake ack to get device info
    let ack = protocol::decode(&response)?;
    let (device_id, device_name) = match ack {
        protocol::Message::HandshakeAck { device_id, name, accepted, reason, .. } => {
            if !accepted {
                return Err(NetworkError::ConnectionFailed(format!(
                    "对方拒绝连接: {}",
                    reason.unwrap_or_else(|| "未知原因".to_string())
                )));
            }
            (device_id, name)
        }
        _ => {
            return Err(NetworkError::ConnectionFailed(
                "无效的握手响应".to_string(),
            ));
        }
    };

    // Connection and handshake successful, add device
    let device = DiscoveredDevice {
        id: device_id,
        name: device_name,
        ip,
        port,
        status: DeviceStatus::Online,
        last_seen: now_ms(),
        is_sharing: false,
    };

    add_device(device.clone());
    log::info!("Manual device added and verified: {} ({})", device.name, device.ip);

    // Start listening for incoming messages on this connection
    // This is important so we can receive sharing status updates from the peer
    let conn_clone = conn.clone();
    tokio::spawn(async move {
        crate::handle_incoming_connection(conn_clone).await;
    });
    Ok(device)
}

/// Shutdown mDNS service
pub fn shutdown() {
    if let Some(daemon) = MDNS_DAEMON.as_ref() {
        let _ = daemon.shutdown();
    }
}

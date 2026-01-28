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
    once_cell::sync::Lazy::new(|| ServiceDaemon::new().ok());

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

    let service_info = ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name,
        &format!("{}.local.", hostname),
        "",
        SERVICE_PORT,
        properties,
    )
    .map_err(|e| NetworkError::DiscoveryError(format!("Failed to create service info: {}", e)))?;

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

    // Get first IPv4 address
    let ip = info
        .addresses
        .iter()
        .find_map(|scoped_ip| {
            let ip_addr = scoped_ip.to_ip_addr();
            if ip_addr.is_ipv4() {
                Some(ip_addr.to_string())
            } else {
                None
            }
        })?;

    let port = info.port;

    Some(DiscoveredDevice {
        id,
        name,
        ip,
        port,
        status: DeviceStatus::Online,
        last_seen: now_ms(),
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

/// Update device status
pub fn update_device_status(id: &str, status: DeviceStatus) {
    let mut devices = DEVICES.write();
    if let Some(device) = devices.get_mut(id) {
        device.status = status;
        device.last_seen = now_ms();
    }
}

/// Manually add a device by IP address
pub async fn add_manual_device(ip: String, port: u16) -> Result<DiscoveredDevice, NetworkError> {
    let device = DiscoveredDevice {
        id: format!("manual-{}", ip.replace('.', "-")),
        name: format!("Device at {}", ip),
        ip,
        port,
        status: DeviceStatus::Online,
        last_seen: now_ms(),
    };

    add_device(device.clone());
    Ok(device)
}

/// Shutdown mDNS service
pub fn shutdown() {
    if let Some(daemon) = MDNS_DAEMON.as_ref() {
        let _ = daemon.shutdown();
    }
}

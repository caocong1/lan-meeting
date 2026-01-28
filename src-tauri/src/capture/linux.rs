// Linux screen capture
// - Wayland: Uses PipeWire via xdg-desktop-portal (requires user interaction for permission)
// - X11: Uses XGetImage/XShmGetImage for efficient capture

use super::{CaptureError, CapturedFrame, Display, FrameFormat, ScreenCapture};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Linux screen capture implementation
pub struct LinuxCapture {
    is_capturing: AtomicBool,
    current_display: RwLock<Option<u32>>,
    cached_displays: RwLock<Vec<Display>>,
    backend: LinuxBackend,
    #[cfg(feature = "x11")]
    x11_state: RwLock<Option<X11State>>,
}

#[derive(Debug, Clone, Copy)]
enum LinuxBackend {
    PipeWire,
    X11,
    None,
}

#[cfg(feature = "x11")]
struct X11State {
    conn: x11rb::rust_connection::RustConnection,
    screen_num: usize,
    root: u32,
    width: u16,
    height: u16,
}

// Safe because we use proper synchronization
unsafe impl Send for LinuxCapture {}
unsafe impl Sync for LinuxCapture {}

impl LinuxCapture {
    pub fn new() -> Result<Self, CaptureError> {
        // Detect display server
        let backend = Self::detect_backend();

        log::info!("Linux capture backend: {:?}", backend);

        Ok(Self {
            is_capturing: AtomicBool::new(false),
            current_display: RwLock::new(None),
            cached_displays: RwLock::new(Vec::new()),
            backend,
            #[cfg(feature = "x11")]
            x11_state: RwLock::new(None),
        })
    }

    fn detect_backend() -> LinuxBackend {
        // Check for Wayland
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            #[cfg(feature = "pipewire")]
            {
                log::info!("Wayland detected, using PipeWire backend");
                return LinuxBackend::PipeWire;
            }
            #[cfg(not(feature = "pipewire"))]
            {
                log::warn!("Wayland detected but PipeWire feature not enabled");
            }
        }

        // Check for X11
        if std::env::var("DISPLAY").is_ok() {
            #[cfg(feature = "x11")]
            {
                log::info!("X11 detected, using X11 backend");
                return LinuxBackend::X11;
            }
            #[cfg(not(feature = "x11"))]
            {
                log::warn!("X11 detected but x11 feature not enabled");
            }
        }

        log::warn!("No supported display server detected");
        LinuxBackend::None
    }

    #[cfg(feature = "x11")]
    fn init_x11(&self) -> Result<(), CaptureError> {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::ConnectionExt;

        let (conn, screen_num) = x11rb::rust_connection::RustConnection::connect(None)
            .map_err(|e| CaptureError::InitError(format!("Failed to connect to X11: {}", e)))?;

        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let width = screen.width_in_pixels;
        let height = screen.height_in_pixels;

        log::info!("X11 connected: screen {}x{}", width, height);

        *self.x11_state.write() = Some(X11State {
            conn,
            screen_num,
            root,
            width,
            height,
        });

        Ok(())
    }

    #[cfg(feature = "x11")]
    fn capture_x11(&self) -> Result<CapturedFrame, CaptureError> {
        use x11rb::protocol::xproto::ConnectionExt;

        let state_guard = self.x11_state.read();
        let state = state_guard
            .as_ref()
            .ok_or_else(|| CaptureError::CaptureError("X11 not initialized".to_string()))?;

        // Get the image from the root window
        let reply = state
            .conn
            .get_image(
                x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
                state.root,
                0,
                0,
                state.width,
                state.height,
                !0, // all planes
            )
            .map_err(|e| CaptureError::CaptureError(format!("get_image failed: {}", e)))?
            .reply()
            .map_err(|e| CaptureError::CaptureError(format!("get_image reply failed: {}", e)))?;

        let width = state.width as u32;
        let height = state.height as u32;

        // X11 returns BGRA data (32-bit depth)
        let frame_data = reply.data;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Ok(CapturedFrame {
            width,
            height,
            timestamp,
            data: frame_data,
            format: FrameFormat::Bgra,
        })
    }

    /// Enumerate displays on X11
    #[cfg(feature = "x11")]
    fn enumerate_x11_displays(&self) -> Result<Vec<Display>, CaptureError> {
        use x11rb::connection::Connection;

        let (conn, screen_num) = x11rb::rust_connection::RustConnection::connect(None)
            .map_err(|e| CaptureError::InitError(format!("Failed to connect to X11: {}", e)))?;

        let mut displays = Vec::new();

        for (i, screen) in conn.setup().roots.iter().enumerate() {
            displays.push(Display {
                id: i as u32,
                name: if i == screen_num {
                    "主显示器".to_string()
                } else {
                    format!("显示器 {}", i + 1)
                },
                width: screen.width_in_pixels as u32,
                height: screen.height_in_pixels as u32,
                scale_factor: 1.0,
                primary: i == screen_num,
            });
        }

        // Sort so primary is first
        displays.sort_by(|a, b| b.primary.cmp(&a.primary));

        Ok(displays)
    }

    /// Get a default display list when no backend is available
    fn get_default_displays() -> Vec<Display> {
        vec![Display {
            id: 0,
            name: "默认显示器".to_string(),
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
            primary: true,
        }]
    }
}

impl ScreenCapture for LinuxCapture {
    fn get_displays(&self) -> Result<Vec<Display>, CaptureError> {
        let displays = match self.backend {
            #[cfg(feature = "x11")]
            LinuxBackend::X11 => self.enumerate_x11_displays()?,
            #[cfg(feature = "pipewire")]
            LinuxBackend::PipeWire => {
                // PipeWire screen selection happens through xdg-desktop-portal dialog
                // Return a placeholder - actual selection is done when starting capture
                log::info!("PipeWire: Display selection handled by portal");
                vec![Display {
                    id: 0,
                    name: "通过系统对话框选择".to_string(),
                    width: 0,
                    height: 0,
                    scale_factor: 1.0,
                    primary: true,
                }]
            }
            _ => Self::get_default_displays(),
        };

        *self.cached_displays.write() = displays.clone();
        Ok(displays)
    }

    fn start(&mut self, display_id: u32) -> Result<(), CaptureError> {
        // Stop any existing capture
        self.stop()?;

        match self.backend {
            #[cfg(feature = "x11")]
            LinuxBackend::X11 => {
                self.init_x11()?;
            }
            #[cfg(feature = "pipewire")]
            LinuxBackend::PipeWire => {
                // PipeWire capture requires:
                // 1. Connect to org.freedesktop.portal.ScreenCast via D-Bus
                // 2. CreateSession -> SelectSources -> Start
                // 3. Get PipeWire fd from portal
                // 4. Connect to PipeWire stream
                //
                // This is complex and requires async D-Bus communication.
                // For now, return an error indicating portal integration is needed.
                return Err(CaptureError::InitError(
                    "PipeWire screen capture requires xdg-desktop-portal integration. \
                     Please use X11 backend or run with XDG_SESSION_TYPE=x11"
                        .to_string(),
                ));
            }
            LinuxBackend::None => {
                return Err(CaptureError::InitError(
                    "No supported display server backend available. \
                     Enable 'x11' or 'pipewire' feature and ensure DISPLAY or WAYLAND_DISPLAY is set"
                        .to_string(),
                ));
            }
            #[allow(unreachable_patterns)]
            _ => {
                return Err(CaptureError::InitError(
                    "Display server backend not available".to_string(),
                ));
            }
        }

        *self.current_display.write() = Some(display_id);
        self.is_capturing.store(true, Ordering::SeqCst);

        log::info!(
            "Started Linux screen capture for display {} ({:?})",
            display_id,
            self.backend
        );
        Ok(())
    }

    fn stop(&mut self) -> Result<(), CaptureError> {
        self.is_capturing.store(false, Ordering::SeqCst);
        *self.current_display.write() = None;

        #[cfg(feature = "x11")]
        {
            *self.x11_state.write() = None;
        }

        log::info!("Stopped Linux screen capture");
        Ok(())
    }

    fn capture_frame(&mut self) -> Result<CapturedFrame, CaptureError> {
        if !self.is_capturing.load(Ordering::SeqCst) {
            return Err(CaptureError::CaptureError("Not capturing".to_string()));
        }

        match self.backend {
            #[cfg(feature = "x11")]
            LinuxBackend::X11 => self.capture_x11(),
            _ => Err(CaptureError::CaptureError(
                "Backend does not support frame capture".to_string(),
            )),
        }
    }

    fn is_capturing(&self) -> bool {
        self.is_capturing.load(Ordering::SeqCst)
    }
}

impl Default for LinuxCapture {
    fn default() -> Self {
        Self::new().expect("Failed to create LinuxCapture")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_backend() {
        let backend = LinuxCapture::detect_backend();
        // Just ensure it doesn't panic
        println!("Detected backend: {:?}", backend);
    }
}

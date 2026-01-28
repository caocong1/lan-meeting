// Screen capture module
// Platform-specific implementations for high-performance screen capture

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("Failed to initialize capture: {0}")]
    InitError(String),
    #[error("Capture permission denied")]
    PermissionDenied,
    #[error("Display not found: {0}")]
    DisplayNotFound(u32),
    #[error("Capture failed: {0}")]
    CaptureError(String),
}

/// Display information
#[derive(Debug, Clone)]
pub struct Display {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
    pub primary: bool,
}

/// Captured frame data
#[derive(Debug)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub timestamp: u64,
    pub data: Vec<u8>,
    pub format: FrameFormat,
}

#[derive(Debug, Clone, Copy)]
pub enum FrameFormat {
    Bgra,
    Rgba,
    Nv12,
}

/// Screen capture trait - implemented per platform
pub trait ScreenCapture: Send + Sync {
    /// Get list of available displays
    fn get_displays(&self) -> Result<Vec<Display>, CaptureError>;

    /// Start capturing a specific display
    fn start(&mut self, display_id: u32) -> Result<(), CaptureError>;

    /// Stop capturing
    fn stop(&mut self) -> Result<(), CaptureError>;

    /// Get the next frame (blocking)
    fn capture_frame(&mut self) -> Result<CapturedFrame, CaptureError>;

    /// Check if currently capturing
    fn is_capturing(&self) -> bool;
}

/// Create platform-specific screen capture instance
pub fn create_capture() -> Result<Box<dyn ScreenCapture>, CaptureError> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::MacOSCapture::new()?))
    }

    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(windows::WindowsCapture::new()?))
    }

    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::LinuxCapture::new()?))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(CaptureError::InitError("Unsupported platform".to_string()))
    }
}

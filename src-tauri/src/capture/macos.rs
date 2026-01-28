// macOS screen capture using CoreGraphics
// Uses CGDisplayCreateImage for reliable cross-version compatibility
// Future: Add ScreenCaptureKit streaming for better performance (macOS 12.3+)

use super::{CaptureError, CapturedFrame, Display, FrameFormat, ScreenCapture};
use core_graphics::display::{CGDirectDisplayID, CGDisplay, CGMainDisplayID};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// External C functions for screen capture
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut CGDirectDisplayID,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayCreateImage(display: CGDirectDisplayID)
        -> *mut core_foundation::base::CFTypeRef;
}

/// macOS screen capture implementation using CoreGraphics
pub struct MacOSCapture {
    is_capturing: AtomicBool,
    current_display: RwLock<Option<u32>>,
    cached_displays: RwLock<Vec<Display>>,
}

// Manual Send + Sync implementation since we only use thread-safe primitives
unsafe impl Send for MacOSCapture {}
unsafe impl Sync for MacOSCapture {}

impl MacOSCapture {
    pub fn new() -> Result<Self, CaptureError> {
        // Check for screen recording permission
        if !Self::has_permission() {
            log::warn!("Screen recording permission not granted, will request on first capture");
        }

        Ok(Self {
            is_capturing: AtomicBool::new(false),
            current_display: RwLock::new(None),
            cached_displays: RwLock::new(Vec::new()),
        })
    }

    /// Check if screen recording permission is granted
    pub fn has_permission() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    /// Request screen recording permission (shows system dialog)
    pub fn request_permission() -> bool {
        unsafe { CGRequestScreenCaptureAccess() }
    }

    /// Enumerate all active displays using CoreGraphics
    fn enumerate_displays() -> Result<Vec<Display>, CaptureError> {
        let mut displays = Vec::new();

        const MAX_DISPLAYS: u32 = 16;
        let mut display_ids: [CGDirectDisplayID; 16] = [0; 16];
        let mut display_count: u32 = 0;

        unsafe {
            let result = CGGetActiveDisplayList(
                MAX_DISPLAYS,
                display_ids.as_mut_ptr(),
                &mut display_count,
            );

            if result != 0 {
                return Err(CaptureError::InitError(format!(
                    "CGGetActiveDisplayList failed with code: {}",
                    result
                )));
            }
        }

        let main_display_id = unsafe { CGMainDisplayID() };

        for i in 0..display_count as usize {
            let display_id = display_ids[i];
            let cg_display = CGDisplay::new(display_id);

            let width = cg_display.pixels_wide() as u32;
            let height = cg_display.pixels_high() as u32;

            // Calculate scale factor for Retina displays
            let bounds = cg_display.bounds();
            let scale_factor = if bounds.size.width > 0.0 {
                width as f32 / bounds.size.width as f32
            } else {
                1.0
            };

            let is_primary = display_id == main_display_id;

            let name = if is_primary {
                "主显示器".to_string()
            } else {
                format!("显示器 {}", i + 1)
            };

            displays.push(Display {
                id: display_id,
                name,
                width,
                height,
                scale_factor,
                primary: is_primary,
            });
        }

        // Sort so primary display is first
        displays.sort_by(|a, b| b.primary.cmp(&a.primary));

        Ok(displays)
    }

    /// Capture a single frame using CGDisplayCreateImage
    fn capture_display(display_id: u32) -> Result<CapturedFrame, CaptureError> {
        // Type aliases for C types
        type CGImageRef = *const std::ffi::c_void;
        type CGDataProviderRef = *const std::ffi::c_void;
        type CFDataRef = *const std::ffi::c_void;

        // Additional FFI declarations for CGImage operations
        unsafe extern "C" {
            fn CGImageGetWidth(image: CGImageRef) -> usize;
            fn CGImageGetHeight(image: CGImageRef) -> usize;
            fn CGImageGetBitsPerPixel(image: CGImageRef) -> usize;
            fn CGImageGetDataProvider(image: CGImageRef) -> CGDataProviderRef;
            fn CGDataProviderCopyData(provider: CGDataProviderRef) -> CFDataRef;
            fn CFDataGetLength(data: CFDataRef) -> isize;
            fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
            fn CFRelease(cf: *const std::ffi::c_void);
        }

        unsafe {
            let image_ref: CGImageRef = CGDisplayCreateImage(display_id) as CGImageRef;
            if image_ref.is_null() {
                return Err(CaptureError::CaptureError(
                    "CGDisplayCreateImage returned null - check screen recording permission"
                        .to_string(),
                ));
            }

            let width = CGImageGetWidth(image_ref) as u32;
            let height = CGImageGetHeight(image_ref) as u32;
            let bits_per_pixel = CGImageGetBitsPerPixel(image_ref);

            // Get pixel data from the image
            let data_provider = CGImageGetDataProvider(image_ref);
            if data_provider.is_null() {
                CFRelease(image_ref);
                return Err(CaptureError::CaptureError(
                    "Failed to get data provider".to_string(),
                ));
            }

            let cf_data = CGDataProviderCopyData(data_provider);
            if cf_data.is_null() {
                CFRelease(image_ref);
                return Err(CaptureError::CaptureError(
                    "Failed to copy image data".to_string(),
                ));
            }

            let data_len = CFDataGetLength(cf_data) as usize;
            let data_ptr = CFDataGetBytePtr(cf_data);

            let frame_data = if !data_ptr.is_null() && data_len > 0 {
                std::slice::from_raw_parts(data_ptr, data_len).to_vec()
            } else {
                CFRelease(cf_data);
                CFRelease(image_ref);
                return Err(CaptureError::CaptureError(
                    "Failed to get image bytes".to_string(),
                ));
            };

            // Release CoreFoundation objects
            CFRelease(cf_data);
            CFRelease(image_ref);

            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            // Determine pixel format based on bits per pixel
            let format = if bits_per_pixel == 32 {
                // macOS typically uses BGRA
                FrameFormat::Bgra
            } else {
                FrameFormat::Rgba
            };

            Ok(CapturedFrame {
                width,
                height,
                timestamp,
                data: frame_data,
                format,
            })
        }
    }
}

impl ScreenCapture for MacOSCapture {
    fn get_displays(&self) -> Result<Vec<Display>, CaptureError> {
        let displays = Self::enumerate_displays()?;
        *self.cached_displays.write() = displays.clone();
        Ok(displays)
    }

    fn start(&mut self, display_id: u32) -> Result<(), CaptureError> {
        // Check and request permission if needed
        if !Self::has_permission() {
            log::info!("Requesting screen recording permission...");
            Self::request_permission();

            // Give system time to show and process permission dialog
            std::thread::sleep(std::time::Duration::from_millis(500));

            if !Self::has_permission() {
                return Err(CaptureError::PermissionDenied);
            }
        }

        // Verify display exists
        let displays = Self::enumerate_displays()?;
        if !displays.iter().any(|d| d.id == display_id) {
            return Err(CaptureError::DisplayNotFound(display_id));
        }

        // Stop any existing capture
        self.stop()?;

        // Set the current display and mark as capturing
        *self.current_display.write() = Some(display_id);
        self.is_capturing.store(true, Ordering::SeqCst);

        log::info!("Started macOS screen capture for display {}", display_id);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), CaptureError> {
        *self.current_display.write() = None;
        self.is_capturing.store(false, Ordering::SeqCst);
        log::info!("Stopped macOS screen capture");
        Ok(())
    }

    fn capture_frame(&mut self) -> Result<CapturedFrame, CaptureError> {
        if !self.is_capturing.load(Ordering::SeqCst) {
            return Err(CaptureError::CaptureError("Not capturing".to_string()));
        }

        let display_id = self
            .current_display
            .read()
            .ok_or_else(|| CaptureError::CaptureError("No display selected".to_string()))?;

        Self::capture_display(display_id)
    }

    fn is_capturing(&self) -> bool {
        self.is_capturing.load(Ordering::SeqCst)
    }
}

impl Default for MacOSCapture {
    fn default() -> Self {
        Self::new().expect("Failed to create MacOSCapture")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_displays() {
        let result = MacOSCapture::enumerate_displays();
        assert!(result.is_ok());
        let displays = result.unwrap();
        assert!(!displays.is_empty(), "Should find at least one display");

        // First display should be primary
        assert!(displays[0].primary);
    }

    #[test]
    fn test_permission_check() {
        // This just tests that the function doesn't panic
        let _has_perm = MacOSCapture::has_permission();
    }
}

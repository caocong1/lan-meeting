// Windows screen capture using DXGI Desktop Duplication API
// High-performance GPU-accelerated screen capture for Windows 8+

use super::{CaptureError, CapturedFrame, Display, FrameFormat, ScreenCapture};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::{
    core::Interface,
    Win32::Foundation::HMODULE,
    Win32::Graphics::Direct3D::*,
    Win32::Graphics::Direct3D11::*,
    Win32::Graphics::Dxgi::Common::*,
    Win32::Graphics::Dxgi::*,
};

/// Windows screen capture implementation using DXGI Desktop Duplication
pub struct WindowsCapture {
    is_capturing: AtomicBool,
    current_display: RwLock<Option<u32>>,
    cached_displays: RwLock<Vec<Display>>,
    // DXGI resources (created on start)
    device: RwLock<Option<ID3D11Device>>,
    context: RwLock<Option<ID3D11DeviceContext>>,
    duplication: RwLock<Option<IDXGIOutputDuplication>>,
    staging_texture: RwLock<Option<ID3D11Texture2D>>,
    output_desc: RwLock<Option<DXGI_OUTPUT_DESC>>,
}

// Send + Sync is safe because we use proper synchronization
unsafe impl Send for WindowsCapture {}
unsafe impl Sync for WindowsCapture {}

impl WindowsCapture {
    pub fn new() -> Result<Self, CaptureError> {
        Ok(Self {
            is_capturing: AtomicBool::new(false),
            current_display: RwLock::new(None),
            cached_displays: RwLock::new(Vec::new()),
            device: RwLock::new(None),
            context: RwLock::new(None),
            duplication: RwLock::new(None),
            staging_texture: RwLock::new(None),
            output_desc: RwLock::new(None),
        })
    }

    /// Enumerate all displays using DXGI
    fn enumerate_displays() -> Result<Vec<Display>, CaptureError> {
        let mut displays = Vec::new();

        unsafe {
            // Create DXGI factory
            let factory: IDXGIFactory1 = CreateDXGIFactory1()
                .map_err(|e| CaptureError::InitError(format!("CreateDXGIFactory1 failed: {}", e)))?;

            // Enumerate adapters
            let mut adapter_idx = 0u32;
            while let Ok(adapter) = factory.EnumAdapters1(adapter_idx) {
                // Enumerate outputs for this adapter
                let mut output_idx = 0u32;
                while let Ok(output) = adapter.EnumOutputs(output_idx) {
                    let desc = output
                        .GetDesc()
                        .map_err(|e| CaptureError::InitError(format!("GetDesc failed: {}", e)))?;

                    let rect = desc.DesktopCoordinates;
                    let width = (rect.right - rect.left) as u32;
                    let height = (rect.bottom - rect.top) as u32;

                    // Convert device name from wide string
                    let name_len = desc
                        .DeviceName
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(desc.DeviceName.len());
                    let name = String::from_utf16_lossy(&desc.DeviceName[..name_len]);

                    // Generate unique display ID from adapter and output indices
                    let display_id = (adapter_idx << 16) | output_idx;

                    // Check if this is the primary display
                    let is_primary = rect.left == 0 && rect.top == 0;

                    displays.push(Display {
                        id: display_id,
                        name: if is_primary {
                            "主显示器".to_string()
                        } else {
                            name.trim_end_matches('\0').to_string()
                        },
                        width,
                        height,
                        scale_factor: 1.0, // Windows DPI scaling handled separately
                        primary: is_primary,
                    });

                    output_idx += 1;
                }
                adapter_idx += 1;
            }
        }

        // Sort so primary display is first
        displays.sort_by(|a, b| b.primary.cmp(&a.primary));

        if displays.is_empty() {
            return Err(CaptureError::InitError("No displays found".to_string()));
        }

        Ok(displays)
    }

    /// Initialize DXGI resources for capturing a specific display
    fn init_capture_resources(&self, display_id: u32) -> Result<(), CaptureError> {
        let adapter_idx = (display_id >> 16) as u32;
        let output_idx = (display_id & 0xFFFF) as u32;

        unsafe {
            // Create DXGI factory
            let factory: IDXGIFactory1 = CreateDXGIFactory1()
                .map_err(|e| CaptureError::InitError(format!("CreateDXGIFactory1 failed: {}", e)))?;

            // Get the adapter
            let adapter: IDXGIAdapter1 = factory.EnumAdapters1(adapter_idx).map_err(|_| {
                CaptureError::DisplayNotFound(display_id)
            })?;

            // Create D3D11 device
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;

            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE(std::ptr::null_mut()),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| CaptureError::InitError(format!("D3D11CreateDevice failed: {}", e)))?;

            let device = device.ok_or_else(|| {
                CaptureError::InitError("D3D11CreateDevice returned null device".to_string())
            })?;
            let context = context.ok_or_else(|| {
                CaptureError::InitError("D3D11CreateDevice returned null context".to_string())
            })?;

            // Get the output
            let output: IDXGIOutput = adapter.EnumOutputs(output_idx).map_err(|_| {
                CaptureError::DisplayNotFound(display_id)
            })?;

            // Get output description
            let output_desc = output.GetDesc().map_err(|e| {
                CaptureError::InitError(format!("GetDesc failed: {}", e))
            })?;

            // Query for IDXGIOutput1 (needed for DuplicateOutput)
            let output1: IDXGIOutput1 = output.cast().map_err(|e| {
                CaptureError::InitError(format!(
                    "Failed to get IDXGIOutput1 - Desktop Duplication requires Windows 8+: {}",
                    e
                ))
            })?;

            // Create output duplication
            let duplication = output1.DuplicateOutput(&device).map_err(|e| {
                CaptureError::InitError(format!(
                    "DuplicateOutput failed - another app may be capturing: {}",
                    e
                ))
            })?;

            // Create staging texture for CPU read
            let rect = output_desc.DesktopCoordinates;
            let width = (rect.right - rect.left) as u32;
            let height = (rect.bottom - rect.top) as u32;

            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: D3D11_BIND_FLAG(0).0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: D3D11_RESOURCE_MISC_FLAG(0).0 as u32,
            };

            let mut staging_texture: Option<ID3D11Texture2D> = None;
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging_texture))
                .map_err(|e| {
                    CaptureError::InitError(format!("CreateTexture2D failed: {}", e))
                })?;

            let staging_texture = staging_texture.ok_or_else(|| {
                CaptureError::InitError("CreateTexture2D returned null".to_string())
            })?;

            // Store resources
            *self.device.write() = Some(device);
            *self.context.write() = Some(context);
            *self.duplication.write() = Some(duplication);
            *self.staging_texture.write() = Some(staging_texture);
            *self.output_desc.write() = Some(output_desc);

            log::info!(
                "DXGI capture initialized for display {} ({}x{})",
                display_id,
                width,
                height
            );
        }

        Ok(())
    }

    /// Release DXGI resources
    fn release_resources(&self) {
        *self.duplication.write() = None;
        *self.staging_texture.write() = None;
        *self.context.write() = None;
        *self.device.write() = None;
        *self.output_desc.write() = None;
    }
}

impl ScreenCapture for WindowsCapture {
    fn get_displays(&self) -> Result<Vec<Display>, CaptureError> {
        let displays = Self::enumerate_displays()?;
        *self.cached_displays.write() = displays.clone();
        Ok(displays)
    }

    fn start(&mut self, display_id: u32) -> Result<(), CaptureError> {
        // Stop any existing capture
        self.stop()?;

        // Initialize DXGI resources
        self.init_capture_resources(display_id)?;

        // Set the current display and mark as capturing
        *self.current_display.write() = Some(display_id);
        self.is_capturing.store(true, Ordering::SeqCst);

        log::info!("Started Windows screen capture for display {}", display_id);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), CaptureError> {
        self.is_capturing.store(false, Ordering::SeqCst);
        *self.current_display.write() = None;
        self.release_resources();
        log::info!("Stopped Windows screen capture");
        Ok(())
    }

    fn capture_frame(&mut self) -> Result<CapturedFrame, CaptureError> {
        if !self.is_capturing.load(Ordering::SeqCst) {
            return Err(CaptureError::CaptureError("Not capturing".to_string()));
        }

        let duplication_guard = self.duplication.read();
        let duplication = duplication_guard
            .as_ref()
            .ok_or_else(|| CaptureError::CaptureError("Duplication not initialized".to_string()))?;

        let context_guard = self.context.read();
        let context = context_guard
            .as_ref()
            .ok_or_else(|| CaptureError::CaptureError("Context not initialized".to_string()))?;

        let staging_guard = self.staging_texture.read();
        let staging_texture = staging_guard
            .as_ref()
            .ok_or_else(|| CaptureError::CaptureError("Staging texture not initialized".to_string()))?;

        let output_desc_guard = self.output_desc.read();
        let output_desc = output_desc_guard
            .as_ref()
            .ok_or_else(|| CaptureError::CaptureError("Output desc not initialized".to_string()))?;

        let rect = output_desc.DesktopCoordinates;
        let width = (rect.right - rect.left) as u32;
        let height = (rect.bottom - rect.top) as u32;

        unsafe {
            // Acquire next frame with timeout
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut desktop_resource: Option<IDXGIResource> = None;

            let result = duplication.AcquireNextFrame(100, &mut frame_info, &mut desktop_resource);

            if result.is_err() {
                // Handle timeout or other errors
                let err = result.unwrap_err();
                if err.code().0 as u32 == 0x887A0027 {
                    // DXGI_ERROR_WAIT_TIMEOUT
                    return Err(CaptureError::CaptureError("Frame timeout".to_string()));
                }
                return Err(CaptureError::CaptureError(format!(
                    "AcquireNextFrame failed: {}",
                    err
                )));
            }

            let desktop_resource = desktop_resource.ok_or_else(|| {
                CaptureError::CaptureError("AcquireNextFrame returned null resource".to_string())
            })?;

            // Get the texture from the resource
            let desktop_texture: ID3D11Texture2D = desktop_resource.cast().map_err(|e| {
                CaptureError::CaptureError(format!("Failed to cast to ID3D11Texture2D: {}", e))
            })?;

            // Copy to staging texture
            context.CopyResource(staging_texture, &desktop_texture);

            // Release the frame
            duplication.ReleaseFrame().map_err(|e| {
                CaptureError::CaptureError(format!("ReleaseFrame failed: {}", e))
            })?;

            // Map staging texture to read pixels
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(
                    staging_texture,
                    0,
                    D3D11_MAP_READ,
                    0,
                    Some(&mut mapped),
                )
                .map_err(|e| CaptureError::CaptureError(format!("Map failed: {}", e)))?;

            // Copy pixel data
            let row_pitch = mapped.RowPitch as usize;
            let data_size = (width * height * 4) as usize;
            let mut frame_data = Vec::with_capacity(data_size);

            let src_ptr = mapped.pData as *const u8;
            for y in 0..height as usize {
                let row_start = src_ptr.add(y * row_pitch);
                let row_slice = std::slice::from_raw_parts(row_start, (width * 4) as usize);
                frame_data.extend_from_slice(row_slice);
            }

            // Unmap
            context.Unmap(staging_texture, 0);

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
    }

    fn is_capturing(&self) -> bool {
        self.is_capturing.load(Ordering::SeqCst)
    }
}

impl Default for WindowsCapture {
    fn default() -> Self {
        Self::new().expect("Failed to create WindowsCapture")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_displays() {
        let result = WindowsCapture::enumerate_displays();
        // This might fail on CI without display
        if let Ok(displays) = result {
            assert!(!displays.is_empty(), "Should find at least one display");
            // First display should be primary
            assert!(displays[0].primary);
        }
    }
}

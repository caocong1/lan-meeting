// Video decoder module
// Hardware decoding with software fallback
//
// Decoder priority:
// 1. Vulkan Video (cross-platform hardware acceleration via vk-video)
// 2. Platform-specific hardware (VideoToolbox/DXVA/VAAPI)
// 3. OpenH264 software decoder

pub mod software;
pub mod vulkan;

#[cfg(target_os = "macos")]
pub mod videotoolbox;

#[cfg(target_os = "windows")]
pub mod dxva;

#[cfg(target_os = "linux")]
pub mod vaapi;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DecoderError {
    #[error("Failed to initialize decoder: {0}")]
    InitError(String),
    #[error("Decoding failed: {0}")]
    DecodeError(String),
    #[error("Hardware decoder not available")]
    HardwareNotAvailable,
    #[error("Invalid data: {0}")]
    InvalidData(String),
}

/// Decoder configuration
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    pub width: u32,
    pub height: u32,
    /// Output format: BGRA for rendering, YUV420 for zero-copy
    pub output_format: OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    BGRA,   // For direct rendering
    YUV420, // For GPU YUV->RGB conversion
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            output_format: OutputFormat::BGRA,
        }
    }
}

/// Decoded frame data - either CPU memory or GPU texture
#[derive(Debug)]
pub enum DecodedFrameData {
    /// Frame data in CPU memory (BGRA or YUV420)
    Cpu {
        data: Vec<u8>,
        /// For YUV420: strides for Y, U, V planes
        strides: Option<[usize; 3]>,
    },
    /// Frame decoded directly to GPU texture (zero-copy path)
    /// Note: Requires vk-video to update to wgpu 28 for full support
    /// For now, this variant is reserved for future use
    #[allow(dead_code)]
    Gpu {
        /// Placeholder for future wgpu::Texture integration
        texture_id: u64,
    },
}

/// Decoded frame ready for rendering
#[derive(Debug)]
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub timestamp: u64,
    pub format: OutputFormat,
    pub data: DecodedFrameData,
}

impl DecodedFrame {
    /// Create a BGRA frame in CPU memory
    pub fn bgra(width: u32, height: u32, timestamp: u64, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            timestamp,
            format: OutputFormat::BGRA,
            data: DecodedFrameData::Cpu { data, strides: None },
        }
    }

    /// Create a YUV420 frame in CPU memory
    pub fn yuv420(
        width: u32,
        height: u32,
        timestamp: u64,
        data: Vec<u8>,
        strides: [usize; 3],
    ) -> Self {
        Self {
            width,
            height,
            timestamp,
            format: OutputFormat::YUV420,
            data: DecodedFrameData::Cpu {
                data,
                strides: Some(strides),
            },
        }
    }

    /// Check if frame is in CPU memory
    pub fn is_cpu(&self) -> bool {
        matches!(self.data, DecodedFrameData::Cpu { .. })
    }

    /// Get CPU data if available
    pub fn cpu_data(&self) -> Option<&[u8]> {
        match &self.data {
            DecodedFrameData::Cpu { data, .. } => Some(data),
            DecodedFrameData::Gpu { .. } => None,
        }
    }

    /// Get YUV strides if available
    pub fn strides(&self) -> Option<[usize; 3]> {
        match &self.data {
            DecodedFrameData::Cpu { strides, .. } => *strides,
            DecodedFrameData::Gpu { .. } => None,
        }
    }
}

/// Video decoder trait
pub trait VideoDecoder: Send + Sync {
    /// Initialize the decoder
    fn init(&mut self, config: DecoderConfig) -> Result<(), DecoderError>;

    /// Decode H.264 NAL units
    fn decode(&mut self, data: &[u8], timestamp: u64) -> Result<Option<DecodedFrame>, DecoderError>;

    /// Flush any buffered frames
    fn flush(&mut self) -> Result<Vec<DecodedFrame>, DecoderError>;

    /// Get decoder info
    fn info(&self) -> &str;
}

/// Create the best available decoder for this platform
pub fn create_decoder() -> Result<Box<dyn VideoDecoder>, DecoderError> {
    // Try Vulkan Video hardware decoder first (cross-platform)
    match vulkan::VulkanDecoder::new() {
        Ok(dec) => {
            log::info!("Using Vulkan Video hardware decoder");
            return Ok(Box::new(dec));
        }
        Err(e) => log::warn!("Vulkan Video decoder not available: {}", e),
    }

    // Try platform-specific hardware decoders
    #[cfg(target_os = "macos")]
    {
        match videotoolbox::VideoToolboxDecoder::new() {
            Ok(dec) => {
                log::info!("Using VideoToolbox hardware decoder");
                return Ok(Box::new(dec));
            }
            Err(e) => log::warn!("VideoToolbox decoder not available: {}", e),
        }
    }

    #[cfg(target_os = "windows")]
    {
        match dxva::DxvaDecoder::new() {
            Ok(dec) => {
                log::info!("Using DXVA2 hardware decoder");
                return Ok(Box::new(dec));
            }
            Err(e) => log::warn!("DXVA2 decoder not available: {}", e),
        }
    }

    #[cfg(target_os = "linux")]
    {
        match vaapi::VaapiDecoder::new() {
            Ok(dec) => {
                log::info!("Using VAAPI hardware decoder");
                return Ok(Box::new(dec));
            }
            Err(e) => log::warn!("VAAPI decoder not available: {}", e),
        }
    }

    // Fall back to software decoder
    log::info!("Using OpenH264 software decoder");
    Ok(Box::new(software::SoftwareDecoder::new()?))
}

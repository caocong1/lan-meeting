// Video encoder module
// Hardware encoding with software fallback

#[cfg(target_os = "macos")]
pub mod videotoolbox;

#[cfg(target_os = "windows")]
pub mod nvenc;

#[cfg(target_os = "linux")]
pub mod vaapi;

pub mod software;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum EncoderError {
    #[error("Failed to initialize encoder: {0}")]
    InitError(String),
    #[error("Encoding failed: {0}")]
    EncodeError(String),
    #[error("Hardware encoder not available")]
    HardwareNotAvailable,
}

#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub max_bitrate: u32,
    pub keyframe_interval: u32,
    pub preset: EncoderPreset,
}

#[derive(Debug, Clone, Copy)]
pub enum EncoderPreset {
    UltraFast, // Lowest latency
    Fast,
    Medium,
    Quality, // Best quality
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate: 8_000_000,     // 8 Mbps
            max_bitrate: 15_000_000, // 15 Mbps peak
            keyframe_interval: 60,   // 1 second at 60fps
            preset: EncoderPreset::UltraFast,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameType {
    KeyFrame, // I-frame
    Delta,    // P-frame
}

#[derive(Debug)]
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub timestamp: u64,
    pub frame_type: FrameType,
    pub size: usize,
}

/// Video encoder trait
pub trait VideoEncoder: Send + Sync {
    /// Initialize the encoder
    fn init(&mut self, config: EncoderConfig) -> Result<(), EncoderError>;

    /// Encode a raw frame
    fn encode(&mut self, frame_data: &[u8], timestamp: u64) -> Result<EncodedFrame, EncoderError>;

    /// Request a keyframe on next encode
    fn request_keyframe(&mut self);

    /// Update bitrate dynamically
    fn set_bitrate(&mut self, bitrate: u32) -> Result<(), EncoderError>;

    /// Get encoder info
    fn info(&self) -> &str;
}

/// Create the best available encoder for this platform
pub fn create_encoder() -> Result<Box<dyn VideoEncoder>, EncoderError> {
    // Try hardware encoders first, fall back to software

    #[cfg(target_os = "macos")]
    {
        match videotoolbox::VideoToolboxEncoder::new() {
            Ok(enc) => {
                log::info!("Using VideoToolbox hardware encoder");
                return Ok(Box::new(enc));
            }
            Err(e) => log::warn!("VideoToolbox not available: {}", e),
        }
    }

    #[cfg(target_os = "windows")]
    {
        match nvenc::NvencEncoder::new() {
            Ok(enc) => {
                log::info!("Using NVENC hardware encoder");
                return Ok(Box::new(enc));
            }
            Err(e) => log::warn!("NVENC not available: {}", e),
        }
    }

    #[cfg(target_os = "linux")]
    {
        match vaapi::VaapiEncoder::new() {
            Ok(enc) => {
                log::info!("Using VAAPI hardware encoder");
                return Ok(Box::new(enc));
            }
            Err(e) => log::warn!("VAAPI not available: {}", e),
        }
    }

    // Fall back to software encoder
    log::info!("Using software encoder (x264)");
    Ok(Box::new(software::SoftwareEncoder::new()?))
}

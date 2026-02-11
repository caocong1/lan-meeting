// NVIDIA NVENC hardware encoder
// Requires NVIDIA GPU with NVENC support
//
// TODO: Implement using NVIDIA Video Codec SDK
// - Load nvEncodeAPI64.dll / libnvidia-encode.so
// - NvEncodeAPICreateInstance
// - NvEncOpenEncodeSession
// - NvEncInitializeEncoder with low-latency preset

use super::{EncodedFrame, EncoderConfig, EncoderError, FrameType, VideoEncoder};

pub struct NvencEncoder {
    config: Option<EncoderConfig>,
    force_keyframe: bool,
}

impl NvencEncoder {
    pub fn new() -> Result<Self, EncoderError> {
        // NVENC implementation not yet available
        // Return error to fall back to software encoder
        Err(EncoderError::HardwareNotAvailable)
    }
}

impl VideoEncoder for NvencEncoder {
    fn init(&mut self, config: EncoderConfig) -> Result<(), EncoderError> {
        self.config = Some(config);
        log::info!("NVENC encoder initialized (stub)");
        Ok(())
    }

    fn encode(&mut self, _frame_data: &[u8], timestamp: u64) -> Result<EncodedFrame, EncoderError> {
        let frame_type = if self.force_keyframe {
            self.force_keyframe = false;
            FrameType::KeyFrame
        } else {
            FrameType::Delta
        };

        Ok(EncodedFrame {
            data: vec![],
            timestamp,
            frame_type,
            size: 0,
        })
    }

    fn request_keyframe(&mut self) {
        self.force_keyframe = true;
    }

    fn set_bitrate(&mut self, bitrate: u32) -> Result<(), EncoderError> {
        if let Some(ref mut config) = self.config {
            config.bitrate = bitrate;
        }
        Ok(())
    }

    fn info(&self) -> &str {
        "NVENC (NVIDIA Hardware)"
    }

    fn get_dimensions(&self) -> Option<(u32, u32)> {
        self.config.as_ref().map(|c| (c.width, c.height))
    }
}

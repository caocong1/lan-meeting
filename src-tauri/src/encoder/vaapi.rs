// Linux VAAPI hardware encoder
// Works with Intel, AMD, and some NVIDIA GPUs
//
// TODO: Implement using libva
// - vaGetDisplay, vaInitialize
// - vaCreateConfig with VAProfileH264ConstrainedBaseline
// - vaCreateContext
// - vaBeginPicture, vaRenderPicture, vaEndPicture

use super::{EncodedFrame, EncoderConfig, EncoderError, FrameType, VideoEncoder};

pub struct VaapiEncoder {
    config: Option<EncoderConfig>,
    force_keyframe: bool,
}

impl VaapiEncoder {
    pub fn new() -> Result<Self, EncoderError> {
        // VAAPI implementation not yet available
        // Return error to fall back to software encoder
        Err(EncoderError::HardwareNotAvailable)
    }
}

impl VideoEncoder for VaapiEncoder {
    fn init(&mut self, config: EncoderConfig) -> Result<(), EncoderError> {
        self.config = Some(config);
        log::info!("VAAPI encoder initialized (stub)");
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
        "VAAPI (Linux Hardware)"
    }

    fn get_dimensions(&self) -> Option<(u32, u32)> {
        self.config.as_ref().map(|c| (c.width, c.height))
    }
}

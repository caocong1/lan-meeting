// Linux VAAPI hardware decoder
// Works with Intel, AMD, and some NVIDIA GPUs
//
// TODO: Implement using libva
// - vaGetDisplay, vaInitialize
// - vaCreateConfig with VAProfileH264ConstrainedBaseline
// - vaCreateSurfaces, vaCreateContext
// - vaBeginPicture, vaRenderPicture, vaEndPicture

use super::{DecodedFrame, DecoderConfig, DecoderError, VideoDecoder};

pub struct VaapiDecoder {
    config: Option<DecoderConfig>,
}

impl VaapiDecoder {
    pub fn new() -> Result<Self, DecoderError> {
        // VAAPI implementation not yet available
        // Return error to fall back to software decoder
        Err(DecoderError::HardwareNotAvailable)
    }
}

impl VideoDecoder for VaapiDecoder {
    fn init(&mut self, config: DecoderConfig) -> Result<(), DecoderError> {
        self.config = Some(config);
        log::info!("VAAPI decoder initialized (stub)");
        Ok(())
    }

    fn decode(&mut self, _data: &[u8], timestamp: u64) -> Result<Option<DecodedFrame>, DecoderError> {
        let config = self.config.as_ref().unwrap();
        Ok(Some(DecodedFrame::bgra(
            config.width,
            config.height,
            timestamp,
            vec![],
        )))
    }

    fn flush(&mut self) -> Result<Vec<DecodedFrame>, DecoderError> {
        Ok(vec![])
    }

    fn info(&self) -> &str {
        "VAAPI (Hardware)"
    }
}

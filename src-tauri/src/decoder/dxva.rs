// Windows DXVA2 hardware decoder
// Uses DirectX Video Acceleration for hardware H.264 decoding
//
// TODO: Implement using Media Foundation or DXVA2
// - MFCreateDXGIDeviceManager
// - IMFDXGIDeviceManager::ResetDevice
// - MFCreateVideoSampleFromSurface

use super::{DecodedFrame, DecoderConfig, DecoderError, VideoDecoder};

pub struct DxvaDecoder {
    config: Option<DecoderConfig>,
}

impl DxvaDecoder {
    pub fn new() -> Result<Self, DecoderError> {
        // DXVA2 implementation not yet available
        // Return error to fall back to software decoder
        Err(DecoderError::HardwareNotAvailable)
    }
}

impl VideoDecoder for DxvaDecoder {
    fn init(&mut self, config: DecoderConfig) -> Result<(), DecoderError> {
        self.config = Some(config);
        log::info!("DXVA2 decoder initialized (stub)");
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
        "DXVA2 (Hardware)"
    }
}

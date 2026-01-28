// macOS VideoToolbox hardware decoder
// Uses Apple's hardware H.264 decoder for low-latency decoding
//
// TODO: Implement actual VideoToolbox decoding using:
// - VTDecompressionSessionCreate
// - VTDecompressionSessionDecodeFrame
// - CMVideoFormatDescriptionCreateFromH264ParameterSets

use super::{DecodedFrame, DecoderConfig, DecoderError, VideoDecoder};

pub struct VideoToolboxDecoder {
    config: Option<DecoderConfig>,
}

impl VideoToolboxDecoder {
    pub fn new() -> Result<Self, DecoderError> {
        // VideoToolbox implementation not yet available
        // Return error to fall back to software decoder
        Err(DecoderError::HardwareNotAvailable)
    }
}

impl VideoDecoder for VideoToolboxDecoder {
    fn init(&mut self, config: DecoderConfig) -> Result<(), DecoderError> {
        self.config = Some(config);
        log::info!("VideoToolbox decoder initialized (stub)");
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
        "VideoToolbox (Hardware)"
    }
}

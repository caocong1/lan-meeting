// macOS VideoToolbox hardware encoder
// Uses Apple's hardware H.264/HEVC encoder for low-latency encoding
//
// TODO: Implement actual VideoToolbox encoding using:
// - VTCompressionSessionCreate
// - VTCompressionSessionEncodeFrame
// - CVPixelBufferCreateWithBytes for zero-copy
//
// Key settings for low latency:
// - kVTCompressionPropertyKey_RealTime = true
// - kVTCompressionPropertyKey_AllowFrameReordering = false
// - kVTCompressionPropertyKey_ProfileLevel = H264_Baseline_AutoLevel

use super::{EncodedFrame, EncoderConfig, EncoderError, FrameType, VideoEncoder};

pub struct VideoToolboxEncoder {
    config: Option<EncoderConfig>,
    force_keyframe: bool,
}

impl VideoToolboxEncoder {
    pub fn new() -> Result<Self, EncoderError> {
        // VideoToolbox implementation not yet available
        // Return error to fall back to software encoder
        Err(EncoderError::HardwareNotAvailable)
    }
}

impl VideoEncoder for VideoToolboxEncoder {
    fn init(&mut self, config: EncoderConfig) -> Result<(), EncoderError> {
        self.config = Some(config);
        log::info!("VideoToolbox encoder initialized (stub)");
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
        "VideoToolbox (Hardware)"
    }
}

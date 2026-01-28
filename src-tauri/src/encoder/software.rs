// Software encoder using Cisco OpenH264
// Cross-platform H.264 software encoding

use super::{EncodedFrame, EncoderConfig, EncoderError, FrameType, VideoEncoder};
use openh264::encoder::{Encoder, EncoderConfig as H264Config};
use openh264::formats::YUVBuffer;
use openh264::OpenH264API;
use parking_lot::Mutex;

pub struct SoftwareEncoder {
    config: Option<EncoderConfig>,
    encoder: Option<Mutex<Encoder>>,
    force_keyframe: bool,
    frame_count: u64,
}

impl SoftwareEncoder {
    pub fn new() -> Result<Self, EncoderError> {
        Ok(Self {
            config: None,
            encoder: None,
            force_keyframe: false,
            frame_count: 0,
        })
    }

    /// Convert BGRA to YUV420 (I420) format for H.264 encoding
    fn bgra_to_yuv420(bgra: &[u8], width: u32, height: u32) -> Vec<u8> {
        let w = width as usize;
        let h = height as usize;

        // YUV420 format: Y plane (w*h) + U plane (w/2 * h/2) + V plane (w/2 * h/2)
        let y_size = w * h;
        let uv_size = (w / 2) * (h / 2);
        let mut yuv = vec![0u8; y_size + 2 * uv_size];

        let (y_plane, uv_planes) = yuv.split_at_mut(y_size);
        let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);

        // Convert each pixel
        for y in 0..h {
            for x in 0..w {
                let bgra_idx = (y * w + x) * 4;
                let b = bgra[bgra_idx] as i32;
                let g = bgra[bgra_idx + 1] as i32;
                let r = bgra[bgra_idx + 2] as i32;

                // RGB to YUV conversion (BT.601)
                let y_val = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
                y_plane[y * w + x] = y_val.clamp(0, 255) as u8;

                // Subsample U and V (2x2 blocks)
                if y % 2 == 0 && x % 2 == 0 {
                    let uv_idx = (y / 2) * (w / 2) + (x / 2);
                    let u_val = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                    let v_val = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                    u_plane[uv_idx] = u_val.clamp(0, 255) as u8;
                    v_plane[uv_idx] = v_val.clamp(0, 255) as u8;
                }
            }
        }

        yuv
    }

    /// Check if encoded data starts with a keyframe (IDR NAL unit)
    fn is_keyframe(data: &[u8]) -> bool {
        // Look for NAL unit type 5 (IDR) or 7 (SPS) which indicates keyframe
        if data.len() < 5 {
            return false;
        }

        // Skip start code (0x00 0x00 0x00 0x01 or 0x00 0x00 0x01)
        let mut offset = 0;
        if data.len() > 4 && data[0] == 0 && data[1] == 0 {
            if data[2] == 0 && data[3] == 1 {
                offset = 4;
            } else if data[2] == 1 {
                offset = 3;
            }
        }

        if offset < data.len() {
            let nal_type = data[offset] & 0x1F;
            // NAL type 5 = IDR, 7 = SPS, 8 = PPS
            return nal_type == 5 || nal_type == 7;
        }

        false
    }
}

impl VideoEncoder for SoftwareEncoder {
    fn init(&mut self, config: EncoderConfig) -> Result<(), EncoderError> {
        // Get the OpenH264 API from compiled source
        let api = OpenH264API::from_source();

        // Configure OpenH264 encoder
        // Note: OpenH264 infers dimensions from the first YUVSource
        let h264_config = H264Config::new()
            .set_bitrate_bps(config.bitrate)
            .max_frame_rate(config.fps as f32)
            .enable_skip_frame(false); // Disable skip for consistent latency

        // Create encoder with config
        let encoder = Encoder::with_api_config(api, h264_config)
            .map_err(|e| EncoderError::InitError(format!("Failed to create OpenH264 encoder: {}", e)))?;

        self.encoder = Some(Mutex::new(encoder));
        self.config = Some(config.clone());
        self.frame_count = 0;

        log::info!(
            "OpenH264 software encoder initialized: {}x{} @ {} fps, {} bps",
            config.width,
            config.height,
            config.fps,
            config.bitrate
        );

        Ok(())
    }

    fn encode(&mut self, frame_data: &[u8], timestamp: u64) -> Result<EncodedFrame, EncoderError> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| EncoderError::EncodeError("Encoder not initialized".to_string()))?;

        let encoder_guard = self
            .encoder
            .as_ref()
            .ok_or_else(|| EncoderError::EncodeError("Encoder not initialized".to_string()))?;

        let mut encoder = encoder_guard.lock();

        // Force keyframe if requested
        if self.force_keyframe {
            encoder.force_intra_frame();
            self.force_keyframe = false;
        }

        // Convert BGRA to YUV420
        let yuv_data = Self::bgra_to_yuv420(frame_data, config.width, config.height);

        // Create YUV buffer from the converted data
        let yuv_buffer = YUVBuffer::from_vec(
            yuv_data,
            config.width as usize,
            config.height as usize,
        );

        // Encode the frame
        let bitstream = encoder
            .encode(&yuv_buffer)
            .map_err(|e| EncoderError::EncodeError(format!("Encode failed: {}", e)))?;

        // Collect encoded data
        let encoded_data = bitstream.to_vec();

        // Determine frame type from NAL units
        let frame_type = if Self::is_keyframe(&encoded_data) {
            FrameType::KeyFrame
        } else {
            FrameType::Delta
        };

        let size = encoded_data.len();
        self.frame_count += 1;

        Ok(EncodedFrame {
            data: encoded_data,
            timestamp,
            frame_type,
            size,
        })
    }

    fn request_keyframe(&mut self) {
        self.force_keyframe = true;
    }

    fn set_bitrate(&mut self, bitrate: u32) -> Result<(), EncoderError> {
        if let Some(ref mut config) = self.config {
            config.bitrate = bitrate;
            // OpenH264 doesn't support dynamic bitrate change easily,
            // would need to recreate the encoder
            log::info!("Bitrate change requested to {} bps (may require re-init)", bitrate);
        }
        Ok(())
    }

    fn info(&self) -> &str {
        "OpenH264 (Software)"
    }
}

impl Default for SoftwareEncoder {
    fn default() -> Self {
        Self::new().expect("Failed to create SoftwareEncoder")
    }
}

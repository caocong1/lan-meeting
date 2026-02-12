// Software encoder using Cisco OpenH264
// Cross-platform H.264 software encoding

use super::scaler::FrameScaler;
use super::{EncodedFrame, EncoderConfig, EncoderError, FrameType, VideoEncoder};
use openh264::encoder::{Encoder, EncoderConfig as H264Config};
use openh264::formats::YUVBuffer;
use openh264::OpenH264API;
use parking_lot::Mutex;

pub struct SoftwareEncoder {
    config: Option<EncoderConfig>,
    encoder: Option<Mutex<Encoder>>,
    scaler: Option<FrameScaler>,
    force_keyframe: bool,
    frame_count: u64,
}

impl SoftwareEncoder {
    pub fn new() -> Result<Self, EncoderError> {
        Ok(Self {
            config: None,
            encoder: None,
            scaler: None,
            force_keyframe: false,
            frame_count: 0,
        })
    }

    /// Convert BGRA to YUV420 (I420) format for H.264 encoding.
    ///
    /// Optimized with two-pass approach:
    /// - Pass 1: Y plane computed row-by-row (sequential memory access)
    /// - Pass 2: UV planes computed in 2x2 blocks using top-left pixel (no branching)
    fn bgra_to_yuv420(bgra: &[u8], width: u32, height: u32) -> Vec<u8> {
        let w = width as usize;
        let h = height as usize;
        let bgra_stride = w * 4;

        // YUV420 format: Y plane (w*h) + U plane (w/2 * h/2) + V plane (w/2 * h/2)
        let y_size = w * h;
        let uv_w = w / 2;
        let uv_h = h / 2;
        let uv_size = uv_w * uv_h;
        let mut yuv = vec![0u8; y_size + 2 * uv_size];

        let (y_plane, uv_planes) = yuv.split_at_mut(y_size);
        let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);

        // Pass 1: Compute Y plane (sequential row access, no branching)
        for y in 0..h {
            let src_row = y * bgra_stride;
            let dst_row = y * w;
            for x in 0..w {
                let si = src_row + x * 4;
                let b = bgra[si] as i32;
                let g = bgra[si + 1] as i32;
                let r = bgra[si + 2] as i32;
                y_plane[dst_row + x] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(0, 255) as u8;
            }
        }

        // Pass 2: Compute UV planes in 2x2 blocks (top-left pixel, no per-pixel branch)
        for by in 0..uv_h {
            let src_row = (by * 2) * bgra_stride;
            let uv_row = by * uv_w;
            for bx in 0..uv_w {
                let si = src_row + (bx * 2) * 4;
                let b = bgra[si] as i32;
                let g = bgra[si + 1] as i32;
                let r = bgra[si + 2] as i32;
                let ui = uv_row + bx;
                u_plane[ui] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
                v_plane[ui] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
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
        // Create scaler to handle oversized frames (OpenH264 max: 3840x2160)
        let scaler = FrameScaler::new(config.width, config.height);

        // Use scaled dimensions for encoder
        let encode_width = scaler.dst_width;
        let encode_height = scaler.dst_height;

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

        // Store scaler and modified config with scaled dimensions
        let mut scaled_config = config.clone();
        scaled_config.width = encode_width;
        scaled_config.height = encode_height;

        self.encoder = Some(Mutex::new(encoder));
        self.scaler = Some(scaler);
        self.config = Some(scaled_config);
        self.frame_count = 0;

        if config.width != encode_width || config.height != encode_height {
            log::info!(
                "OpenH264 software encoder initialized: {}x{} -> {}x{} (scaled) @ {} fps, {} bps",
                config.width,
                config.height,
                encode_width,
                encode_height,
                config.fps,
                config.bitrate
            );
        } else {
            log::info!(
                "OpenH264 software encoder initialized: {}x{} @ {} fps, {} bps",
                encode_width,
                encode_height,
                config.fps,
                config.bitrate
            );
        }

        Ok(())
    }

    fn encode(&mut self, frame_data: &[u8], timestamp: u64) -> Result<EncodedFrame, EncoderError> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| EncoderError::EncodeError("Encoder not initialized".to_string()))?;

        let scaler = self
            .scaler
            .as_ref()
            .ok_or_else(|| EncoderError::EncodeError("Scaler not initialized".to_string()))?;

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

        // Scale frame if needed (when resolution exceeds OpenH264 limits)
        let scaled_frame = scaler.scale(frame_data);

        // Convert BGRA to YUV420 using scaled dimensions
        let yuv_data = Self::bgra_to_yuv420(&scaled_frame, config.width, config.height);

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

    fn get_dimensions(&self) -> Option<(u32, u32)> {
        self.config.as_ref().map(|c| (c.width, c.height))
    }
}

impl Default for SoftwareEncoder {
    fn default() -> Self {
        Self::new().expect("Failed to create SoftwareEncoder")
    }
}

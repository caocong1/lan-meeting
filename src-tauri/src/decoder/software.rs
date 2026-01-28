// Software decoder using Cisco OpenH264
// Cross-platform H.264 software decoding

use super::{DecodedFrame, DecoderConfig, DecoderError, OutputFormat, VideoDecoder};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use parking_lot::Mutex;

pub struct SoftwareDecoder {
    config: Option<DecoderConfig>,
    decoder: Option<Mutex<Decoder>>,
    frame_count: u64,
}

impl SoftwareDecoder {
    pub fn new() -> Result<Self, DecoderError> {
        Ok(Self {
            config: None,
            decoder: None,
            frame_count: 0,
        })
    }

    /// Convert YUV420 to BGRA format
    fn yuv420_to_bgra(
        y_data: &[u8],
        u_data: &[u8],
        v_data: &[u8],
        y_stride: usize,
        u_stride: usize,
        v_stride: usize,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let w = width as usize;
        let h = height as usize;
        let mut bgra = vec![0u8; w * h * 4];

        for y in 0..h {
            for x in 0..w {
                let y_idx = y * y_stride + x;
                let uv_x = x / 2;
                let uv_y = y / 2;
                let u_idx = uv_y * u_stride + uv_x;
                let v_idx = uv_y * v_stride + uv_x;

                let y_val = y_data[y_idx] as i32;
                let u_val = u_data[u_idx] as i32 - 128;
                let v_val = v_data[v_idx] as i32 - 128;

                // YUV to RGB conversion (BT.601)
                let r = (y_val + ((v_val * 359) >> 8)).clamp(0, 255) as u8;
                let g = (y_val - ((u_val * 88 + v_val * 183) >> 8)).clamp(0, 255) as u8;
                let b = (y_val + ((u_val * 454) >> 8)).clamp(0, 255) as u8;

                let bgra_idx = (y * w + x) * 4;
                bgra[bgra_idx] = b;
                bgra[bgra_idx + 1] = g;
                bgra[bgra_idx + 2] = r;
                bgra[bgra_idx + 3] = 255;
            }
        }

        bgra
    }
}

impl VideoDecoder for SoftwareDecoder {
    fn init(&mut self, config: DecoderConfig) -> Result<(), DecoderError> {
        // Create decoder
        let decoder = Decoder::new()
            .map_err(|e| DecoderError::InitError(format!("Failed to create OpenH264 decoder: {}", e)))?;

        self.decoder = Some(Mutex::new(decoder));
        self.config = Some(config.clone());
        self.frame_count = 0;

        log::info!(
            "OpenH264 software decoder initialized: {}x{}, output format: {:?}",
            config.width,
            config.height,
            config.output_format
        );

        Ok(())
    }

    fn decode(&mut self, data: &[u8], timestamp: u64) -> Result<Option<DecodedFrame>, DecoderError> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| DecoderError::DecodeError("Decoder not initialized".to_string()))?;

        let decoder_guard = self
            .decoder
            .as_ref()
            .ok_or_else(|| DecoderError::DecodeError("Decoder not initialized".to_string()))?;

        let mut decoder = decoder_guard.lock();

        // Decode the H.264 NAL units
        let maybe_yuv = decoder
            .decode(data)
            .map_err(|e| DecoderError::DecodeError(format!("Decode failed: {}", e)))?;

        // OpenH264 may not produce a frame for every input (buffering)
        let Some(yuv) = maybe_yuv else {
            return Ok(None);
        };

        let (width, height) = yuv.dimensions();
        let width = width as u32;
        let height = height as u32;

        self.frame_count += 1;

        // Convert based on requested output format
        match config.output_format {
            OutputFormat::BGRA => {
                let (y_stride, u_stride, v_stride) = yuv.strides();
                let bgra = Self::yuv420_to_bgra(
                    yuv.y(),
                    yuv.u(),
                    yuv.v(),
                    y_stride,
                    u_stride,
                    v_stride,
                    width,
                    height,
                );

                Ok(Some(DecodedFrame::bgra(width, height, timestamp, bgra)))
            }
            OutputFormat::YUV420 => {
                let (y_stride, u_stride, v_stride) = yuv.strides();

                // Copy YUV data to contiguous buffer
                let y_size = y_stride * height as usize;
                let uv_height = (height as usize + 1) / 2;
                let u_size = u_stride * uv_height;
                let v_size = v_stride * uv_height;

                let mut yuv_data = Vec::with_capacity(y_size + u_size + v_size);
                yuv_data.extend_from_slice(&yuv.y()[..y_size]);
                yuv_data.extend_from_slice(&yuv.u()[..u_size]);
                yuv_data.extend_from_slice(&yuv.v()[..v_size]);

                Ok(Some(DecodedFrame::yuv420(
                    width,
                    height,
                    timestamp,
                    yuv_data,
                    [y_stride, u_stride, v_stride],
                )))
            }
        }
    }

    fn flush(&mut self) -> Result<Vec<DecodedFrame>, DecoderError> {
        // OpenH264 doesn't have explicit flush, frames are output immediately
        Ok(vec![])
    }

    fn info(&self) -> &str {
        "OpenH264 (Software)"
    }
}

impl Default for SoftwareDecoder {
    fn default() -> Self {
        Self::new().expect("Failed to create SoftwareDecoder")
    }
}

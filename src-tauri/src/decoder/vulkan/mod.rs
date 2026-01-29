//! Vulkan Video hardware decoder using vk-video
//!
//! Provides hardware-accelerated H.264 decoding via Vulkan Video extension.
//! Supported on NVIDIA and AMD GPUs with recent drivers.
//!
//! Note: Not available on macOS (Apple uses Metal, not Vulkan).
//! Note: vk-video uses wgpu 24 while our renderer uses wgpu 28.
//! For now, this decoder outputs to CPU memory (NV12 -> BGRA conversion).

use crate::decoder::{DecodedFrame, DecoderConfig, DecoderError, VideoDecoder};

#[cfg(not(target_os = "macos"))]
use crate::decoder::OutputFormat;

// vk-video is not available on macOS
#[cfg(not(target_os = "macos"))]
mod inner {
    use super::*;
    use parking_lot::Mutex;
    use std::sync::Arc;

    /// Vulkan Video decoder state
    struct VulkanDecoderState {
        device: Arc<vk_video::VulkanDevice>,
        width: u32,
        height: u32,
        output_format: OutputFormat,
    }

    /// Vulkan Video hardware decoder
    pub struct VulkanDecoder {
        state: Option<Mutex<VulkanDecoderState>>,
        instance: Option<Arc<vk_video::VulkanInstance>>,
    }

    impl VulkanDecoder {
        /// Create a new Vulkan Video decoder
        pub fn new() -> Result<Self, DecoderError> {
            // Try to initialize Vulkan Video
            let instance = vk_video::VulkanInstance::new()
                .map_err(|e| DecoderError::InitError(format!("Failed to create Vulkan instance: {:?}", e)))?;

            log::info!("Vulkan Video decoder available");
            Ok(Self {
                state: None,
                instance: Some(instance),
            })
        }

        /// Convert NV12 to BGRA
        fn nv12_to_bgra(nv12: &[u8], width: u32, height: u32) -> Vec<u8> {
            let w = width as usize;
            let h = height as usize;
            let mut bgra = vec![0u8; w * h * 4];

            let y_plane = &nv12[..w * h];
            let uv_plane = &nv12[w * h..];

            for row in 0..h {
                for col in 0..w {
                    let y_idx = row * w + col;
                    let uv_idx = (row / 2) * w + (col / 2) * 2;

                    let y = y_plane[y_idx] as i32;
                    let u = uv_plane.get(uv_idx).copied().unwrap_or(128) as i32 - 128;
                    let v = uv_plane.get(uv_idx + 1).copied().unwrap_or(128) as i32 - 128;

                    // YUV to RGB (BT.601)
                    let r = ((298 * (y - 16) + 409 * v + 128) >> 8).clamp(0, 255) as u8;
                    let g = ((298 * (y - 16) - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
                    let b = ((298 * (y - 16) + 516 * u + 128) >> 8).clamp(0, 255) as u8;

                    let bgra_idx = (row * w + col) * 4;
                    bgra[bgra_idx] = b;
                    bgra[bgra_idx + 1] = g;
                    bgra[bgra_idx + 2] = r;
                    bgra[bgra_idx + 3] = 255;
                }
            }

            bgra
        }

        /// Convert NV12 to YUV420P (planar)
        fn nv12_to_yuv420p(nv12: &[u8], width: u32, height: u32) -> (Vec<u8>, [usize; 3]) {
            let w = width as usize;
            let h = height as usize;

            let y_size = w * h;
            let uv_size = (w / 2) * (h / 2);
            let mut yuv420p = vec![0u8; y_size + 2 * uv_size];

            // Copy Y plane (same in both formats)
            yuv420p[..y_size].copy_from_slice(&nv12[..y_size]);

            // Deinterleave UV from NV12 (interleaved) to YUV420P (planar)
            let nv12_uv = &nv12[y_size..];
            let (u_plane, v_plane) = yuv420p[y_size..].split_at_mut(uv_size);

            for i in 0..uv_size {
                u_plane[i] = nv12_uv.get(i * 2).copied().unwrap_or(128);
                v_plane[i] = nv12_uv.get(i * 2 + 1).copied().unwrap_or(128);
            }

            let strides = [w, w / 2, w / 2];
            (yuv420p, strides)
        }
    }

    impl VideoDecoder for VulkanDecoder {
        fn init(&mut self, config: DecoderConfig) -> Result<(), DecoderError> {
            let instance = self.instance.as_ref()
                .ok_or_else(|| DecoderError::InitError("Vulkan instance not available".to_string()))?;

            // Create device
            let device = instance
                .create_device(
                    wgpu::Features::empty(),
                    wgpu::Limits::default(),
                    None, // No surface needed for decode-only
                )
                .map_err(|e| DecoderError::InitError(format!("Failed to create Vulkan device: {:?}", e)))?;

            let state = VulkanDecoderState {
                device,
                width: config.width,
                height: config.height,
                output_format: config.output_format,
            };

            self.state = Some(Mutex::new(state));

            log::info!(
                "Vulkan Video decoder initialized: {}x{}, output: {:?}",
                config.width,
                config.height,
                config.output_format
            );

            Ok(())
        }

        fn decode(&mut self, data: &[u8], timestamp: u64) -> Result<Option<DecodedFrame>, DecoderError> {
            let state_guard = self.state.as_ref()
                .ok_or_else(|| DecoderError::DecodeError("Decoder not initialized".to_string()))?;

            let state = state_guard.lock();

            // Create a BytesDecoder for this decode operation
            // Note: BytesDecoder has lifetime constraints tied to VulkanDevice
            let mut decoder = state.device.create_bytes_decoder()
                .map_err(|e| DecoderError::DecodeError(format!("Failed to create decoder: {:?}", e)))?;

            // Create encoded chunk from H.264 data
            let chunk = vk_video::EncodedChunk {
                data,
                pts: Some(timestamp),
            };

            // Decode
            let frames = decoder.decode(chunk)
                .map_err(|e| DecoderError::DecodeError(format!("Decode failed: {:?}", e)))?;

            // Get the first decoded frame if available
            if let Some(frame) = frames.into_iter().next() {
                let raw_data = frame.data;
                let width = raw_data.width;
                let height = raw_data.height;
                let nv12_data = raw_data.frame;
                let pts = frame.pts.unwrap_or(timestamp);

                // Convert based on output format
                let decoded = match state.output_format {
                    OutputFormat::BGRA => {
                        let bgra = Self::nv12_to_bgra(&nv12_data, width, height);
                        DecodedFrame::bgra(width, height, pts, bgra)
                    }
                    OutputFormat::YUV420 => {
                        let (yuv420p, strides) = Self::nv12_to_yuv420p(&nv12_data, width, height);
                        DecodedFrame::yuv420(width, height, pts, yuv420p, strides)
                    }
                };

                Ok(Some(decoded))
            } else {
                // No frame available yet (buffering)
                Ok(None)
            }
        }

        fn flush(&mut self) -> Result<Vec<DecodedFrame>, DecoderError> {
            let state_guard = self.state.as_ref()
                .ok_or_else(|| DecoderError::DecodeError("Decoder not initialized".to_string()))?;

            let state = state_guard.lock();

            // Create decoder and flush
            let mut decoder = state.device.create_bytes_decoder()
                .map_err(|e| DecoderError::DecodeError(format!("Failed to create decoder: {:?}", e)))?;

            let frames = decoder.flush();

            let decoded_frames: Vec<DecodedFrame> = frames
                .into_iter()
                .map(|frame| {
                    let raw_data = frame.data;
                    let width = raw_data.width;
                    let height = raw_data.height;
                    let nv12_data = raw_data.frame;
                    let pts = frame.pts.unwrap_or(0);

                    match state.output_format {
                        OutputFormat::BGRA => {
                            let bgra = Self::nv12_to_bgra(&nv12_data, width, height);
                            DecodedFrame::bgra(width, height, pts, bgra)
                        }
                        OutputFormat::YUV420 => {
                            let (yuv420p, strides) = Self::nv12_to_yuv420p(&nv12_data, width, height);
                            DecodedFrame::yuv420(width, height, pts, yuv420p, strides)
                        }
                    }
                })
                .collect();

            Ok(decoded_frames)
        }

        fn info(&self) -> &str {
            "Vulkan Video (Hardware)"
        }
    }
}

// Stub for macOS
#[cfg(target_os = "macos")]
mod inner {
    use super::*;

    /// Stub Vulkan decoder for macOS (not supported)
    pub struct VulkanDecoder;

    impl VulkanDecoder {
        pub fn new() -> Result<Self, DecoderError> {
            Err(DecoderError::HardwareNotAvailable)
        }
    }

    impl VideoDecoder for VulkanDecoder {
        fn init(&mut self, _config: DecoderConfig) -> Result<(), DecoderError> {
            Err(DecoderError::HardwareNotAvailable)
        }

        fn decode(&mut self, _data: &[u8], _timestamp: u64) -> Result<Option<DecodedFrame>, DecoderError> {
            Err(DecoderError::HardwareNotAvailable)
        }

        fn flush(&mut self) -> Result<Vec<DecodedFrame>, DecoderError> {
            Err(DecoderError::HardwareNotAvailable)
        }

        fn info(&self) -> &str {
            "Vulkan Video (Not available on macOS)"
        }
    }
}

pub use inner::VulkanDecoder;

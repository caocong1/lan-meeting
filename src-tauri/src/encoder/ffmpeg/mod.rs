//! FFmpeg-based hardware-accelerated video encoder
//!
//! Supports hardware encoders:
//! - NVENC (NVIDIA)
//! - VideoToolbox (macOS)
//! - VAAPI (Linux)
//! - QSV (Intel)
//! - libx264 software fallback

use crate::encoder::{EncodedFrame, EncoderConfig, EncoderError, EncoderPreset, FrameType, VideoEncoder};
use ffmpeg_next as ffmpeg;
use ffmpeg_next::codec::Context;
use ffmpeg_next::encoder::Video as VideoEncoder_;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::frame::Video as VideoFrame;
use ffmpeg_next::{Dictionary, Packet, Rational};
use parking_lot::Mutex;
use std::sync::Once;

static FFMPEG_INIT: Once = Once::new();

/// Initialize FFmpeg (call once)
fn init_ffmpeg() {
    FFMPEG_INIT.call_once(|| {
        ffmpeg::init().expect("Failed to initialize FFmpeg");
        // Enable verbose logging in debug builds
        if cfg!(debug_assertions) {
            ffmpeg::log::set_level(ffmpeg::log::Level::Info);
        }
    });
}

/// Hardware encoder types in order of preference
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HwEncoderType {
    Nvenc,        // NVIDIA NVENC
    VideoToolbox, // Apple VideoToolbox
    Vaapi,        // Linux VAAPI
    Qsv,          // Intel QuickSync
    Libx264,      // Software fallback
}

impl HwEncoderType {
    /// Get the FFmpeg codec name for H.264
    fn codec_name(&self) -> &'static str {
        match self {
            HwEncoderType::Nvenc => "h264_nvenc",
            HwEncoderType::VideoToolbox => "h264_videotoolbox",
            HwEncoderType::Vaapi => "h264_vaapi",
            HwEncoderType::Qsv => "h264_qsv",
            HwEncoderType::Libx264 => "libx264",
        }
    }

    /// Get encoder-specific options
    fn options(&self, preset: EncoderPreset) -> Dictionary<'static> {
        let mut opts = Dictionary::new();

        match self {
            HwEncoderType::Nvenc => {
                // NVENC options for low latency
                opts.set("preset", match preset {
                    EncoderPreset::UltraFast => "p1",  // Fastest
                    EncoderPreset::Fast => "p2",
                    EncoderPreset::Medium => "p4",
                    EncoderPreset::Quality => "p7",    // Best quality
                });
                opts.set("tune", "ll");  // Low latency
                opts.set("rc", "cbr");   // Constant bitrate
                opts.set("zerolatency", "1");
            }
            HwEncoderType::VideoToolbox => {
                // VideoToolbox options
                opts.set("realtime", "1");
                opts.set("allow_sw", "0");  // Prefer hardware
            }
            HwEncoderType::Vaapi => {
                // VAAPI options
                opts.set("rc_mode", "CBR");
            }
            HwEncoderType::Qsv => {
                // Intel QSV options
                opts.set("preset", match preset {
                    EncoderPreset::UltraFast => "veryfast",
                    EncoderPreset::Fast => "faster",
                    EncoderPreset::Medium => "medium",
                    EncoderPreset::Quality => "veryslow",
                });
            }
            HwEncoderType::Libx264 => {
                // libx264 options for low latency
                opts.set("preset", match preset {
                    EncoderPreset::UltraFast => "ultrafast",
                    EncoderPreset::Fast => "veryfast",
                    EncoderPreset::Medium => "medium",
                    EncoderPreset::Quality => "slow",
                });
                opts.set("tune", "zerolatency");
                opts.set("crf", "23");
            }
        }

        opts
    }
}

/// FFmpeg-based video encoder with hardware acceleration
pub struct FfmpegEncoder {
    encoder: Option<Mutex<VideoEncoder_>>,
    config: Option<EncoderConfig>,
    encoder_type: HwEncoderType,
    force_keyframe: bool,
    frame_count: u64,
    pts: i64,
}

impl FfmpegEncoder {
    /// Create a new FFmpeg encoder, trying hardware encoders in order
    pub fn new() -> Result<Self, EncoderError> {
        init_ffmpeg();

        // Try hardware encoders in order of preference
        let encoder_type = Self::detect_best_encoder()?;

        log::info!("Selected FFmpeg encoder: {:?}", encoder_type);

        Ok(Self {
            encoder: None,
            config: None,
            encoder_type,
            force_keyframe: false,
            frame_count: 0,
            pts: 0,
        })
    }

    /// Create with a specific encoder type
    pub fn with_type(encoder_type: HwEncoderType) -> Result<Self, EncoderError> {
        init_ffmpeg();

        // Verify the encoder is available
        let codec_name = encoder_type.codec_name();
        ffmpeg::encoder::find_by_name(codec_name)
            .ok_or_else(|| EncoderError::InitError(format!("Codec {} not found", codec_name)))?;

        Ok(Self {
            encoder: None,
            config: None,
            encoder_type,
            force_keyframe: false,
            frame_count: 0,
            pts: 0,
        })
    }

    /// Detect the best available hardware encoder
    fn detect_best_encoder() -> Result<HwEncoderType, EncoderError> {
        // Platform-specific priority
        #[cfg(target_os = "macos")]
        let priority = [
            HwEncoderType::VideoToolbox,
            HwEncoderType::Libx264,
        ];

        #[cfg(target_os = "windows")]
        let priority = [
            HwEncoderType::Nvenc,
            HwEncoderType::Qsv,
            HwEncoderType::Libx264,
        ];

        #[cfg(target_os = "linux")]
        let priority = [
            HwEncoderType::Nvenc,
            HwEncoderType::Vaapi,
            HwEncoderType::Qsv,
            HwEncoderType::Libx264,
        ];

        for encoder_type in priority {
            let codec_name = encoder_type.codec_name();
            if ffmpeg::encoder::find_by_name(codec_name).is_some() {
                log::info!("Found encoder: {}", codec_name);
                return Ok(encoder_type);
            } else {
                log::debug!("Encoder not available: {}", codec_name);
            }
        }

        Err(EncoderError::HardwareNotAvailable)
    }

    /// Convert BGRA to YUV420P for encoding (two-pass, no branching)
    fn bgra_to_yuv420(bgra: &[u8], width: u32, height: u32) -> Vec<u8> {
        let w = width as usize;
        let h = height as usize;
        let bgra_stride = w * 4;

        let y_size = w * h;
        let uv_w = w / 2;
        let uv_h = h / 2;
        let uv_size = uv_w * uv_h;
        let mut yuv = vec![0u8; y_size + 2 * uv_size];

        let (y_plane, uv_planes) = yuv.split_at_mut(y_size);
        let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);

        // Pass 1: Y plane (sequential row access, no branching)
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

        // Pass 2: UV planes in 2x2 blocks (top-left pixel, no per-pixel branch)
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

    /// Check if NAL unit indicates a keyframe
    fn is_keyframe(data: &[u8]) -> bool {
        if data.len() < 5 {
            return false;
        }

        // Find NAL unit start code
        let mut i = 0;
        while i < data.len() - 4 {
            if data[i] == 0 && data[i + 1] == 0 {
                let (start_code_len, nal_offset) = if data[i + 2] == 0 && data[i + 3] == 1 {
                    (4, i + 4)
                } else if data[i + 2] == 1 {
                    (3, i + 3)
                } else {
                    i += 1;
                    continue;
                };

                if nal_offset < data.len() {
                    let nal_type = data[nal_offset] & 0x1F;
                    // NAL type 5 = IDR, 7 = SPS, 8 = PPS
                    if nal_type == 5 || nal_type == 7 {
                        return true;
                    }
                }
                i += start_code_len;
            } else {
                i += 1;
            }
        }

        false
    }
}

impl VideoEncoder for FfmpegEncoder {
    fn init(&mut self, config: EncoderConfig) -> Result<(), EncoderError> {
        let codec_name = self.encoder_type.codec_name();
        let codec = ffmpeg::encoder::find_by_name(codec_name)
            .ok_or_else(|| EncoderError::InitError(format!("Codec {} not found", codec_name)))?;

        let context = Context::new_with_codec(codec);
        let mut encoder = context.encoder().video()
            .map_err(|e| EncoderError::InitError(format!("Failed to create encoder context: {}", e)))?;

        // Configure encoder
        encoder.set_width(config.width);
        encoder.set_height(config.height);
        encoder.set_format(Pixel::YUV420P);
        encoder.set_time_base(Rational::new(1, config.fps as i32));
        encoder.set_frame_rate(Some(Rational::new(config.fps as i32, 1)));
        encoder.set_bit_rate(config.bitrate as usize);
        encoder.set_max_bit_rate(config.max_bitrate as usize);
        encoder.set_gop(config.keyframe_interval);

        // Set encoder-specific options
        let opts = self.encoder_type.options(config.preset);

        let encoder = encoder.open_with(opts)
            .map_err(|e| EncoderError::InitError(format!("Failed to open encoder: {}", e)))?;

        self.encoder = Some(Mutex::new(encoder));
        self.config = Some(config.clone());
        self.frame_count = 0;
        self.pts = 0;

        log::info!(
            "FFmpeg {} encoder initialized: {}x{} @ {} fps, {} bps",
            codec_name,
            config.width,
            config.height,
            config.fps,
            config.bitrate
        );

        Ok(())
    }

    fn encode(&mut self, frame_data: &[u8], timestamp: u64) -> Result<EncodedFrame, EncoderError> {
        let config = self.config.as_ref()
            .ok_or_else(|| EncoderError::EncodeError("Encoder not initialized".to_string()))?;

        let encoder_guard = self.encoder.as_ref()
            .ok_or_else(|| EncoderError::EncodeError("Encoder not initialized".to_string()))?;

        let mut encoder = encoder_guard.lock();

        // Convert BGRA to YUV420P
        let yuv_data = Self::bgra_to_yuv420(frame_data, config.width, config.height);

        // Create video frame
        let mut frame = VideoFrame::new(Pixel::YUV420P, config.width, config.height);
        frame.set_pts(Some(self.pts));

        // Force keyframe if requested
        if self.force_keyframe {
            frame.set_kind(ffmpeg::picture::Type::I);
            self.force_keyframe = false;
        }

        // Copy YUV data to frame planes
        {
            let y_size = (config.width * config.height) as usize;
            let uv_size = ((config.width / 2) * (config.height / 2)) as usize;

            let y_stride = frame.stride(0);
            let u_stride = frame.stride(1);
            let v_stride = frame.stride(2);

            // Copy Y plane
            for y in 0..config.height as usize {
                let src_offset = y * config.width as usize;
                let dst_offset = y * y_stride;
                frame.data_mut(0)[dst_offset..dst_offset + config.width as usize]
                    .copy_from_slice(&yuv_data[src_offset..src_offset + config.width as usize]);
            }

            // Copy U plane
            for y in 0..(config.height / 2) as usize {
                let src_offset = y_size + y * (config.width / 2) as usize;
                let dst_offset = y * u_stride;
                frame.data_mut(1)[dst_offset..dst_offset + (config.width / 2) as usize]
                    .copy_from_slice(&yuv_data[src_offset..src_offset + (config.width / 2) as usize]);
            }

            // Copy V plane
            for y in 0..(config.height / 2) as usize {
                let src_offset = y_size + uv_size + y * (config.width / 2) as usize;
                let dst_offset = y * v_stride;
                frame.data_mut(2)[dst_offset..dst_offset + (config.width / 2) as usize]
                    .copy_from_slice(&yuv_data[src_offset..src_offset + (config.width / 2) as usize]);
            }
        }

        // Send frame to encoder
        encoder.send_frame(&frame)
            .map_err(|e| EncoderError::EncodeError(format!("Failed to send frame: {}", e)))?;

        // Receive encoded packet
        let mut packet = Packet::empty();
        let mut encoded_data = Vec::new();

        while encoder.receive_packet(&mut packet).is_ok() {
            encoded_data.extend_from_slice(packet.data().unwrap_or(&[]));
        }

        // If no data, the encoder is buffering
        if encoded_data.is_empty() {
            // Return an empty delta frame - this is normal for B-frame encoders
            return Ok(EncodedFrame {
                data: vec![],
                timestamp,
                frame_type: FrameType::Delta,
                size: 0,
            });
        }

        let frame_type = if Self::is_keyframe(&encoded_data) {
            FrameType::KeyFrame
        } else {
            FrameType::Delta
        };

        let size = encoded_data.len();
        self.frame_count += 1;
        self.pts += 1;

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
            log::info!("Bitrate change requested to {} bps", bitrate);
            // Note: Dynamic bitrate change would require recreating the encoder
            // or using encoder-specific rate control APIs
        }
        Ok(())
    }

    fn info(&self) -> &str {
        match self.encoder_type {
            HwEncoderType::Nvenc => "FFmpeg NVENC (Hardware)",
            HwEncoderType::VideoToolbox => "FFmpeg VideoToolbox (Hardware)",
            HwEncoderType::Vaapi => "FFmpeg VAAPI (Hardware)",
            HwEncoderType::Qsv => "FFmpeg QuickSync (Hardware)",
            HwEncoderType::Libx264 => "FFmpeg libx264 (Software)",
        }
    }

    fn get_dimensions(&self) -> Option<(u32, u32)> {
        self.config.as_ref().map(|c| (c.width, c.height))
    }
}

impl Default for FfmpegEncoder {
    fn default() -> Self {
        Self::new().expect("Failed to create FfmpegEncoder")
    }
}

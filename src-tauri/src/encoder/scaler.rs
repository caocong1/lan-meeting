//! Frame scaler for downscaling BGRA frames when resolution exceeds encoder limits
//!
//! OpenH264 has a maximum resolution of 3840x2160 (4K UHD).
//! This module provides efficient bilinear scaling for oversized frames.

/// Maximum dimensions supported by OpenH264
pub const OPENH264_MAX_WIDTH: u32 = 3840;
pub const OPENH264_MAX_HEIGHT: u32 = 2160;

/// Frame scaler for BGRA frames
pub struct FrameScaler {
    /// Original dimensions
    pub src_width: u32,
    pub src_height: u32,
    /// Target dimensions (after scaling)
    pub dst_width: u32,
    pub dst_height: u32,
    /// Whether scaling is needed
    pub needs_scaling: bool,
}

impl FrameScaler {
    /// Create a new scaler that fits dimensions within OpenH264 limits
    /// while maintaining aspect ratio
    pub fn new(src_width: u32, src_height: u32) -> Self {
        let (dst_width, dst_height, needs_scaling) =
            Self::calculate_scaled_dimensions(src_width, src_height);

        if needs_scaling {
            log::info!(
                "Frame scaler initialized: {}x{} -> {}x{} (aspect ratio preserved)",
                src_width, src_height, dst_width, dst_height
            );
        }

        Self {
            src_width,
            src_height,
            dst_width,
            dst_height,
            needs_scaling,
        }
    }

    /// Calculate target dimensions that fit within OpenH264 limits
    fn calculate_scaled_dimensions(width: u32, height: u32) -> (u32, u32, bool) {
        if width <= OPENH264_MAX_WIDTH && height <= OPENH264_MAX_HEIGHT {
            return (width, height, false);
        }

        // Calculate scale factor to fit within limits
        let scale_w = OPENH264_MAX_WIDTH as f64 / width as f64;
        let scale_h = OPENH264_MAX_HEIGHT as f64 / height as f64;
        let scale = scale_w.min(scale_h);

        // Apply scale and ensure dimensions are even (required for YUV420)
        let new_width = ((width as f64 * scale) as u32) & !1;
        let new_height = ((height as f64 * scale) as u32) & !1;

        (new_width, new_height, true)
    }

    /// Scale a BGRA frame using bilinear interpolation
    /// Returns the scaled frame data, or the original if no scaling needed
    pub fn scale(&self, bgra: &[u8]) -> Vec<u8> {
        if !self.needs_scaling {
            return bgra.to_vec();
        }

        self.scale_bilinear(bgra)
    }

    /// Bilinear interpolation scaling for BGRA frames
    fn scale_bilinear(&self, src: &[u8]) -> Vec<u8> {
        let src_w = self.src_width as usize;
        let src_h = self.src_height as usize;
        let dst_w = self.dst_width as usize;
        let dst_h = self.dst_height as usize;

        let mut dst = vec![0u8; dst_w * dst_h * 4];

        let x_ratio = src_w as f64 / dst_w as f64;
        let y_ratio = src_h as f64 / dst_h as f64;

        for dst_y in 0..dst_h {
            let src_y_f = dst_y as f64 * y_ratio;
            let src_y0 = src_y_f as usize;
            let src_y1 = (src_y0 + 1).min(src_h - 1);
            let y_frac = src_y_f - src_y0 as f64;

            for dst_x in 0..dst_w {
                let src_x_f = dst_x as f64 * x_ratio;
                let src_x0 = src_x_f as usize;
                let src_x1 = (src_x0 + 1).min(src_w - 1);
                let x_frac = src_x_f - src_x0 as f64;

                // Get four surrounding pixels
                let idx00 = (src_y0 * src_w + src_x0) * 4;
                let idx01 = (src_y0 * src_w + src_x1) * 4;
                let idx10 = (src_y1 * src_w + src_x0) * 4;
                let idx11 = (src_y1 * src_w + src_x1) * 4;

                // Bilinear interpolation for each channel (BGRA)
                let dst_idx = (dst_y * dst_w + dst_x) * 4;
                for c in 0..4 {
                    let v00 = src[idx00 + c] as f64;
                    let v01 = src[idx01 + c] as f64;
                    let v10 = src[idx10 + c] as f64;
                    let v11 = src[idx11 + c] as f64;

                    let v0 = v00 * (1.0 - x_frac) + v01 * x_frac;
                    let v1 = v10 * (1.0 - x_frac) + v11 * x_frac;
                    let v = v0 * (1.0 - y_frac) + v1 * y_frac;

                    dst[dst_idx + c] = v as u8;
                }
            }
        }

        dst
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_scaling_needed() {
        let scaler = FrameScaler::new(1920, 1080);
        assert!(!scaler.needs_scaling);
        assert_eq!(scaler.dst_width, 1920);
        assert_eq!(scaler.dst_height, 1080);
    }

    #[test]
    fn test_scaling_needed_height() {
        // 3456x2168 exceeds 2160 height limit
        let scaler = FrameScaler::new(3456, 2168);
        assert!(scaler.needs_scaling);
        assert!(scaler.dst_height <= OPENH264_MAX_HEIGHT);
        assert!(scaler.dst_width <= OPENH264_MAX_WIDTH);
        // Check aspect ratio is preserved (approximately)
        let src_ratio = 3456.0 / 2168.0;
        let dst_ratio = scaler.dst_width as f64 / scaler.dst_height as f64;
        assert!((src_ratio - dst_ratio).abs() < 0.01);
    }

    #[test]
    fn test_scaling_needed_width() {
        // 4096x2160 exceeds 3840 width limit
        let scaler = FrameScaler::new(4096, 2160);
        assert!(scaler.needs_scaling);
        assert!(scaler.dst_width <= OPENH264_MAX_WIDTH);
    }

    #[test]
    fn test_dimensions_are_even() {
        let scaler = FrameScaler::new(3457, 2169);
        assert!(scaler.dst_width % 2 == 0);
        assert!(scaler.dst_height % 2 == 0);
    }
}

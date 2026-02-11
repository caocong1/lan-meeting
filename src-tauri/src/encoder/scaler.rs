//! Frame scaler for adapting BGRA frames when resolution exceeds encoder limits
//!
//! OpenH264 has a maximum resolution of 3840x2160 (4K UHD).
//! This module uses fast cropping to fit oversized frames within limits.
//! Cropping removes edge pixels rather than expensive per-pixel scaling.

/// Maximum dimensions supported by OpenH264
pub const OPENH264_MAX_WIDTH: u32 = 3840;
pub const OPENH264_MAX_HEIGHT: u32 = 2160;

/// How the frame is adapted to fit encoder limits
#[derive(Debug, Clone, Copy)]
enum AdaptMode {
    /// No adaptation needed
    None,
    /// Crop rows from bottom only (width unchanged)
    CropHeight,
    /// Crop columns from right only (height unchanged)
    CropWidth,
    /// Crop both rows and columns
    CropBoth,
}

/// Frame scaler for BGRA frames
pub struct FrameScaler {
    /// Original dimensions
    pub src_width: u32,
    pub src_height: u32,
    /// Target dimensions (after adaptation)
    pub dst_width: u32,
    pub dst_height: u32,
    /// Whether adaptation is needed
    pub needs_scaling: bool,
    /// Adaptation strategy
    mode: AdaptMode,
}

impl FrameScaler {
    /// Create a new scaler that fits dimensions within OpenH264 limits.
    /// Uses cropping (removing edge pixels) for near-zero performance cost.
    pub fn new(src_width: u32, src_height: u32) -> Self {
        let width_exceeds = src_width > OPENH264_MAX_WIDTH;
        let height_exceeds = src_height > OPENH264_MAX_HEIGHT;

        let (dst_width, dst_height, mode) = match (width_exceeds, height_exceeds) {
            (false, false) => (src_width, src_height, AdaptMode::None),
            (false, true) => {
                // Only height exceeds - crop to max height, keep even
                let h = OPENH264_MAX_HEIGHT & !1;
                (src_width, h, AdaptMode::CropHeight)
            }
            (true, false) => {
                // Only width exceeds - crop to max width, keep even
                let w = OPENH264_MAX_WIDTH & !1;
                (w, src_height, AdaptMode::CropWidth)
            }
            (true, true) => {
                // Both exceed - crop both
                let w = OPENH264_MAX_WIDTH & !1;
                let h = OPENH264_MAX_HEIGHT & !1;
                (w, h, AdaptMode::CropBoth)
            }
        };

        let needs_scaling = !matches!(mode, AdaptMode::None);

        if needs_scaling {
            log::info!(
                "Frame scaler initialized: {}x{} -> {}x{} (cropped)",
                src_width, src_height, dst_width, dst_height
            );
        }

        Self {
            src_width,
            src_height,
            dst_width,
            dst_height,
            needs_scaling,
            mode,
        }
    }

    /// Adapt a BGRA frame to fit within encoder limits.
    /// Returns cropped frame data, or the original slice if no adaptation needed.
    pub fn scale<'a>(&self, bgra: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        match self.mode {
            AdaptMode::None => {
                // No adaptation needed - return reference, zero cost
                std::borrow::Cow::Borrowed(bgra)
            }
            AdaptMode::CropHeight => {
                // Just take fewer rows - single slice operation
                let row_bytes = self.src_width as usize * 4;
                let total = row_bytes * self.dst_height as usize;
                std::borrow::Cow::Borrowed(&bgra[..total])
            }
            AdaptMode::CropWidth => {
                // Need to copy each row with fewer pixels
                self.crop_width(bgra)
            }
            AdaptMode::CropBoth => {
                // Crop both dimensions
                self.crop_both(bgra)
            }
        }
    }

    /// Crop width only - copy each row with fewer columns
    fn crop_width<'a>(&self, src: &[u8]) -> std::borrow::Cow<'a, [u8]> {
        let src_stride = self.src_width as usize * 4;
        let dst_stride = self.dst_width as usize * 4;
        let mut dst = vec![0u8; dst_stride * self.src_height as usize];

        for y in 0..self.src_height as usize {
            let src_offset = y * src_stride;
            let dst_offset = y * dst_stride;
            dst[dst_offset..dst_offset + dst_stride]
                .copy_from_slice(&src[src_offset..src_offset + dst_stride]);
        }

        std::borrow::Cow::Owned(dst)
    }

    /// Crop both width and height
    fn crop_both<'a>(&self, src: &[u8]) -> std::borrow::Cow<'a, [u8]> {
        let src_stride = self.src_width as usize * 4;
        let dst_stride = self.dst_width as usize * 4;
        let mut dst = vec![0u8; dst_stride * self.dst_height as usize];

        for y in 0..self.dst_height as usize {
            let src_offset = y * src_stride;
            let dst_offset = y * dst_stride;
            dst[dst_offset..dst_offset + dst_stride]
                .copy_from_slice(&src[src_offset..src_offset + dst_stride]);
        }

        std::borrow::Cow::Owned(dst)
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
    fn test_crop_height_only() {
        // 3456x2168 exceeds 2160 height limit
        let scaler = FrameScaler::new(3456, 2168);
        assert!(scaler.needs_scaling);
        assert_eq!(scaler.dst_width, 3456); // Width unchanged
        assert_eq!(scaler.dst_height, 2160); // Height cropped to limit
    }

    #[test]
    fn test_crop_width_only() {
        // 4096x2160 exceeds 3840 width limit
        let scaler = FrameScaler::new(4096, 2160);
        assert!(scaler.needs_scaling);
        assert_eq!(scaler.dst_width, 3840);
        assert_eq!(scaler.dst_height, 2160);
    }

    #[test]
    fn test_crop_both() {
        let scaler = FrameScaler::new(4096, 2200);
        assert!(scaler.needs_scaling);
        assert_eq!(scaler.dst_width, 3840);
        assert_eq!(scaler.dst_height, 2160);
    }

    #[test]
    fn test_dimensions_are_even() {
        let scaler = FrameScaler::new(3457, 2169);
        assert!(scaler.dst_width % 2 == 0);
        assert!(scaler.dst_height % 2 == 0);
    }

    #[test]
    fn test_crop_height_zero_cost() {
        // For height-only crop, the returned data should be a borrowed slice
        let scaler = FrameScaler::new(4, 6);
        // Create fake 4x6 BGRA frame (4 * 6 * 4 = 96 bytes)
        // Limit doesn't apply here, but let's test with a scaler that crops height
        let scaler = FrameScaler {
            src_width: 4,
            src_height: 6,
            dst_width: 4,
            dst_height: 4,
            needs_scaling: true,
            mode: AdaptMode::CropHeight,
        };
        let frame = vec![0u8; 4 * 6 * 4];
        let result = scaler.scale(&frame);
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
        assert_eq!(result.len(), 4 * 4 * 4);
    }
}

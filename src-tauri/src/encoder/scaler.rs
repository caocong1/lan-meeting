//! Frame scaler for adapting BGRA frames to target encoder dimensions
//!
//! Supports two modes:
//! 1. Cropping: fast edge removal when dimensions slightly exceed OpenH264 limits
//! 2. Downscaling: nearest-neighbor resize for significant resolution reduction

/// Maximum dimensions supported by OpenH264
pub const OPENH264_MAX_WIDTH: u32 = 3840;
pub const OPENH264_MAX_HEIGHT: u32 = 2160;

/// How the frame is adapted
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
    /// Nearest-neighbor downscale
    Downscale,
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
            (false, false) => (src_width & !1, src_height & !1, AdaptMode::None),
            (false, true) => {
                let h = OPENH264_MAX_HEIGHT & !1;
                (src_width & !1, h, AdaptMode::CropHeight)
            }
            (true, false) => {
                let w = OPENH264_MAX_WIDTH & !1;
                (w, src_height & !1, AdaptMode::CropWidth)
            }
            (true, true) => {
                let w = OPENH264_MAX_WIDTH & !1;
                let h = OPENH264_MAX_HEIGHT & !1;
                (w, h, AdaptMode::CropBoth)
            }
        };

        let needs_scaling = dst_width != src_width || dst_height != src_height;

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

    /// Create a scaler that downscales to fit within a target resolution box.
    /// Maintains source aspect ratio (fit-inside), clamped to even numbers and OpenH264 limits.
    /// If source is already smaller than or equal to target, no scaling is done.
    pub fn new_with_target(src_width: u32, src_height: u32, target_width: u32, target_height: u32) -> Self {
        // Fit source aspect ratio inside target box
        let max_w = target_width.min(src_width).min(OPENH264_MAX_WIDTH);
        let max_h = target_height.min(src_height).min(OPENH264_MAX_HEIGHT);

        let scale_w = max_w as f64 / src_width as f64;
        let scale_h = max_h as f64 / src_height as f64;
        let scale = scale_w.min(scale_h).min(1.0); // never upscale

        let dst_width = ((src_width as f64 * scale) as u32) & !1;
        let dst_height = ((src_height as f64 * scale) as u32) & !1;

        let needs_scaling = dst_width != src_width || dst_height != src_height;
        let mode = if needs_scaling {
            AdaptMode::Downscale
        } else {
            AdaptMode::None
        };

        if needs_scaling {
            log::info!(
                "Frame scaler initialized: {}x{} -> {}x{} (downscale)",
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

    /// Adapt a BGRA frame to fit target dimensions.
    /// Returns scaled/cropped frame data, or the original slice if no adaptation needed.
    pub fn scale<'a>(&self, bgra: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        match self.mode {
            AdaptMode::None => {
                std::borrow::Cow::Borrowed(bgra)
            }
            AdaptMode::CropHeight => {
                let row_bytes = self.src_width as usize * 4;
                let total = row_bytes * self.dst_height as usize;
                std::borrow::Cow::Borrowed(&bgra[..total])
            }
            AdaptMode::CropWidth => {
                self.crop_width(bgra)
            }
            AdaptMode::CropBoth => {
                self.crop_both(bgra)
            }
            AdaptMode::Downscale => {
                std::borrow::Cow::Owned(self.downscale_nearest(bgra))
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

    /// Nearest-neighbor downscale for BGRA frames.
    /// Precomputes X source offsets to avoid per-pixel division.
    fn downscale_nearest(&self, src: &[u8]) -> Vec<u8> {
        let sw = self.src_width as usize;
        let sh = self.src_height as usize;
        let dw = self.dst_width as usize;
        let dh = self.dst_height as usize;
        let src_stride = sw * 4;
        let dst_stride = dw * 4;
        let mut dst = vec![0u8; dst_stride * dh];

        // Precompute source X byte offsets for each destination column
        let x_offsets: Vec<usize> = (0..dw).map(|dx| (dx * sw / dw) * 4).collect();

        for dy in 0..dh {
            let sy = dy * sh / dh;
            let src_row = sy * src_stride;
            let dst_row = dy * dst_stride;
            for (dx, &sx_off) in x_offsets.iter().enumerate() {
                let si = src_row + sx_off;
                let di = dst_row + dx * 4;
                dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
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
    fn test_crop_height_only() {
        let scaler = FrameScaler::new(3456, 2168);
        assert!(scaler.needs_scaling);
        assert_eq!(scaler.dst_width, 3456);
        assert_eq!(scaler.dst_height, 2160);
    }

    #[test]
    fn test_crop_width_only() {
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

    #[test]
    fn test_downscale_target() {
        // 3456x2160 (16:10) into 1280x720 box → height-limited → 1152x720
        let scaler = FrameScaler::new_with_target(3456, 2160, 1280, 720);
        assert!(scaler.needs_scaling);
        assert_eq!(scaler.dst_width, 1152);
        assert_eq!(scaler.dst_height, 720);
    }

    #[test]
    fn test_downscale_preserves_aspect_ratio() {
        // 1920x1080 (16:9) into 1280x720 box → exact fit
        let scaler = FrameScaler::new_with_target(1920, 1080, 1280, 720);
        assert_eq!(scaler.dst_width, 1280);
        assert_eq!(scaler.dst_height, 720);
    }

    #[test]
    fn test_downscale_no_upscale() {
        // Target larger than source - should not upscale
        let scaler = FrameScaler::new_with_target(640, 480, 1280, 720);
        assert!(!scaler.needs_scaling);
        assert_eq!(scaler.dst_width, 640);
        assert_eq!(scaler.dst_height, 480);
    }

    #[test]
    fn test_downscale_even_dimensions() {
        let scaler = FrameScaler::new_with_target(3456, 2160, 1281, 721);
        assert_eq!(scaler.dst_width % 2, 0);
        assert_eq!(scaler.dst_height % 2, 0);
    }

    #[test]
    fn test_downscale_nearest_pixel_values() {
        let scaler = FrameScaler::new_with_target(4, 4, 2, 2);
        // 4x4 BGRA frame: pixel (0,0)=red, (1,0)=green, (0,1)=blue, (1,1)=white
        let mut src = vec![0u8; 4 * 4 * 4];
        // Row 0: red, green, red, green
        src[0..4].copy_from_slice(&[0, 0, 255, 255]); // BGRA red
        src[4..8].copy_from_slice(&[0, 255, 0, 255]); // BGRA green
        // Row 2: blue, white, blue, white
        let row2 = 2 * 4 * 4;
        src[row2..row2 + 4].copy_from_slice(&[255, 0, 0, 255]); // BGRA blue
        src[row2 + 4..row2 + 8].copy_from_slice(&[255, 255, 255, 255]); // BGRA white

        let result = scaler.scale(&src);
        assert_eq!(result.len(), 2 * 2 * 4);
        // (0,0) maps to src (0,0) = red
        assert_eq!(&result[0..4], &[0, 0, 255, 255]);
        // (1,0) maps to src (2,0) = red (same as 0,0 in our pattern)
        // (0,1) maps to src (0,2) = blue
        assert_eq!(&result[8..12], &[255, 0, 0, 255]);
    }
}

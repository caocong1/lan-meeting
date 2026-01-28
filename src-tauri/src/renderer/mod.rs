// GPU renderer module
// wgpu-based rendering for decoded frames

mod wgpu_renderer;
mod window;

pub use wgpu_renderer::WgpuRenderer;
pub use window::{RenderWindow, WindowEvent};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum RendererError {
    #[error("Failed to initialize renderer: {0}")]
    InitError(String),
    #[error("Render failed: {0}")]
    RenderError(String),
    #[error("Window error: {0}")]
    WindowError(String),
    #[error("GPU not available: {0}")]
    GpuNotAvailable(String),
}

/// Frame format for rendering
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameFormat {
    BGRA,
    YUV420,
}

/// Frame to be rendered
#[derive(Debug)]
pub struct RenderFrame {
    pub width: u32,
    pub height: u32,
    pub format: FrameFormat,
    pub data: Vec<u8>,
    /// For YUV420: strides for Y, U, V planes
    pub strides: Option<[usize; 3]>,
}

impl RenderFrame {
    pub fn from_bgra(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            format: FrameFormat::BGRA,
            data,
            strides: None,
        }
    }

    pub fn from_yuv420(width: u32, height: u32, data: Vec<u8>, strides: [usize; 3]) -> Self {
        Self {
            width,
            height,
            format: FrameFormat::YUV420,
            data,
            strides: Some(strides),
        }
    }
}

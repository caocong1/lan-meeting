// wgpu-based GPU renderer
// Efficient texture upload and rendering for video frames

use super::{FrameFormat, RenderFrame, RendererError};
use std::sync::Arc;

/// WGSL shader for rendering BGRA textures
const BGRA_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Full-screen quad using 6 vertices
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    var tex_coords = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 0.0),
    );

    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.tex_coord = tex_coords[vertex_index];
    return output;
}

@group(0) @binding(0) var frame_texture: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(frame_texture, frame_sampler, input.tex_coord);
}
"#;

/// WGSL shader for YUV420 to RGB conversion
const YUV_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    var tex_coords = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 0.0),
    );

    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.tex_coord = tex_coords[vertex_index];
    return output;
}

@group(0) @binding(0) var y_texture: texture_2d<f32>;
@group(0) @binding(1) var u_texture: texture_2d<f32>;
@group(0) @binding(2) var v_texture: texture_2d<f32>;
@group(0) @binding(3) var yuv_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let y = textureSample(y_texture, yuv_sampler, input.tex_coord).r;
    let u = textureSample(u_texture, yuv_sampler, input.tex_coord).r - 0.5;
    let v = textureSample(v_texture, yuv_sampler, input.tex_coord).r - 0.5;

    // BT.601 YUV to RGB conversion
    let r = y + 1.402 * v;
    let g = y - 0.344 * u - 0.714 * v;
    let b = y + 1.772 * u;

    return vec4<f32>(r, g, b, 1.0);
}
"#;

/// wgpu-based GPU renderer
pub struct WgpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: Option<wgpu::Surface<'static>>,
    surface_config: Option<wgpu::SurfaceConfiguration>,

    // BGRA pipeline
    bgra_pipeline: wgpu::RenderPipeline,
    bgra_bind_group_layout: wgpu::BindGroupLayout,
    bgra_texture: Option<wgpu::Texture>,
    bgra_bind_group: Option<wgpu::BindGroup>,

    // YUV pipeline
    yuv_pipeline: wgpu::RenderPipeline,
    yuv_bind_group_layout: wgpu::BindGroupLayout,
    yuv_textures: Option<(wgpu::Texture, wgpu::Texture, wgpu::Texture)>,
    yuv_bind_group: Option<wgpu::BindGroup>,

    // Samplers
    sampler: wgpu::Sampler,

    // Current frame dimensions
    frame_width: u32,
    frame_height: u32,
}

impl WgpuRenderer {
    /// Create a new renderer without a surface (headless)
    pub async fn new() -> Result<Self, RendererError> {
        Self::new_internal(None).await
    }

    /// Create a new renderer with a window surface
    pub async fn new_with_surface(
        window: Arc<winit::window::Window>,
    ) -> Result<Self, RendererError> {
        Self::new_internal(Some(window)).await
    }

    /// Create a new renderer with a pre-created raw surface (for macOS native windows).
    /// The instance must be the same one that created the surface.
    pub async fn new_with_raw_surface(
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, RendererError> {
        Self::new_internal_raw(instance, surface, width, height).await
    }

    async fn new_internal_raw(
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, RendererError> {

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|e| RendererError::GpuNotAvailable(format!("Failed to request adapter: {}", e)))?;

        log::info!("Using GPU adapter: {:?}", adapter.get_info().name);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|e| RendererError::InitError(format!("Failed to create device: {}", e)))?;

        // Configure surface
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(capabilities.formats[0]);

        // Pick the best present mode from what's supported
        let present_mode = if capabilities.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else if capabilities.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::Fifo // always supported
        };
        log::info!("wgpu present mode: {:?} (available: {:?})", present_mode, capabilities.present_modes);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Create sampler, pipelines (same as new_internal)
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Frame Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let bgra_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("BGRA Shader"),
            source: wgpu::ShaderSource::Wgsl(BGRA_SHADER.into()),
        });

        let bgra_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("BGRA Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let bgra_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("BGRA Pipeline Layout"),
                bind_group_layouts: &[&bgra_bind_group_layout],
                immediate_size: 0,
            });

        let bgra_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("BGRA Pipeline"),
            layout: Some(&bgra_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &bgra_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &bgra_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let yuv_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("YUV Shader"),
            source: wgpu::ShaderSource::Wgsl(YUV_SHADER.into()),
        });

        let yuv_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("YUV Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let yuv_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("YUV Pipeline Layout"),
                bind_group_layouts: &[&yuv_bind_group_layout],
                immediate_size: 0,
            });

        let yuv_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("YUV Pipeline"),
            layout: Some(&yuv_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &yuv_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &yuv_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        log::info!("wgpu renderer initialized (raw surface)");

        Ok(Self {
            device,
            queue,
            surface: Some(surface),
            surface_config: Some(config),
            bgra_pipeline,
            bgra_bind_group_layout,
            bgra_texture: None,
            bgra_bind_group: None,
            yuv_pipeline,
            yuv_bind_group_layout,
            yuv_textures: None,
            yuv_bind_group: None,
            sampler,
            frame_width: 0,
            frame_height: 0,
        })
    }

    async fn new_internal(
        window: Option<Arc<winit::window::Window>>,
    ) -> Result<Self, RendererError> {
        // Create wgpu instance
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Create surface if window provided
        let surface = if let Some(ref window) = window {
            Some(
                instance
                    .create_surface(window.clone())
                    .map_err(|e| RendererError::InitError(format!("Failed to create surface: {}", e)))?,
            )
        } else {
            None
        };

        // Request adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: surface.as_ref(),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|e| RendererError::GpuNotAvailable(format!("Failed to request adapter: {}", e)))?;

        log::info!("Using GPU adapter: {:?}", adapter.get_info().name);

        // Request device
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|e| RendererError::InitError(format!("Failed to create device: {}", e)))?;

        // Configure surface if available
        let surface_config = if let (Some(surface), Some(window)) = (&surface, &window) {
            let size = window.inner_size();
            let capabilities = surface.get_capabilities(&adapter);
            let format = capabilities
                .formats
                .iter()
                .find(|f| f.is_srgb())
                .copied()
                .unwrap_or(capabilities.formats[0]);

            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: wgpu::PresentMode::Mailbox, // Low latency
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };
            surface.configure(&device, &config);
            Some(config)
        } else {
            None
        };

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Frame Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // Create BGRA pipeline
        let bgra_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("BGRA Shader"),
            source: wgpu::ShaderSource::Wgsl(BGRA_SHADER.into()),
        });

        let bgra_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("BGRA Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let bgra_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("BGRA Pipeline Layout"),
                bind_group_layouts: &[&bgra_bind_group_layout],
                immediate_size: 0,
            });

        let surface_format = surface_config
            .as_ref()
            .map(|c| c.format)
            .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb);

        let bgra_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("BGRA Pipeline"),
            layout: Some(&bgra_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &bgra_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &bgra_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Create YUV pipeline
        let yuv_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("YUV Shader"),
            source: wgpu::ShaderSource::Wgsl(YUV_SHADER.into()),
        });

        let yuv_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("YUV Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let yuv_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("YUV Pipeline Layout"),
                bind_group_layouts: &[&yuv_bind_group_layout],
                immediate_size: 0,
            });

        let yuv_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("YUV Pipeline"),
            layout: Some(&yuv_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &yuv_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &yuv_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        log::info!("wgpu renderer initialized");

        Ok(Self {
            device,
            queue,
            surface,
            surface_config,
            bgra_pipeline,
            bgra_bind_group_layout,
            bgra_texture: None,
            bgra_bind_group: None,
            yuv_pipeline,
            yuv_bind_group_layout,
            yuv_textures: None,
            yuv_bind_group: None,
            sampler,
            frame_width: 0,
            frame_height: 0,
        })
    }

    /// Resize the render surface
    pub fn resize(&mut self, width: u32, height: u32) {
        if let (Some(surface), Some(config)) = (&self.surface, &mut self.surface_config) {
            config.width = width.max(1);
            config.height = height.max(1);
            surface.configure(&self.device, config);
            log::debug!("Surface resized to {}x{}", width, height);
        }
    }

    /// Upload a frame to GPU textures
    pub fn upload_frame(&mut self, frame: &RenderFrame) -> Result<(), RendererError> {
        match frame.format {
            FrameFormat::BGRA => self.upload_bgra_frame(frame),
            FrameFormat::YUV420 => self.upload_yuv_frame(frame),
        }
    }

    fn upload_bgra_frame(&mut self, frame: &RenderFrame) -> Result<(), RendererError> {
        // Recreate texture if dimensions changed
        if self.frame_width != frame.width || self.frame_height != frame.height {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("BGRA Frame Texture"),
                size: wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("BGRA Bind Group"),
                layout: &self.bgra_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

            self.bgra_texture = Some(texture);
            self.bgra_bind_group = Some(bind_group);
            self.frame_width = frame.width;
            self.frame_height = frame.height;
        }

        // Upload texture data
        if let Some(ref texture) = self.bgra_texture {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(frame.width * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
            );
        }

        Ok(())
    }

    fn upload_yuv_frame(&mut self, frame: &RenderFrame) -> Result<(), RendererError> {
        let strides = frame
            .strides
            .ok_or_else(|| RendererError::RenderError("YUV frame missing strides".to_string()))?;

        let uv_width = (frame.width + 1) / 2;
        let uv_height = (frame.height + 1) / 2;

        // Recreate textures if dimensions changed
        if self.frame_width != frame.width || self.frame_height != frame.height {
            let y_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Y Texture"),
                size: wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let u_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("U Texture"),
                size: wgpu::Extent3d {
                    width: uv_width,
                    height: uv_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let v_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("V Texture"),
                size: wgpu::Extent3d {
                    width: uv_width,
                    height: uv_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let u_view = u_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let v_view = v_texture.create_view(&wgpu::TextureViewDescriptor::default());

            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("YUV Bind Group"),
                layout: &self.yuv_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&y_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&u_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&v_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

            self.yuv_textures = Some((y_texture, u_texture, v_texture));
            self.yuv_bind_group = Some(bind_group);
            self.frame_width = frame.width;
            self.frame_height = frame.height;
        }

        // Upload texture data
        if let Some((ref y_tex, ref u_tex, ref v_tex)) = self.yuv_textures {
            let y_size = strides[0] * frame.height as usize;
            let u_size = strides[1] * uv_height as usize;

            // Y plane
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: y_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.data[..y_size],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(strides[0] as u32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
            );

            // U plane
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: u_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.data[y_size..y_size + u_size],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(strides[1] as u32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: uv_width,
                    height: uv_height,
                    depth_or_array_layers: 1,
                },
            );

            // V plane
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: v_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.data[y_size + u_size..],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(strides[2] as u32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: uv_width,
                    height: uv_height,
                    depth_or_array_layers: 1,
                },
            );
        }

        Ok(())
    }

    /// Render the current frame to the surface
    pub fn render(&mut self, format: FrameFormat) -> Result<(), RendererError> {
        let surface = self
            .surface
            .as_ref()
            .ok_or_else(|| RendererError::RenderError("No surface configured".to_string()))?;

        let output = surface
            .get_current_texture()
            .map_err(|e| RendererError::RenderError(format!("Failed to get surface texture: {}", e)))?;

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Set viewport to maintain video aspect ratio (letterbox/pillarbox)
            if let Some(ref config) = self.surface_config {
                if self.frame_width > 0 && self.frame_height > 0 {
                    let surface_w = config.width as f32;
                    let surface_h = config.height as f32;
                    let frame_aspect = self.frame_width as f32 / self.frame_height as f32;
                    let surface_aspect = surface_w / surface_h;

                    let (vp_x, vp_y, vp_w, vp_h) = if frame_aspect > surface_aspect {
                        // Video wider than window - fit width, letterbox top/bottom
                        let h = surface_w / frame_aspect;
                        (0.0, (surface_h - h) / 2.0, surface_w, h)
                    } else {
                        // Video taller than window - fit height, pillarbox left/right
                        let w = surface_h * frame_aspect;
                        ((surface_w - w) / 2.0, 0.0, w, surface_h)
                    };

                    render_pass.set_viewport(vp_x, vp_y, vp_w, vp_h, 0.0, 1.0);
                }
            }

            match format {
                FrameFormat::BGRA => {
                    if let Some(ref bind_group) = self.bgra_bind_group {
                        render_pass.set_pipeline(&self.bgra_pipeline);
                        render_pass.set_bind_group(0, bind_group, &[]);
                        render_pass.draw(0..6, 0..1);
                    }
                }
                FrameFormat::YUV420 => {
                    if let Some(ref bind_group) = self.yuv_bind_group {
                        render_pass.set_pipeline(&self.yuv_pipeline);
                        render_pass.set_bind_group(0, bind_group, &[]);
                        render_pass.draw(0..6, 0..1);
                    }
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    /// Get device and queue for external use
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}

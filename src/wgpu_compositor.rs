// GPU Compositor - Native wgpu + WGSL shader for real-time alpha blending
use wgpu::{self as wgpu, util::DeviceExt};
use pollster::block_on;
use bytemuck::{Pod, Zeroable};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CompositorError {
    #[error("Failed to create WGPU instance")]
    InstanceCreation,
    #[error("Failed to create WGPU adapter")]
    AdapterCreation,
    #[error("Failed to create WGPU device")]
    DeviceCreation,
    #[error("Failed to create WGPU queue")]
    QueueCreation,
    #[error("Invalid texture dimensions")]
    InvalidDimensions,
    #[error("Texture update failed")]
    TextureUpdateFailed,
    #[error("Render failed")]
    RenderFailed,
    #[error("Buffer async error: {0}")]
    BufferAsync(#[from] wgpu::BufferAsyncError),
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
        0 => Float32x2, // position
        1 => Float32x2, // tex_coords
    ];

    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

const VERTICES: &[Vertex] = &[
    Vertex { position: [-1.0, 1.0], tex_coords: [0.0, 0.0] },  // top-left
    Vertex { position: [1.0, 1.0], tex_coords: [1.0, 0.0] },   // top-right
    Vertex { position: [1.0, -1.0], tex_coords: [1.0, 1.0] },  // bottom-right
    Vertex { position: [-1.0, -1.0], tex_coords: [0.0, 1.0] }, // bottom-left
];

const INDICES: &[u16] = &[0, 1, 2, 2, 3, 0];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FxParams {
    fx_rect: [f32; 4], // x, y, width, height (normalized 0-1)
    fx_alpha: f32,
    _padding: [f32; 3],
}

pub struct WgpuCompositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    output_texture: wgpu::Texture,
    output_texture_view: wgpu::TextureView,
    camera_texture: wgpu::Texture,
    camera_texture_view: wgpu::TextureView,
    fx_texture: wgpu::Texture,
    fx_texture_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    fx_params_buffer: wgpu::Buffer,
    output_buffer: wgpu::Buffer, // Reusable output buffer
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    output_width: u32,
    output_height: u32,
    camera_width: u32,
    camera_height: u32,
    fx_width: u32,
    fx_height: u32,
    fx_params: FxParams,
    buffer_mapped: bool, // Track if buffer is currently mapped
}

impl WgpuCompositor {
    pub fn new(output_width: u32, output_height: u32) -> Result<Self, CompositorError> {
        println!("[WGPU] Initializing GPU compositor {}x{}", output_width, output_height);

        // Create WGPU instance
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // Get adapter
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|_| CompositorError::AdapterCreation)?;

        // Get device and queue
        let (device, queue) = block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                label: Some("WGPU Compositor Device"),
                memory_hints: Default::default(),
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            }
        ))
        .map_err(|_| CompositorError::DeviceCreation)?;

        // Create output texture (RGBA8)
        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Output Texture"),
            size: wgpu::Extent3d {
                width: output_width,
                height: output_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let output_texture_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create camera texture (RGBA8)
        let camera_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Camera Texture"),
            size: wgpu::Extent3d {
                width: output_width,
                height: output_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let camera_texture_view = camera_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create FX texture (RGBA8)
        let fx_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("FX Texture"),
            size: wgpu::Extent3d {
                width: output_width,
                height: output_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let fx_texture_view = fx_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Create vertex buffer
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Create index buffer
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Create output buffer (reusable)
        let output_buffer_size = (output_width * output_height * 4) as u64;
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Output Buffer"),
            size: output_buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // Create FX params buffer
        let fx_params = FxParams {
            fx_rect: [0.0, 0.0, 1.0, 1.0], // full screen by default
            fx_alpha: 1.0,
            _padding: [0.0; 3],
        };

        let fx_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("FX Params Buffer"),
            contents: bytemuck::bytes_of(&fx_params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Compositor Bind Group Layout"),
            entries: &[
                // Camera texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // Camera sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // FX texture
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // FX sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // FX params
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Compositor Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&camera_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&fx_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: fx_params_buffer.as_entire_binding(),
                },
            ],
        });

        // Create shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Compositor Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("compositor.wgsl").into()),
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Compositor Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create render pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Compositor Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        println!("[WGPU] ✅ GPU compositor initialized successfully");

        Ok(Self {
            device,
            queue,
            output_texture,
            output_texture_view,
            camera_texture,
            camera_texture_view,
            fx_texture,
            fx_texture_view,
            sampler,
            vertex_buffer,
            index_buffer,
            fx_params_buffer,
            output_buffer,
            bind_group_layout,
            bind_group,
            pipeline,
            output_width,
            output_height,
            camera_width: output_width,
            camera_height: output_height,
            fx_width: output_width,
            fx_height: output_height,
            fx_params,
            buffer_mapped: false,
        })
    }

    pub fn set_fx_params(&mut self, x: f32, y: f32, width: f32, height: f32, alpha: f32) {
        // Convert pixel coordinates to normalized (0-1)
        let norm_x = x / self.output_width as f32;
        let norm_y = y / self.output_height as f32;
        let norm_width = width / self.output_width as f32;
        let norm_height = height / self.output_height as f32;

        self.fx_params.fx_rect = [norm_x, norm_y, norm_width, norm_height];
        self.fx_params.fx_alpha = alpha.clamp(0.0, 1.0);

        // Update uniform buffer
        self.queue.write_buffer(&self.fx_params_buffer, 0, bytemuck::bytes_of(&self.fx_params));
    }

    pub fn update_camera_rgba(&mut self, width: u32, height: u32, _stride: u32, data: &[u8]) -> Result<(), CompositorError> {
        if width == 0 || height == 0 || data.is_empty() {
            return Err(CompositorError::InvalidDimensions);
        }

        // Store dimensions
        self.camera_width = width;
        self.camera_height = height;

        // Calculate expected size (RGBA = 4 bytes per pixel)
        let expected_size = (width * height * 4) as usize;
        if data.len() != expected_size {
            return Err(CompositorError::InvalidDimensions);
        }

        // Update texture
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.camera_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        Ok(())
    }

    pub fn update_fx_rgba(&mut self, width: u32, height: u32, _stride: u32, data: &[u8]) -> Result<(), CompositorError> {
        if width == 0 || height == 0 || data.is_empty() {
            return Err(CompositorError::InvalidDimensions);
        }

        // Store dimensions
        self.fx_width = width;
        self.fx_height = height;

        // Calculate expected size (RGBA = 4 bytes per pixel)
        let expected_size = (width * height * 4) as usize;
        if data.len() != expected_size {
            return Err(CompositorError::InvalidDimensions);
        }

        // Update texture
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.fx_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        Ok(())
    }

    pub fn render_rgba(&mut self) -> Result<Vec<u8>, CompositorError> {
        // Don't render if buffer is still mapped from previous call
        if self.buffer_mapped {
            return Err(CompositorError::RenderFailed);
        }

        // Create command encoder
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Compositor Render Encoder"),
        });

        // Render pass
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Compositor Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output_texture_view,
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
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..INDICES.len() as u32, 0, 0..1);
        }

        // Copy output texture to buffer
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.output_width * 4),
                    rows_per_image: Some(self.output_height),
                },
            },
            wgpu::Extent3d {
                width: self.output_width,
                height: self.output_height,
                depth_or_array_layers: 1,
            },
        );

        // Submit commands
        self.queue.submit(Some(encoder.finish()));

        // Read back the data
        let buffer_slice = self.output_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();

        self.buffer_mapped = true;
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });

        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });

        receiver.recv().map_err(|_| CompositorError::RenderFailed)??;

        // Copy data to Vec<u8>
        let data = buffer_slice.get_mapped_range();
        let result = data.to_vec();

        // Unmap buffer
        self.output_buffer.unmap();
        self.buffer_mapped = false;

        Ok(result)
    }
}

impl Drop for WgpuCompositor {
    fn drop(&mut self) {
        // Ensure buffer is unmapped before dropping
        if self.buffer_mapped {
            self.output_buffer.unmap();
        }
        // WGPU resources will be automatically cleaned up by the Drop implementations
        // of Device, Queue, Texture, Buffer, etc.
    }
}

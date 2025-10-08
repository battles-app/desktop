// WGPU Surface-Based Renderer (Direct to Window - OPTIMAL)
// This replaces the readback-based approach with direct surface rendering

use wgpu::*;
use wgpu::util::DeviceExt;
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

const VERTICES: &[Vertex] = &[
    Vertex { position: [-1.0, -1.0], tex_coords: [0.0, 1.0] },
    Vertex { position: [ 1.0, -1.0], tex_coords: [1.0, 1.0] },
    Vertex { position: [ 1.0,  1.0], tex_coords: [1.0, 0.0] },
    Vertex { position: [-1.0,  1.0], tex_coords: [0.0, 0.0] },
];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    key_color: [f32; 3],
    _padding1: f32,
    tolerance: f32,
    similarity: f32,
    use_chroma_key: f32,
    _padding2: f32,
}

pub struct WgpuSurfaceRenderer {
    _window: Arc<tauri::WebviewWindow>, // Keep window alive for surface lifetime
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    render_pipeline: RenderPipeline,
    vertex_buffer: Buffer,
    bind_group_layout: BindGroupLayout,
    sampler: Sampler,
    current_bind_group: Option<BindGroup>,
    current_texture: Option<Texture>,
    current_texture_view: Option<TextureView>,
    uniforms_buffer: Buffer,
    current_uniforms: Uniforms,
    frame_count: u64,
}

impl WgpuSurfaceRenderer {
    pub async fn new(
        window: Arc<tauri::Window>,
        width: u32,
        height: u32
    ) -> Result<Self, String> {
        println!("[WGPU Surface] 🚀 Initializing direct surface renderer ({}x{})", width, height);

        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::PRIMARY,
            ..Default::default()
        });

        // Create surface from Tauri window  
        // SAFETY: We store the Arc<Window> in the struct to ensure it lives as long as the surface
        let surface = unsafe {
            instance.create_surface_unsafe(
                wgpu::SurfaceTargetUnsafe::from_window(window.as_ref())
                    .map_err(|e| format!("Failed to get surface target: {}", e))?
            ).map_err(|e| format!("Failed to create surface: {}", e))?
        };

        let adapter = match instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await {
                Ok(adapter) => adapter,
                Err(e) => return Err(format!("Failed to find suitable GPU adapter: {:?}", e)),
            };

        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                label: Some("WGPU Surface Device"),
                required_features: Features::empty(),
                required_limits: Limits::default(),
                memory_hints: MemoryHints::Performance,
                experimental_features: Default::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| format!("Failed to create device: {}", e))?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: PresentMode::Fifo, // VSync for smooth rendering
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &config);

        // Chroma key shader (same as before)
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Chroma Key Shader"),
            source: ShaderSource::Wgsl(include_str!("chroma_key_shader.wgsl").into()),
        });

        // Create vertex buffer
        let vertex_buffer = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: BufferUsages::VERTEX,
        });

        // Create uniform buffer
        let uniforms_buffer = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("Uniforms Buffer"),
            contents: bytemuck::cast_slice(&[Uniforms {
                key_color: [0.0, 1.0, 0.0],
                _padding1: 0.0,
                tolerance: 0.1,
                similarity: 0.1,
                use_chroma_key: 0.0,
                _padding2: 0.0,
            }]),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });

        // Sampler
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("Texture Sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Nearest,
            ..Default::default()
        });

        // Bind group layout
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Bind Group Layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Render pipeline
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as BufferAddress,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format: config.format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        println!("[WGPU Surface] ✅ Direct surface renderer initialized");

        Ok(Self {
            _window: window,
            surface,
            device,
            queue,
            config,
            render_pipeline,
            vertex_buffer,
            bind_group_layout,
            sampler,
            current_bind_group: None,
            current_texture: None,
            current_texture_view: None,
            uniforms_buffer,
            current_uniforms: Uniforms {
                key_color: [0.0, 1.0, 0.0],
                _padding1: 0.0,
                tolerance: 0.1,
                similarity: 0.1,
                use_chroma_key: 0.0,
                _padding2: 0.0,
            },
            frame_count: 0,
        })
    }

    pub fn set_chroma_key_params(&mut self, key_color: [f32; 3], tolerance: f32, similarity: f32, use_chroma_key: bool) {
        self.current_uniforms = Uniforms {
            key_color,
            _padding1: 0.0,
            tolerance,
            similarity,
            use_chroma_key: if use_chroma_key { 1.0 } else { 0.0 },
            _padding2: 0.0,
        };

        self.queue.write_buffer(&self.uniforms_buffer, 0, bytemuck::cast_slice(&[self.current_uniforms]));
    }

    pub fn update_texture_from_rgba(&mut self, rgba_data: &[u8], width: u32, height: u32) -> Result<(), String> {
        // Create texture if dimensions changed or first time
        if self.current_texture.is_none() || 
           self.current_texture.as_ref().unwrap().width() != width ||
           self.current_texture.as_ref().unwrap().height() != height {
            
            let texture = self.device.create_texture(&TextureDescriptor {
                label: Some("Camera Texture"),
                size: Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });

            let texture_view = texture.create_view(&TextureViewDescriptor::default());

            // Create bind group
            let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
                label: Some("Texture Bind Group"),
                layout: &self.bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&texture_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: self.uniforms_buffer.as_entire_binding(),
                    },
                ],
            });

            self.current_texture = Some(texture);
            self.current_texture_view = Some(texture_view);
            self.current_bind_group = Some(bind_group);
        }

        // Upload RGBA data to GPU
        if let Some(texture) = &self.current_texture {
            self.queue.write_texture(
                TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                rgba_data,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                Extent3d { width, height, depth_or_array_layers: 1 },
            );
        }

        Ok(())
    }

    pub fn render_to_surface(&mut self) -> Result<(), String> {
        // Get next frame from surface
        let output = self.surface.get_current_texture()
            .map_err(|e| format!("Failed to get surface texture: {}", e))?;

        let view = output.texture.create_view(&TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLACK),
                        store: StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            if let Some(bind_group) = &self.current_bind_group {
                render_pass.set_pipeline(&self.render_pipeline);
                render_pass.set_bind_group(0, bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.draw(0..4, 0..1);
            }
        }

        self.queue.submit([encoder.finish()]);
        output.present(); // CRITICAL: Present to display immediately!

        self.frame_count += 1;
        Ok(())
    }

    pub fn resize(&mut self, new_width: u32, new_height: u32) {
        if new_width > 0 && new_height > 0 {
            self.config.width = new_width;
            self.config.height = new_height;
            self.surface.configure(&self.device, &self.config);
        }
    }
}


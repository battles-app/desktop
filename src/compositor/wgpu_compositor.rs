use crate::compositor::layer::Layer;
use glam::{Vec2, Mat4};
use std::collections::HashMap;
use wgpu::{self, util::DeviceExt};
use winit::window::Window;

/// Vertex data for a quad
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];

    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Instance data for each layer
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    transform: [[f32; 4]; 4], // 4x4 transformation matrix
    opacity: f32,
    chroma_key: [f32; 3],
    chroma_tolerance: f32,
    layer_index: u32,
}

impl Instance {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        static ATTRIBS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
            2 => Float32x4,
            3 => Float32x4,
            4 => Float32x4,
            5 => Float32x4,
            6 => Float32,
            7 => Float32x3,
            8 => Float32,
            9 => Uint32
        ];
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Instance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRIBS,
        }
    }
}

/// Main WGPU compositor that handles GPU-accelerated compositing
pub struct WgpuCompositor {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: Option<wgpu::Surface<'static>>,
    surface_config: Option<wgpu::SurfaceConfiguration>,

    // Render pipeline
    render_pipeline: wgpu::RenderPipeline,

    // Vertex and index buffers for quad
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,

    // Texture array for layer textures
    texture_bind_group_layout: wgpu::BindGroupLayout,
    texture_bind_group: Option<wgpu::BindGroup>,
    texture_array: Vec<wgpu::Texture>,

    // Layers
    layers: HashMap<String, Layer>,
    sorted_layer_ids: Vec<String>,

    // Output dimensions
    output_width: u32,
    output_height: u32,

    // Frame timing
    target_fps: u32,
    frame_count: u64,
}

impl WgpuCompositor {
    /// Create a new WGPU compositor with offscreen rendering
    pub async fn new_offscreen(width: u32, height: u32, target_fps: u32) -> Result<Self, String> {
        println!("[WGPU] Creating offscreen compositor: {}x{} @ {}fps", width, height, target_fps);

        // Create WGPU instance
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Request adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| "Failed to find WGPU adapter")?;

        println!("[WGPU] Using adapter: {:?}", adapter.get_info());

        // Request device
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("WGPU Compositor Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits {
                        max_texture_array_layers: 1,
                        max_binding_array_elements_per_shader_stage: 1,
                        ..wgpu::Limits::default()
                    },
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::default(),
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                },
            )
            .await
            .map_err(|e| format!("Failed to create WGPU device: {}", e))?;

        // Create vertex and index buffers for a quad
        let vertex_buffer = Self::create_vertex_buffer(&device);
        let index_buffer = Self::create_index_buffer(&device);
        let instance_buffer = Self::create_instance_buffer(&device, 4); // Support up to 4 layers initially

        // Create texture bind group layout for texture array
        let texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Texture Array Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: std::num::NonZeroU32::new(1), // Single texture for now
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Create render pipeline
        let render_pipeline = Self::create_render_pipeline(&device, &texture_bind_group_layout)?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface: None,
            surface_config: None,
            render_pipeline,
            vertex_buffer,
            index_buffer,
            instance_buffer,
            texture_bind_group_layout,
            texture_bind_group: None,
            texture_array: Vec::new(),
            layers: HashMap::new(),
            sorted_layer_ids: Vec::new(),
            output_width: width,
            output_height: height,
            target_fps,
            frame_count: 0,
        })
    }

    /// Create a new WGPU compositor with window surface for preview
    pub async fn new_with_window(_window: &Window, width: u32, height: u32, target_fps: u32) -> Result<Self, String> {
        println!("[WGPU] Creating window compositor: {}x{} @ {}fps (offscreen only for now)", width, height, target_fps);

        // For now, just create offscreen compositor
        // Window surface support can be added later if needed
        Self::new_offscreen(width, height, target_fps).await
    }

    /// Create vertex buffer for quad
    fn create_vertex_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        let vertices = [
            Vertex { position: [-1.0, -1.0], tex_coords: [0.0, 1.0] }, // Bottom-left
            Vertex { position: [ 1.0, -1.0], tex_coords: [1.0, 1.0] }, // Bottom-right
            Vertex { position: [ 1.0,  1.0], tex_coords: [1.0, 0.0] }, // Top-right
            Vertex { position: [-1.0,  1.0], tex_coords: [0.0, 0.0] }, // Top-left
        ];

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    /// Create index buffer for quad
    fn create_index_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        let indices: [u16; 6] = [0, 1, 2, 2, 3, 0];

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        })
    }

    /// Create instance buffer
    fn create_instance_buffer(device: &wgpu::Device, max_instances: u32) -> wgpu::Buffer {
        let instance_size = std::mem::size_of::<Instance>() as u64;
        let buffer_size = instance_size * max_instances as u64;

        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Instance Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Create render pipeline
    fn create_render_pipeline(
        device: &wgpu::Device,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Result<wgpu::RenderPipeline, String> {
        // Shader source for compositing
        let shader_source = r#"
// Vertex shader for compositing layers
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) transform_0: vec4<f32>,
    @location(3) transform_1: vec4<f32>,
    @location(4) transform_2: vec4<f32>,
    @location(5) transform_3: vec4<f32>,
    @location(6) opacity: f32,
    @location(7) chroma_key: vec3<f32>,
    @location(8) chroma_tolerance: f32,
    @location(9) layer_index: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) opacity: f32,
    @location(2) chroma_key: vec3<f32>,
    @location(3) chroma_tolerance: f32,
    @location(4) layer_index: u32,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    // Reconstruct transformation matrix
    let transform = mat4x4<f32>(
        input.transform_0,
        input.transform_1,
        input.transform_2,
        input.transform_3
    );

    // Transform vertex position
    let transformed_pos = transform * vec4<f32>(input.position, 0.0, 1.0);

    var output: VertexOutput;
    output.position = vec4<f32>(transformed_pos.xy, 0.0, 1.0);
    output.tex_coords = input.tex_coords;
    output.opacity = input.opacity;
    output.chroma_key = input.chroma_key;
    output.chroma_tolerance = input.chroma_tolerance;
    output.layer_index = input.layer_index;

    return output;
}

// Fragment shader for compositing with optional chroma keying
@group(0) @binding(0)
var texture_array: binding_array<texture_2d<f32>>;
@group(0) @binding(1)
var texture_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Sample texture from array
    let tex_color = textureSample(texture_array[input.layer_index], texture_sampler, input.tex_coords);

    // Apply chroma keying if chroma key is set (non-zero)
    if (length(input.chroma_key) > 0.0) {
        let diff = abs(tex_color.rgb - input.chroma_key);
        let key_mask = 1.0 - smoothstep(0.0, input.chroma_tolerance, max(max(diff.r, diff.g), diff.b));
        let keyed_color = mix(tex_color, vec4<f32>(0.0, 0.0, 0.0, 0.0), key_mask);

        // Apply opacity
        return vec4<f32>(keyed_color.rgb, keyed_color.a * input.opacity);
    } else {
        // No chroma keying, just apply opacity
        return vec4<f32>(tex_color.rgb, tex_color.a * input.opacity);
    }
}
"#;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Compositor Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Compositor Pipeline Layout"),
            bind_group_layouts: &[texture_bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Compositor Render Pipeline"),
            layout: Some(&pipeline_layout),
            cache: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc(), Instance::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Ok(render_pipeline)
    }

    /// Add a layer to the compositor
    pub fn add_layer(&mut self, layer: Layer) {
        println!("[WGPU] Adding layer: {}", layer.id);
        self.layers.insert(layer.id.clone(), layer);
        self.update_sorted_layers();
        self.update_texture_array();
    }

    /// Remove a layer from the compositor
    pub fn remove_layer(&mut self, layer_id: &str) -> bool {
        if self.layers.remove(layer_id).is_some() {
            println!("[WGPU] Removed layer: {}", layer_id);
            self.update_sorted_layers();
            self.update_texture_array();
            true
        } else {
            false
        }
    }

    /// Get a mutable reference to a layer
    pub fn get_layer_mut(&mut self, layer_id: &str) -> Option<&mut Layer> {
        self.layers.get_mut(layer_id)
    }

    /// Set transform for a layer
    pub fn set_transform(&mut self, layer_id: &str, position: Vec2, scale: Vec2, rotation: f32) {
        if let Some(layer) = self.layers.get_mut(layer_id) {
            layer.position = position;
            layer.scale = scale;
            layer.rotation = rotation;
        }
    }

    /// Update sorted layer list by z-order
    fn update_sorted_layers(&mut self) {
        let mut layer_vec: Vec<_> = self.layers.values().collect();
        layer_vec.sort_by(|a, b| a.z_order.cmp(&b.z_order));
        self.sorted_layer_ids = layer_vec.into_iter().map(|l| l.id.clone()).collect();
    }

    /// Update texture array and bind group
    fn update_texture_array(&mut self) {
        // Create texture array views
        let texture_views: Vec<_> = self.texture_array.iter().map(|tex| {
            tex.create_view(&wgpu::TextureViewDescriptor::default())
        }).collect();

        // Create sampler
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Create bind group
        let texture_views: Vec<wgpu::TextureView> = self.texture_array.iter().map(|tex| {
            tex.create_view(&wgpu::TextureViewDescriptor::default())
        }).collect();

        self.texture_bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Texture Array Bind Group"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureViewArray(
                        texture_views.iter().collect::<Vec<_>>().as_slice()
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        }));
    }

    /// Create texture from RGBA bytes
    pub fn create_texture_from_rgba(&self, width: u32, height: u32, rgba_data: &[u8]) -> wgpu::Texture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Layer Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        texture
    }

    /// Render a single frame to the output texture
    pub fn render_frame(&mut self, output_texture: &wgpu::Texture) -> Result<(), String> {
        // Prepare instance data
        let mut instances = Vec::new();

        for layer_id in &self.sorted_layer_ids {
            if let Some(layer) = self.layers.get(layer_id) {
                if !layer.visible || layer.texture.is_none() {
                    continue;
                }

                let transform = layer.transform_matrix(self.output_width as f32, self.output_height as f32);
                let chroma_key = layer.chroma_key.unwrap_or([0.0, 0.0, 0.0]);

                // Find texture index
                let texture_index = self.texture_array.iter().position(|tex| {
                    // Compare texture IDs (simplified - in real implementation you'd store texture IDs)
                    true // Placeholder - need proper texture tracking
                }).unwrap_or(0) as u32;

                instances.push(Instance {
                    transform: transform.to_cols_array_2d(),
                    opacity: layer.opacity,
                    chroma_key,
                    chroma_tolerance: layer.chroma_tolerance,
                    layer_index: texture_index,
                });
            }
        }

        if instances.is_empty() {
            return Ok(()); // Nothing to render
        }

        // Update instance buffer
        self.queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));

        // Create command encoder
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Compositor Render Encoder"),
        });

        // Create render pass
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Compositor Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &output_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        // Set pipeline and bind groups
        render_pass.set_pipeline(&self.render_pipeline);
        if let Some(bind_group) = &self.texture_bind_group {
            render_pass.set_bind_group(0, bind_group, &[]);
        }

        // Set vertex buffers
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

        // Draw instances
        render_pass.draw_indexed(0..6, 0, 0..instances.len() as u32);

        drop(render_pass);

        // Submit commands
        self.queue.submit(std::iter::once(encoder.finish()));

        self.frame_count += 1;
        Ok(())
    }

    /// Render to surface if available (for preview window)
    pub fn render_to_surface(&mut self) -> Result<(), String> {
        if let (Some(surface), Some(config)) = (&self.surface, &self.surface_config) {
            let output = surface.get_current_texture()
                .map_err(|e| format!("Failed to get surface texture: {:?}", e))?;

            let output_texture = &output.texture;
            self.render_frame(output_texture)?;

            output.present();
        }

        Ok(())
    }

    /// Create output texture for offscreen rendering
    pub fn create_output_texture(&self) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Compositor Output Texture"),
            size: wgpu::Extent3d {
                width: self.output_width,
                height: self.output_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Read output texture back to CPU memory as RGBA bytes
    pub fn read_output_texture(&self, texture: &wgpu::Texture) -> Result<Vec<u8>, String> {
        let buffer_size = (self.output_width * self.output_height * 4) as u64;

        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Output Read Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Readback Encoder"),
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * self.output_width),
                    rows_per_image: Some(self.output_height),
                },
            },
            wgpu::Extent3d {
                width: self.output_width,
                height: self.output_height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::once(encoder.finish()));

        // Map buffer for reading
        let buffer_slice = output_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();

        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });

        self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        }).unwrap();

        rx.recv().unwrap()
            .map_err(|e| format!("Failed to map buffer: {:?}", e))?;

        let data = buffer_slice.get_mapped_range();
        let result = data.to_vec();

        drop(data);
        output_buffer.unmap();

        Ok(result)
    }

    /// Get output dimensions
    pub fn output_size(&self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }

    /// Get target FPS
    pub fn target_fps(&self) -> u32 {
        self.target_fps
    }
}

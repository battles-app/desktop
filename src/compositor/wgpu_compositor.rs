use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use glam::Vec2;
use tokio::sync::broadcast;
use wgpu::{
    util::DeviceExt, Backends, BindGroup, BindGroupLayout, Buffer, Device, Queue,
    RenderPipeline, TextureFormat, TextureView,
};

use crate::compositor::layer::{Layer, LayerInstanceRaw};

/// Maximum number of layers that can be composited at once
const MAX_LAYERS: usize = 16;

/// Shader code for the compositor
const SHADER_SOURCE: &str = r#"
// Vertex shader
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) opacity: f32,
    @location(2) @interpolate(flat) instance_index: u32,
};

struct LayerInstance {
    @location(5) transform_0: vec4<f32>,
    @location(6) transform_1: vec4<f32>,
    @location(7) transform_2: vec4<f32>,
    @location(8) transform_3: vec4<f32>,
    @location(9) opacity: f32,
};

@vertex
fn vs_main(
    vertex: VertexInput,
    @builtin(instance_index) instance_index: u32,
    instance: LayerInstance,
) -> VertexOutput {
    var out: VertexOutput;
    
    // Reconstruct the transform matrix
    var transform = mat4x4<f32>(
        instance.transform_0,
        instance.transform_1,
        instance.transform_2,
        instance.transform_3
    );
    
    // Apply the transform
    out.clip_position = transform * vec4<f32>(vertex.position, 0.0, 1.0);
    out.tex_coords = vertex.tex_coords;
    out.opacity = instance.opacity;
    out.instance_index = instance_index;
    
    return out;
}

// Fragment shader
@group(0) @binding(0) var t_diffuse: texture_2d_array<f32>;
@group(0) @binding(1) var s_diffuse: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(t_diffuse, s_diffuse, in.tex_coords, in.instance_index);
    color.a = color.a * in.opacity;
    return color;
}
"#;

/// Vertex data for a quad (two triangles)
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

impl Vertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

/// Instance data layout for the GPU
fn instance_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<LayerInstanceRaw>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            // Transform matrix - first row
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 5,
                format: wgpu::VertexFormat::Float32x4,
            },
            // Transform matrix - second row
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                shader_location: 6,
                format: wgpu::VertexFormat::Float32x4,
            },
            // Transform matrix - third row
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                shader_location: 7,
                format: wgpu::VertexFormat::Float32x4,
            },
            // Transform matrix - fourth row
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 12]>() as wgpu::BufferAddress,
                shader_location: 8,
                format: wgpu::VertexFormat::Float32x4,
            },
            // Opacity
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 16]>() as wgpu::BufferAddress,
                shader_location: 9,
                format: wgpu::VertexFormat::Float32,
            },
        ],
    }
}

/// The main compositor struct
pub struct WgpuCompositor {
    /// The wgpu device
    device: Device,
    
    /// The wgpu queue
    queue: Queue,
    
    /// The render pipeline
    render_pipeline: RenderPipeline,
    
    /// The vertex buffer for the quad
    vertex_buffer: Buffer,
    
    /// The index buffer for the quad
    index_buffer: Buffer,
    
    /// The instance buffer for layer transforms
    instance_buffer: Buffer,
    
    /// The bind group layout for textures
    bind_group_layout: BindGroupLayout,
    
    /// The bind group for textures
    bind_group: Option<BindGroup>,
    
    /// The layers to composite
    layers: HashMap<String, Layer>,
    
    /// The output width
    width: u32,
    
    /// The output height
    height: u32,
    
    /// The output format
    format: TextureFormat,
    
    /// The output texture
    output_texture: Option<wgpu::Texture>,
    
    /// The output texture view
    output_view: Option<TextureView>,
    
    /// The frame sender
    frame_sender: Arc<Mutex<Option<broadcast::Sender<Vec<u8>>>>>,
    
    /// The frame clock
    frame_clock: Instant,
    
    /// The target frame rate
    target_fps: u32,
    
    /// The frame interval in nanoseconds
    frame_interval_ns: u64,
}

impl WgpuCompositor {
    /// Create a new compositor
    pub async fn new(width: u32, height: u32, target_fps: u32) -> Result<Self> {
        // Create the wgpu instance
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: Backends::all(),
            ..Default::default()
        });
        
        // Get the adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow!("Failed to find an appropriate adapter"))?;
        
        // Create the device and queue
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Compositor Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await?;
        
        // Create the shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Compositor Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        
        // Create the bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Compositor Bind Group Layout"),
            entries: &[
                // Texture array
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        
        // Create the pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Compositor Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        
        // Create the render pipeline
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Compositor Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[Vertex::desc(), instance_buffer_layout()],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
        });
        
        // Create the vertex buffer for a quad
        let vertices = [
            Vertex { position: [-1.0, -1.0], tex_coords: [0.0, 1.0] },
            Vertex { position: [1.0, -1.0], tex_coords: [1.0, 1.0] },
            Vertex { position: [1.0, 1.0], tex_coords: [1.0, 0.0] },
            Vertex { position: [-1.0, 1.0], tex_coords: [0.0, 0.0] },
        ];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        
        // Create the index buffer
        let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        
        // Create the instance buffer (initially empty)
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Instance Buffer"),
            size: std::mem::size_of::<LayerInstanceRaw>() as u64 * MAX_LAYERS as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        
        // Create the output texture
        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Output Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        // Calculate frame interval in nanoseconds
        let frame_interval_ns = 1_000_000_000 / target_fps as u64;
        
        Ok(Self {
            device,
            queue,
            render_pipeline,
            vertex_buffer,
            index_buffer,
            instance_buffer,
            bind_group_layout,
            bind_group: None,
            layers: HashMap::new(),
            width,
            height,
            format: TextureFormat::Rgba8Unorm,
            output_texture: Some(output_texture),
            output_view: Some(output_view),
            frame_sender: Arc::new(Mutex::new(None)),
            frame_clock: Instant::now(),
            target_fps,
            frame_interval_ns,
        })
    }
    
    /// Set the frame sender
    pub fn set_frame_sender(&mut self, sender: broadcast::Sender<Vec<u8>>) {
        *self.frame_sender.lock().unwrap() = Some(sender);
    }
    
    /// Add a layer to the compositor
    pub fn add_layer(&mut self, layer: Layer) -> Result<()> {
        if self.layers.len() >= MAX_LAYERS {
            return Err(anyhow!("Maximum number of layers reached"));
        }
        
        self.layers.insert(layer.id.clone(), layer);
        
        // Rebuild the texture array and bind group
        self.rebuild_bind_group()?;
        
        Ok(())
    }
    
    /// Remove a layer from the compositor
    pub fn remove_layer(&mut self, id: &str) -> Result<()> {
        if self.layers.remove(id).is_some() {
            // Rebuild the texture array and bind group
            self.rebuild_bind_group()?;
        }
        
        Ok(())
    }
    
    /// Update a layer's texture
    pub fn update_layer_texture(
        &mut self,
        id: &str,
        data: &[u8],
        width: u32,
        height: u32,
        pts: u64,
        duration: u64,
    ) -> Result<()> {
        if let Some(layer) = self.layers.get_mut(id) {
            layer.update_texture(
                &self.device,
                &self.queue,
                data,
                width,
                height,
                self.format,
            );
            layer.pts = pts;
            layer.duration = duration;
            
            // Rebuild the texture array and bind group
            self.rebuild_bind_group()?;
        }
        
        Ok(())
    }
    
    /// Set a layer's transform
    pub fn set_layer_transform(
        &mut self,
        id: &str,
        position: Vec2,
        scale: Vec2,
        rotation: f32,
        opacity: f32,
    ) -> Result<()> {
        if let Some(layer) = self.layers.get_mut(id) {
            layer.position = position;
            layer.scale = scale;
            layer.rotation = rotation;
            layer.opacity = opacity;
        }
        
        Ok(())
    }
    
    /// Set a layer's visibility
    pub fn set_layer_visibility(&mut self, id: &str, visible: bool) -> Result<()> {
        if let Some(layer) = self.layers.get_mut(id) {
            layer.visible = visible;
        }
        
        Ok(())
    }
    
    /// Set a layer's z-order
    pub fn set_layer_z_order(&mut self, id: &str, z_order: i32) -> Result<()> {
        if let Some(layer) = self.layers.get_mut(id) {
            layer.z_order = z_order;
        }
        
        Ok(())
    }
    
    /// Rebuild the texture array and bind group
    fn rebuild_bind_group(&mut self) -> Result<()> {
        // Skip if there are no layers
        if self.layers.is_empty() {
            self.bind_group = None;
            return Ok(());
        }
        
        // Create a texture array with all layer textures
        let texture_array = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Layer Texture Array"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: self.layers.len() as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        
        // Create a view into the texture array
        let texture_array_view = texture_array.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Layer Texture Array View"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        
        // Create a sampler
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Layer Texture Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        
        // Create the bind group
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Layer Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_array_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        
        self.bind_group = Some(bind_group);
        
        Ok(())
    }
    
    /// Render the composite frame
    pub fn render(&mut self, current_time_ns: u64) -> Result<Vec<u8>> {
        // Check if it's time to render a new frame
        let elapsed = self.frame_clock.elapsed();
        let elapsed_ns = elapsed.as_nanos() as u64;
        
        if elapsed_ns < self.frame_interval_ns {
            // Not time for a new frame yet
            return Err(anyhow!("Not time for a new frame yet"));
        }
        
        // Reset the frame clock
        self.frame_clock = Instant::now();
        
        // Sort layers by z-order
        let mut sorted_layers: Vec<&Layer> = self.layers.values().collect();
        sorted_layers.sort_by_key(|layer| layer.z_order);
        
        // Filter visible layers
        let visible_layers: Vec<&Layer> = sorted_layers
            .into_iter()
            .filter(|layer| layer.visible)
            .collect();
        
        // Skip rendering if there are no visible layers
        if visible_layers.is_empty() || self.bind_group.is_none() {
            return Err(anyhow!("No visible layers to render"));
        }
        
        // Create instance data for each visible layer
        let output_aspect_ratio = self.width as f32 / self.height as f32;
        let mut instance_data = Vec::with_capacity(visible_layers.len());
        
        for layer in &visible_layers {
            instance_data.push(layer.create_instance_data(output_aspect_ratio));
        }
        
        // Update the instance buffer
        self.queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&instance_data),
        );
        
        // Create a command encoder
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });
        
        // Begin the render pass
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.output_view.as_ref().unwrap(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            
            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, self.bind_group.as_ref().unwrap(), &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..6, 0, 0..visible_layers.len() as u32);
        }
        
        // Submit the command buffer
        self.queue.submit(std::iter::once(encoder.finish()));
        
        // Read back the rendered frame
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Output Buffer"),
            size: (self.width * self.height * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        
        // Copy the output texture to the buffer
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Copy Encoder"),
        });
        
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: self.output_texture.as_ref().unwrap(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &output_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(self.width * 4),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        
        self.queue.submit(std::iter::once(encoder.finish()));
        
        // Map the buffer and read the data
        let buffer_slice = output_buffer.slice(..);
        
        // Create a oneshot channel for the callback
        let (tx, rx) = std::sync::mpsc::channel();
        
        // Map the buffer asynchronously
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        
        // Wait for the buffer to be mapped
        self.device.poll(wgpu::Maintain::Wait);
        
        // Wait for the mapping operation to complete
        rx.recv().unwrap()?;
        
        // Get the mapped data
        let data = buffer_slice.get_mapped_range();
        let result = data.to_vec();
        
        // Unmap the buffer
        drop(data);
        output_buffer.unmap();
        
        // Send the frame to listeners
        if let Some(sender) = self.frame_sender.lock().unwrap().as_ref() {
            let _ = sender.send(result.clone());
        }
        
        Ok(result)
    }
    
    /// Start the compositor render loop
    pub async fn start_render_loop(&mut self) -> Result<()> {
        let mut interval = tokio::time::interval(Duration::from_nanos(self.frame_interval_ns));
        
        loop {
            interval.tick().await;
            
            let current_time_ns = self.frame_clock.elapsed().as_nanos() as u64;
            
            // Try to render a frame
            match self.render(current_time_ns) {
                Ok(_) => {
                    // Frame rendered successfully
                }
                Err(e) => {
                    // Skip frame if not ready or no layers
                    if !e.to_string().contains("Not time for a new frame yet")
                        && !e.to_string().contains("No visible layers to render")
                    {
                        log::error!("Error rendering frame: {}", e);
                    }
                }
            }
        }
    }
}

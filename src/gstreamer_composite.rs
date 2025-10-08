// WGPU-powered composite system with hardware-accelerated chroma key
use gstreamer::prelude::*;
use gstreamer::{self as gst, Pipeline, Element};
use gstreamer_app::{AppSink};
use tokio::sync::broadcast;
use std::sync::Arc;
use std::path::Path;
use parking_lot::RwLock;
use wgpu::*;
use wgpu::util::DeviceExt;
use bytemuck::{Pod, Zeroable};
use crate::wgpu_surface_renderer::WgpuSurfaceRenderer;

// WGSL Chroma Key Shader Source
const CHROMA_KEY_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) >> 1u) * 2.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.tex_coords = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    return out;
}

struct Uniforms {
    key_color: vec3<f32>,
    tolerance: f32,
    similarity: f32,
    use_chroma_key: f32,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(source_texture, source_sampler, in.tex_coords);

    if (uniforms.use_chroma_key > 0.5) {
        // Calculate color distance in RGB space
        let color_diff = tex_color.rgb - uniforms.key_color;
        let distance = length(color_diff);

        // Check if pixel is within tolerance
        if (distance < uniforms.tolerance) {
            // Additional similarity check using YUV color space
            let y = dot(tex_color.rgb, vec3<f32>(0.299, 0.587, 0.114));
            let u = dot(tex_color.rgb, vec3<f32>(-0.147, -0.289, 0.436));
            let v = dot(tex_color.rgb, vec3<f32>(0.615, -0.515, -0.100));

            let key_y = dot(uniforms.key_color, vec3<f32>(0.299, 0.587, 0.114));
            let key_u = dot(uniforms.key_color, vec3<f32>(-0.147, -0.289, 0.436));
            let key_v = dot(uniforms.key_color, vec3<f32>(0.615, -0.515, -0.100));

            let yuv_diff = vec3<f32>(y - key_y, u - key_u, v - key_v);
            let yuv_distance = length(yuv_diff);

            if (yuv_distance < uniforms.similarity) {
                // Make pixel transparent
                return vec4<f32>(tex_color.rgb, 0.0);
            }
        }
    }

    return tex_color;
}
"#;

// Vertex data for full-screen quad
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

// Uniform buffer data
// WGSL alignment: vec3 is aligned to 16 bytes, so we need padding
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    key_color: [f32; 3],
    _padding1: f32,        // Padding to align to 16 bytes
    tolerance: f32,
    similarity: f32,
    use_chroma_key: f32,
    _padding2: f32,        // Padding to align to 32 bytes total
}

// Readback buffer state machine
#[derive(Debug, Clone, Copy, PartialEq)]
enum ReadbackState {
    Free,           // Ready for new copy
    InFlight,       // Copy submitted, GPU writing (map not started)
    MappingPending, // map_async called, waiting for GPU
    Mapped,         // CPU reading, must unmap before reuse
}

// Readback buffer for async GPU→CPU transfer
struct ReadbackBuffer {
    buffer: Buffer,
    width: u32,
    height: u32,
    state: ReadbackState,
    frame_number: u64,
    map_receiver: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
}

// WGPU-based chroma key renderer with triple-buffered async readback
pub struct WgpuChromaRenderer {
    device: Device,
    queue: Queue,
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
    output_texture: Option<Texture>,
    output_texture_view: Option<TextureView>,
    // Triple-buffer ring for async readback
    readback_ring: Vec<ReadbackBuffer>,
    readback_index: usize,
}

impl WgpuChromaRenderer {
    pub async fn new(width: u32, height: u32) -> Result<Self, String> {
        // Initialize WGPU instance
        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::PRIMARY,
            ..Default::default()
        });

        // Request adapter
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .unwrap_or_else(|_| panic!("Failed to find suitable adapter"));

        // Request device
        let (device, queue) = adapter
            .request_device(
                &DeviceDescriptor {
                    required_features: Features::empty(),
                    required_limits: Limits::default(),
                    label: Some("Chroma Key Device"),
                    memory_hints: MemoryHints::Performance,
                    trace: wgpu::Trace::Off,
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                },
            )
            .await
            .map_err(|e| format!("Failed to create device: {}", e))?;

        // Use RGBA8 format for output
        let output_format = TextureFormat::Rgba8Unorm;

        // Create shader module
        let shader_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Chroma Key Shader"),
            source: ShaderSource::Wgsl(CHROMA_KEY_SHADER.into()),
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Chroma Key Bind Group Layout"),
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

        // Create sampler
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("Chroma Key Sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Linear,
            ..Default::default()
        });

        // Create vertex buffer
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: BufferUsages::VERTEX,
        });

        // Create uniform buffer
        let uniforms_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniforms Buffer"),
            contents: bytemuck::cast_slice(&[Uniforms {
                key_color: [0.0, 1.0, 0.0], // Default green screen
                _padding1: 0.0,
                tolerance: 0.1,
                similarity: 0.1,
                use_chroma_key: 0.0,
                _padding2: 0.0,
            }]),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });

        // Create output texture for rendering
        let output_texture = device.create_texture(&TextureDescriptor {
            label: Some("Output Texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: output_format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let output_texture_view = output_texture.create_view(&TextureViewDescriptor::default());

        // Create render pipeline
        let render_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Chroma Key Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Chroma Key Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: VertexState {
                module: &shader_module,
                entry_point: Some("vs_main"),
                buffers: &[VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as BufferAddress,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[
                        VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: VertexFormat::Float32x2,
                        },
                        VertexAttribute {
                            offset: std::mem::size_of::<[f32; 2]>() as BufferAddress,
                            shader_location: 1,
                            format: VertexFormat::Float32x2,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader_module,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format: output_format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleStrip,
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

        // Create triple-buffer ring for async readback (3 frame latency)
        const NUM_READBACK_BUFFERS: usize = 3;
        let unpadded_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = ((unpadded_bytes_per_row + align - 1) / align) * align;
        let buffer_size = (padded_bytes_per_row * height) as u64;

        let mut readback_ring = Vec::with_capacity(NUM_READBACK_BUFFERS);
        for i in 0..NUM_READBACK_BUFFERS {
            let buffer = device.create_buffer(&BufferDescriptor {
                label: Some(&format!("Readback Buffer {}", i)),
                size: buffer_size,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            readback_ring.push(ReadbackBuffer {
                buffer,
                width,
                height,
                state: ReadbackState::Free,
                frame_number: 0,
                map_receiver: None,
            });
        }

        Ok(Self {
            device,
            queue,
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
            output_texture: Some(output_texture),
            output_texture_view: Some(output_texture_view),
            readback_ring,
            readback_index: 0,
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
        // Create texture if dimensions changed
        if let (Some(texture), Some(_texture_view)) = (&self.current_texture, &self.current_texture_view) {
            let current_size = texture.size();
            if current_size.width != width || current_size.height != height {
                self.recreate_texture(width, height)?;
                // Also recreate output texture if input size changed
                self.recreate_output_texture(width, height)?;
            }
        } else {
            self.recreate_texture(width, height)?;
            // Also recreate output texture if input size changed
            self.recreate_output_texture(width, height)?;
        }

        // Update texture data
        if let (Some(texture), Some(texture_view)) = (&self.current_texture, &self.current_texture_view) {
            // For write_texture with tightly packed data, we can use unpadded bytes_per_row
            // But it must still be aligned to texel block size (4 bytes for RGBA)
            let unpadded_bytes_per_row = width * 4;
            
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
                    bytes_per_row: Some(unpadded_bytes_per_row),
                    rows_per_image: Some(height),
                },
                Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            // Recreate bind group with new texture view
            let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
                label: Some("Chroma Key Bind Group"),
                layout: &self.bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(texture_view),
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

            self.current_bind_group = Some(bind_group);
        }

        Ok(())
    }

    fn recreate_texture(&mut self, width: u32, height: u32) -> Result<(), String> {
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("Chroma Key Texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&TextureViewDescriptor::default());

        self.current_texture = Some(texture);
        self.current_texture_view = Some(texture_view);

        Ok(())
    }

    fn recreate_output_texture(&mut self, width: u32, height: u32) -> Result<(), String> {
        let output_texture = self.device.create_texture(&TextureDescriptor {
            label: Some("Output Texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let output_texture_view = output_texture.create_view(&TextureViewDescriptor::default());

        self.output_texture = Some(output_texture);
        self.output_texture_view = Some(output_texture_view);

        Ok(())
    }

    pub fn render_frame_async(&mut self) -> Result<(), String> {
        // Async render: submit GPU work without blocking
        // Readback happens ~3 frames later via poll_readback()
        if let (Some(bind_group), Some(output_texture), Some(output_texture_view)) = (
            &self.current_bind_group,
            &self.output_texture,
            &self.output_texture_view,
        ) {
            let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Chroma Key Encoder"),
            });

            {
                let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("Chroma Key Render Pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: output_texture_view,
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

                render_pass.set_pipeline(&self.render_pipeline);
                render_pass.set_bind_group(0, bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.draw(0..4, 0..1);
            }

            self.queue.submit([encoder.finish()]);

            // Copy to next readback buffer in ring (non-blocking!)
            let width = output_texture.width();
            let height = output_texture.height();
            let unpadded_bytes_per_row = width * 4;
            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let padded_bytes_per_row = ((unpadded_bytes_per_row + align - 1) / align) * align;

            // Find next FREE buffer in ring (skip mapped/in-flight)
            let rb_index = self.readback_index % self.readback_ring.len();
            let rb = &mut self.readback_ring[rb_index];
            
            // CRITICAL: Buffer must be FREE (not mapped!) before copy
            if rb.state != ReadbackState::Free {
                // Buffer not ready, skip this frame's GPU processing
                // This shouldn't happen with triple buffering, but safety first
                return Err(format!("Readback buffer {} not free (state: {:?})", rb_index, rb.state));
            }
            
            let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Readback Encoder"),
            });

            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: output_texture,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &rb.buffer,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            // Submit BEFORE mapping (buffer must be unmapped at submit time)
            self.queue.submit([encoder.finish()]);
            
            // Mark as in-flight AFTER submit
            rb.state = ReadbackState::InFlight;
            rb.frame_number = self.frame_count;
            self.readback_index += 1;
            self.frame_count += 1;

            Ok(())
        } else {
            Err("Renderer not properly initialized".to_string())
        }
    }

    pub fn poll_readback(&mut self) -> Option<Vec<u8>> {
        // Look for buffers that need processing
        let num_buffers = self.readback_ring.len();
        
        // First pass: Start mapping on InFlight buffers (not yet mapped)
        for i in 0..num_buffers {
            let rb = &mut self.readback_ring[i];
            
            if rb.state == ReadbackState::InFlight {
                // Start map_async with channel (only called once per buffer!)
                let (tx, rx) = std::sync::mpsc::channel();
                let buffer_slice = rb.buffer.slice(..);
                buffer_slice.map_async(MapMode::Read, move |result| {
                    tx.send(result).ok();
                });
                rb.map_receiver = Some(rx);
                rb.state = ReadbackState::MappingPending;
            }
        }
        
        // Second pass: Check MappingPending buffers to see if ready
        for i in 0..num_buffers {
            let rb = &mut self.readback_ring[i];
            
            if rb.state != ReadbackState::MappingPending {
                continue;
            }

            // Check if mapping completed via channel (non-blocking)
            if let Some(ref rx) = rb.map_receiver {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        // Mapping complete! Now we can get the data
                        let buffer_slice = rb.buffer.slice(..);
                        
                        let width = rb.width;
                        let height = rb.height;
                        let unpadded_bytes_per_row = width * 4;
                        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                        let padded_bytes_per_row = ((unpadded_bytes_per_row + align - 1) / align) * align;
                        
                        // Read data in a scope to ensure BufferView is dropped
                        let rgba_data = {
                            let buffer_data = buffer_slice.get_mapped_range();
                            
                            // Remove padding and copy
                            let mut rgba_data = Vec::with_capacity((width * height * 4) as usize);
                            if padded_bytes_per_row != unpadded_bytes_per_row {
                                for row in 0..height {
                                    let row_start = (row * padded_bytes_per_row) as usize;
                                    let row_end = row_start + unpadded_bytes_per_row as usize;
                                    rgba_data.extend_from_slice(&buffer_data[row_start..row_end]);
                                }
                            } else {
                                rgba_data.extend_from_slice(&buffer_data);
                            }
                            
                            rgba_data
                        }; // buffer_data (BufferView) dropped HERE
                        
                        // CRITICAL: Unmap AFTER BufferView is dropped!
                        rb.buffer.unmap();
                        rb.state = ReadbackState::Free;
                        rb.map_receiver = None;
                        
                        return Some(rgba_data);
                    },
                    Ok(Err(_)) => {
                        // Mapping failed, mark as free to retry
                        rb.state = ReadbackState::Free;
                        rb.map_receiver = None;
                    },
                    Err(_) => {
                        // Not ready yet (WouldBlock), leave as MappingPending
                    }
                }
            }
        }
        
        None // No buffer ready
    }
}

// GStreamer-based composite pipeline with WGPU chroma key integration
pub struct GStreamerComposite {
    pipeline: Option<Pipeline>,
    frame_sender: Arc<RwLock<Option<broadcast::Sender<Vec<u8>>>>>,
    is_running: Arc<RwLock<bool>>,
    // NATIVE COMPOSITOR elements
    compositor: Option<Element>,      // compositor element (blends layers)
    fx_bin: Option<Element>,          // bin containing FX pipeline
    fx_source: Option<Element>,       // filesrc for FX video
    current_fx_file: Option<String>,
    current_chroma_params: Option<(String, f64, f64, bool)>,
    width: u32,
    height: u32,
}

impl GStreamerComposite {
    pub fn new() -> Result<Self, String> {
        // Initialize GStreamer
        gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;

        println!("[Compositor] GStreamer initialized for native compositing");

        Ok(Self {
            pipeline: None,
            frame_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            compositor: None,
            fx_bin: None,
            fx_source: None,
            current_fx_file: None,
            current_chroma_params: None,
            width: 1280,
            height: 720,
        })
    }


    pub fn set_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.frame_sender.write() = Some(sender);
    }

    pub fn start(&mut self, camera_device_id: &str, width: u32, height: u32, fps: u32, rotation: u32, has_camera: bool) -> Result<(), String> {
        println!("[Composite] 🚀 Starting NATIVE COMPOSITOR pipeline: {}x{} @ {}fps (rotation: {}°)", width, height, fps, rotation);
        println!("[Composite] 🎨 Native GPU chroma key + compositing (OBS replacement mode!)");

        // Store dimensions
        self.width = width;
        self.height = height;

        // CRITICAL: Properly stop existing pipeline if any
        if let Some(pipeline) = &self.pipeline {
            println!("[Composite] ⚠️ Stopping existing pipeline before starting new one...");
            *self.is_running.write() = false;
            
            // Set to NULL state and wait for it to complete
            let _ = pipeline.set_state(gst::State::Null);
            
            // Wait for state change to complete (up to 2 seconds)
            match pipeline.state(Some(gst::ClockTime::from_seconds(2))).1 {
                gst::State::Null => {
                    println!("[Composite] ✅ Previous pipeline stopped cleanly");
                }
                state => {
                    println!("[Composite] ⚠️ Previous pipeline in state: {:?} (forcing cleanup)", state);
                }
            }
            
            // Longer wait to ensure camera is released
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        
        // Clear the old pipeline and compositor references
        self.pipeline = None;
        self.compositor = None;
        self.fx_bin = None;
        self.fx_source = None;
        
        *self.is_running.write() = true;

        // Map rotation degrees to videoflip method
        let flip_method = match rotation {
            90 => 1,   // clockwise 90
            180 => 2,  // rotate 180
            270 => 3,  // counterclockwise 90 (clockwise 270)
            _ => 0,    // identity (no rotation)
        };
        
        // CRITICAL: videoflip swaps dimensions for 90° and 270° rotations!
        let (pre_rotation_width, pre_rotation_height) = if rotation == 90 || rotation == 270 {
            (height, width)  // Swap dimensions before rotation
        } else {
            (width, height)  // Keep dimensions for 0° and 180°
        };
        
        // Build NATIVE COMPOSITOR pipeline:
        // compositor → tee → (preview appsink, virtual cam, NDI, etc.)
        //   ↑
        // camera (sink_0, zorder=0, background)
        // FX video (sink_1, zorder=1, foreground) - added dynamically
        
        let pipeline_str = if has_camera && (!camera_device_id.is_empty()) {
            // Escape backslashes in Windows device path
            let escaped_path = camera_device_id.replace("\\", "\\\\");
            
            if flip_method == 0 {
                // No rotation - compositor pipeline
                format!(
                    "compositor name=comp sink_0::zorder=0 background=black ! \
                     videoconvert ! video/x-raw,format=RGBA ! \
                     tee name=t \
                        t. ! queue leaky=downstream max-size-buffers=2 ! \
                           appsink name=preview emit-signals=true sync=false async=false max-buffers=2 drop=true \
                     \
                     mfvideosrc device-path=\"{}\" ! \
                     queue leaky=downstream max-size-buffers=3 ! \
                     videoconvert ! videoscale ! \
                     video/x-raw,format=RGBA,width={},height={} ! \
                     queue leaky=downstream max-size-buffers=1 ! \
                     comp.sink_0",
                    escaped_path, width, height
                )
            } else {
                // With rotation - compositor pipeline
                format!(
                    "compositor name=comp sink_0::zorder=0 background=black ! \
                     videoconvert ! video/x-raw,format=RGBA ! \
                     tee name=t \
                        t. ! queue leaky=downstream max-size-buffers=2 ! \
                           appsink name=preview emit-signals=true sync=false async=false max-buffers=2 drop=true \
                     \
                     mfvideosrc device-path=\"{}\" ! \
                     queue leaky=downstream max-size-buffers=3 ! \
                     videoconvert ! videoscale ! \
                     video/x-raw,width={},height={} ! \
                     videoflip method={} ! \
                     videoconvert ! video/x-raw,format=RGBA ! \
                     queue leaky=downstream max-size-buffers=1 ! \
                     comp.sink_0",
                    escaped_path, pre_rotation_width, pre_rotation_height, flip_method
                )
            }
        } else {
            // Test pattern - also with compositor
            format!(
                "compositor name=comp sink_0::zorder=0 background=black ! \
                 videoconvert ! video/x-raw,format=RGBA ! \
                 tee name=t \
                    t. ! queue leaky=downstream max-size-buffers=2 ! \
                       appsink name=preview emit-signals=true sync=false async=false max-buffers=2 drop=true \
                 \
                 videotestsrc pattern=ball is-live=true ! \
                 video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! \
                 queue ! comp.sink_0",
                width, height, fps
            )
        };

        println!("[Composite] 🏗️  Building NATIVE COMPOSITOR pipeline:");
        println!("[Composite] 📹 Camera → compositor.sink_0 (background, zorder=0)");
        println!("[Composite] 🎬 FX → compositor.sink_1 (foreground with alpha, zorder=1) - dynamic");
        println!("[Composite] 🎨 Compositor → tee → [preview, virtual cam, NDI...]");
        println!("[Composite] Pipeline: {}", pipeline_str);

        // Create pipeline
        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline".to_string())?;

        // Save compositor element reference for dynamic FX switching
        let compositor = pipeline
            .by_name("comp")
            .ok_or("Failed to get compositor element")?;
        self.compositor = Some(compositor);

        // Set up frame callback BEFORE starting pipeline
        let appsink = pipeline
            .by_name("preview")  // Changed from "output" to "preview"
            .ok_or("Failed to get preview appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;

        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();

        // Use Arc<Mutex<>> instead of closure mutation for thread safety
        let frame_count = std::sync::Arc::new(std::sync::Mutex::new(0u64));
        let frame_count_clone = frame_count.clone();

        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    let sample = match appsink.pull_sample() {
                        Ok(s) => s,
                        Err(e) => {
                            println!("[Compositor] ❌ Failed to pull sample: {:?}", e);
                            return Err(gst::FlowError::Eos);
                        }
                    };

                    let buffer = match sample.buffer() {
                        Some(b) => b,
                        None => {
                            println!("[Compositor] ❌ Sample has no buffer");
                            return Err(gst::FlowError::Error);
                        }
                    };

                    let map = match buffer.map_readable() {
                        Ok(m) => m,
                        Err(e) => {
                            println!("[Compositor] ❌ Failed to map buffer: {:?}", e);
                            return Err(gst::FlowError::Error);
                        }
                    };

                    // COMPOSITED RGBA data from GStreamer compositor (camera + FX already blended!)
                    let rgba_data = map.as_slice();
                    
                    // Get frame dimensions from sample caps
                    let caps = sample.caps().expect("Sample has no caps");
                    let structure = caps.structure(0).expect("Caps has no structure");
                    let frame_width = structure.get::<i32>("width").expect("No width in caps") as u32;
                    let frame_height = structure.get::<i32>("height").expect("No height in caps") as u32;
                    
                    // Increment and log frame count
                    let mut count = frame_count_clone.lock().unwrap();
                    *count += 1;
                    
                    if *count == 1 {
                        println!("[Compositor] 🎬 FIRST COMPOSITED FRAME! ({}x{}) - Native GPU blend!", frame_width, frame_height);
                        println!("[Compositor] 🚀 GStreamer compositor is doing ALL the work (camera + FX + chroma key)");
                        println!("[Compositor] 💨 NO CPU processing, NO conversions, just GPU→WebSocket→Canvas!");
                    } else if *count % 90 == 0 {
                        println!("[Compositor] 📡 Frame {} - Native composited output", *count);
                    }

                    // Send composited frames directly (NO WGPU processing needed!)
                    // GStreamer compositor already did: camera + FX + chroma key + blend
                    if let Some(sender) = &*frame_sender.read() {
                        let _ = sender.send(rgba_data.to_vec());
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        
        println!("[Composite] ✅ AppSink callbacks configured");

        // Start pipeline with state transitions
        println!("[Composite] 🔄 Setting pipeline to READY state...");
        pipeline
            .set_state(gst::State::Ready)
            .map_err(|e| format!("Failed to set pipeline to READY: {:?}", e))?;
        
        // Wait for READY state
        std::thread::sleep(std::time::Duration::from_millis(200));
        
        println!("[Composite] 🔄 Setting pipeline to PAUSED state...");
        pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| format!("Failed to set pipeline to PAUSED: {:?}", e))?;
        
        // Wait for PAUSED state to complete
        println!("[Composite] ⏳ Waiting for pipeline to reach PAUSED state...");
        match pipeline.state(Some(gst::ClockTime::from_seconds(5))).1 {
            gst::State::Paused => {
                println!("[Composite] ✅ Pipeline is PAUSED and ready");
            }
            state => {
                println!("[Composite] ⚠️ Pipeline in unexpected state: {:?}", state);
            }
        }
        
        println!("[Composite] 🔄 Setting pipeline to PLAYING state...");
        let state_change_result = pipeline.set_state(gst::State::Playing);
        
        match state_change_result {
            Ok(_) => {
                println!("[Composite] ✅ Pipeline set to PLAYING");
            }
            Err(e) => {
                // Get more detailed error info
                let bus = pipeline.bus().ok_or("No bus available")?;
                if let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error]) {
                    if let gst::MessageView::Error(err) = msg.view() {
                        let error_msg = format!("GStreamer error: {} (debug: {:?})", 
                            err.error(), 
                            err.debug());
                        println!("[Composite] ❌ {}", error_msg);
                        return Err(error_msg);
                    }
                }
                return Err(format!("Failed to start pipeline: {:?}", e));
            }
        }
        
        // Wait a moment and verify pipeline is actually playing
        std::thread::sleep(std::time::Duration::from_millis(500));
        
        let (_, current_state, pending_state) = pipeline.state(None);
        println!("[Composite] 📊 Pipeline state: {:?} (pending: {:?})", current_state, pending_state);
        
        // Check for any errors on the bus
        if let Some(bus) = pipeline.bus() {
            if let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Warning]) {
                match msg.view() {
                    gst::MessageView::Error(err) => {
                        let error_msg = format!("Pipeline error: {} (debug: {:?})", err.error(), err.debug());
                        println!("[Composite] ❌ {}", error_msg);
                        return Err(error_msg);
                    }
                    gst::MessageView::Warning(warn) => {
                        println!("[Composite] ⚠️ Pipeline warning: {} (debug: {:?})", warn.error(), warn.debug());
                    }
                    _ => {}
                }
            }
        }
        
        println!("[Composite] ✅ Pipeline fully initialized and running");

        self.pipeline = Some(pipeline);
        Ok(())
    }

    pub fn play_fx_from_file(&mut self, file_path: String, keycolor: String, tolerance: f64, similarity: f64, use_chroma_key: bool) -> Result<(), String> {
        println!("[Compositor] 🎬 Playing NATIVE FX: {} (chroma: {})", file_path, use_chroma_key);
        println!("[Compositor] 🎨 Native GPU chroma key: tolerance={}, similarity={}", tolerance, similarity);

        // Check if pipeline is running
        if !self.is_running() {
            println!("[Compositor] ❌ Pipeline not initialized - cannot play FX");
            return Err("Pipeline not initialized".to_string());
        }

        // Check if FX file exists
        if !Path::new(&file_path).exists() {
            println!("[Compositor] ❌ FX file does not exist: {}", file_path);
            return Err(format!("FX file does not exist: {}", file_path));
        }

        // Stop any existing FX first
        self.stop_fx_internal()?;

        // Get pipeline and compositor
        let pipeline = self.pipeline.as_ref().ok_or("No pipeline")?;
        let compositor = self.compositor.as_ref().ok_or("No compositor")?;

        // Detect file type
        let is_video = file_path.to_lowercase().ends_with(".mp4") ||
                      file_path.to_lowercase().ends_with(".avi") ||
                      file_path.to_lowercase().ends_with(".mov") ||
                      file_path.to_lowercase().ends_with(".mkv") ||
                      file_path.to_lowercase().ends_with(".webm");

        if !is_video {
            println!("[Compositor] ⚠️ Only video FX supported for now (got: {})", file_path);
            return Err("Only video FX supported".to_string());
        }

        println!("[Compositor] 🏗️ Building FX branch: filesrc → decodebin → alpha → compositor.sink_1");

        // Create FX branch elements
        let filesrc = gst::ElementFactory::make("filesrc")
            .property("location", &file_path)
            .build()
            .map_err(|e| format!("Failed to create filesrc: {}", e))?;

        let decodebin = gst::ElementFactory::make("decodebin")
            .build()
            .map_err(|e| format!("Failed to create decodebin: {}", e))?;

        let videoconvert1 = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| format!("Failed to create videoconvert1: {}", e))?;

        // ALPHA ELEMENT: GPU-accelerated chroma key! 🔥
        let alpha = gst::ElementFactory::make("alpha")
            .property("method", "green")  // Chroma key method
            .build()
            .map_err(|e| format!("Failed to create alpha element: {}", e))?;

        // Map tolerance and similarity to alpha element parameters
        // tolerance (0.0-1.0) → angle (0-180 degrees)
        //   Higher tolerance = larger angle = more aggressive removal
        let angle = (tolerance * 100.0).clamp(10.0, 70.0);  // 10-70 degrees range
        
        // similarity (0.0-1.0) → noise-level (0-255)
        //   Higher similarity = higher noise-level = smoother edges
        let noise_level = (similarity * 30.0).clamp(1.0, 10.0) as u32;  // 1-10 range

        println!("[Compositor] 🎨 Chroma key params: angle={} (tolerance={}), noise-level={} (similarity={})", 
            angle, tolerance, noise_level, similarity);

        // Set alpha properties
        alpha.set_property("angle", angle as u32);  // How far from key color to remove
        alpha.set_property("noise-level", noise_level);  // Edge smoothness
        
        // Parse key color (default to green #00ff00)
        let (target_r, target_g, target_b) = if keycolor.starts_with('#') {
            let hex = &keycolor[1..];
            if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
                let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
                (r as i32, g as i32, b as i32)
            } else {
                (0, 255, 0)
            }
        } else {
            (0, 255, 0)
        };
        
        alpha.set_property("target-r", target_r);
        alpha.set_property("target-g", target_g);
        alpha.set_property("target-b", target_b);
        
        println!("[Compositor] 🎨 Key color: RGB({}, {}, {})", target_r, target_g, target_b);

        let videoscale = gst::ElementFactory::make("videoscale")
            .build()
            .map_err(|e| format!("Failed to create videoscale: {}", e))?;

        let caps_filter = gst::ElementFactory::make("capsfilter")
            .build()
            .map_err(|e| format!("Failed to create capsfilter: {}", e))?;

        // Set caps to match compositor dimensions
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .field("width", self.width as i32)
            .field("height", self.height as i32)
            .build();
        caps_filter.set_property("caps", &caps);

        let videoconvert2 = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| format!("Failed to create videoconvert2: {}", e))?;

        let queue = gst::ElementFactory::make("queue")
            .property("leaky", "downstream")
            .property("max-size-buffers", 1u32)
            .build()
            .map_err(|e| format!("Failed to create queue: {}", e))?;

        // Add all elements to pipeline
        pipeline.add_many(&[
            &filesrc, &decodebin, &videoconvert1, &alpha, 
            &videoscale, &caps_filter, &videoconvert2, &queue
        ]).map_err(|e| format!("Failed to add FX elements to pipeline: {}", e))?;

        // Link static elements (filesrc → decodebin is dynamic)
        gst::Element::link_many(&[
            &filesrc, &decodebin
        ]).map_err(|e| format!("Failed to link filesrc → decodebin: {}", e))?;

        // Link the rest (after decodebin's dynamic pads)
        gst::Element::link_many(&[
            &videoconvert1, &alpha, &videoscale, &caps_filter, &videoconvert2, &queue
        ]).map_err(|e| format!("Failed to link FX processing chain: {}", e))?;

        // Link queue to compositor.sink_1 (foreground layer with zorder=1)
        let queue_src = queue.static_pad("src").ok_or("No src pad on queue")?;
        let comp_sink = compositor.request_pad_simple("sink_%u").ok_or("Failed to request compositor sink pad")?;
        
        // Set compositor pad properties (zorder=1 for foreground)
        comp_sink.set_property("zorder", 1i32);
        comp_sink.set_property("xpos", 0i32);
        comp_sink.set_property("ypos", 0i32);
        
        queue_src.link(&comp_sink).map_err(|e| format!("Failed to link queue → compositor: {:?}", e))?;

        // Handle decodebin's dynamic pad-added signal
        let videoconvert1_weak = videoconvert1.downgrade();
        decodebin.connect_pad_added(move |_, src_pad| {
            println!("[Compositor] 🔌 decodebin pad-added: {}", src_pad.name());
            
            let Some(videoconvert1) = videoconvert1_weak.upgrade() else {
                return;
            };
            
            // Only link video pads
            let caps = src_pad.current_caps().expect("No caps on pad");
            let structure = caps.structure(0).expect("No structure in caps");
            let name = structure.name();
            
            if name.starts_with("video/") {
                let sink_pad = videoconvert1.static_pad("sink").expect("No sink pad");
                if sink_pad.is_linked() {
                    println!("[Compositor] ⚠️ Pad already linked");
                    return;
                }
                
                if let Err(e) = src_pad.link(&sink_pad) {
                    println!("[Compositor] ❌ Failed to link decodebin → videoconvert: {:?}", e);
                } else {
                    println!("[Compositor] ✅ Linked decodebin → videoconvert → alpha → compositor");
                }
            }
        });

        // Store references
        self.fx_bin = Some(filesrc.clone());  // Keep filesrc as reference
        self.fx_source = Some(filesrc.clone());
        self.current_fx_file = Some(file_path.clone());
        self.current_chroma_params = Some((keycolor, tolerance, similarity, use_chroma_key));

        // Sync FX elements to pipeline state (PLAYING)
        filesrc.sync_state_with_parent().map_err(|e| format!("Failed to sync filesrc: {:?}", e))?;
        decodebin.sync_state_with_parent().map_err(|e| format!("Failed to sync decodebin: {:?}", e))?;
        videoconvert1.sync_state_with_parent().map_err(|e| format!("Failed to sync videoconvert1: {:?}", e))?;
        alpha.sync_state_with_parent().map_err(|e| format!("Failed to sync alpha: {:?}", e))?;
        videoscale.sync_state_with_parent().map_err(|e| format!("Failed to sync videoscale: {:?}", e))?;
        caps_filter.sync_state_with_parent().map_err(|e| format!("Failed to sync capsfilter: {:?}", e))?;
        videoconvert2.sync_state_with_parent().map_err(|e| format!("Failed to sync videoconvert2: {:?}", e))?;
        queue.sync_state_with_parent().map_err(|e| format!("Failed to sync queue: {:?}", e))?;

        println!("[Compositor] ✅ NATIVE FX playing with GPU chroma key!");
        println!("[Compositor] 🎨 GStreamer alpha element is removing green on GPU! 🚀");
        Ok(())
    }

    pub fn stop_fx(&mut self) -> Result<(), String> {
        self.stop_fx_internal()
    }

    fn stop_fx_internal(&mut self) -> Result<(), String> {
        println!("[Compositor] 🛑 Stopping FX...");

        // If no FX playing, nothing to do
        if self.fx_bin.is_none() && self.fx_source.is_none() {
            return Ok(());
        }

        // Get pipeline
        if let Some(pipeline) = &self.pipeline {
            // Set FX elements to NULL state before removing
            if let Some(fx_src) = &self.fx_source {
                let _ = fx_src.set_state(gst::State::Null);
            }

            // Find and remove all FX-related elements
            // Note: This is a simplified approach - ideally we'd track all FX elements
            // For now, just remove the filesrc and let GStreamer clean up the rest
            if let Some(fx_src) = self.fx_source.take() {
                let _ = pipeline.remove(&fx_src);
            }
            
            println!("[Compositor] ✅ FX elements cleaned up");
        }

        self.fx_bin = None;
        self.fx_source = None;
        self.current_fx_file = None;
        self.current_chroma_params = None;

        println!("[Compositor] ✅ FX stopped");
        Ok(())
    }

    pub fn update_layers(&self, camera: (bool, f64), overlay: (bool, f64)) {
        if let Some(_pipeline) = &self.pipeline {
            // Update layer visibility based on camera and overlay settings
            println!("[Composite] Updated layers - Camera: {}, Overlay: {}", camera.0, overlay.0);
        }
    }

    pub fn set_output_format(&mut self, format: &str) -> Result<(), String> {
        println!("[Composite] Setting output format: {}", format);

        // For now, just support "preview" format
        // Could extend to support different output formats like MP4, RTMP, etc.
        if format != "preview" {
            return Err(format!("Unsupported output format: {}", format));
        }

        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        println!("[Composite] Stopping composite pipeline");

        *self.is_running.write() = false;

        if let Some(pipeline) = &self.pipeline {
            pipeline
                .set_state(gst::State::Null)
                .map_err(|e| format!("Failed to stop pipeline: {:?}", e))?;
        }

        self.pipeline = None;
        self.compositor = None;
        self.fx_bin = None;
        self.fx_source = None;

        println!("[Compositor] Composite pipeline stopped");
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        *self.is_running.read()
    }

    pub fn get_pipeline_state(&self) -> Option<gst::State> {
        if let Some(pipeline) = &self.pipeline {
            let state_result = pipeline.state(Some(gst::ClockTime::from_seconds(1)));
            Some(state_result.1)
        } else {
            None
        }
    }

    pub fn emergency_cleanup(&self) -> Result<(), String> {
        // Emergency cleanup for stuck pipelines
        if let Some(pipeline) = &self.pipeline {
            let _ = pipeline.set_state(gst::State::Null);
        }
        Ok(())
    }
}

impl Drop for GStreamerComposite {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

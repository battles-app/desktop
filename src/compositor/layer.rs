use glam::{Mat4, Vec2, Vec3};
use wgpu::{Device, Queue, Texture, TextureView};

/// Represents a layer in the compositor
#[derive(Debug)]
pub struct Layer {
    /// Unique identifier for the layer
    pub id: String,
    
    /// The texture containing the layer's image data
    pub texture: Option<Texture>,
    
    /// View into the texture for rendering
    pub texture_view: Option<TextureView>,
    
    /// Position (x, y) in normalized coordinates (-1 to 1)
    pub position: Vec2,
    
    /// Scale (width, height) in normalized coordinates
    pub scale: Vec2,
    
    /// Rotation in radians
    pub rotation: f32,
    
    /// Opacity (0.0 - 1.0)
    pub opacity: f32,
    
    /// Z-order (higher values are rendered on top)
    pub z_order: i32,
    
    /// Whether the layer is visible
    pub visible: bool,
    
    /// Presentation timestamp in nanoseconds
    pub pts: u64,
    
    /// Duration of the frame in nanoseconds
    pub duration: u64,
    
    /// Source width in pixels
    pub width: u32,
    
    /// Source height in pixels
    pub height: u32,
}

impl Layer {
    /// Create a new layer with default values
    pub fn new(id: String, _transform: Mat4, opacity: f32, rotation: f32, z_order: i32, visible: bool, pts: u64, duration: u64, width: u32, height: u32) -> Self {
        Self {
            id,
            texture: None,
            texture_view: None,
            position: Vec2::ZERO,
            scale: Vec2::ONE,
            rotation,
            opacity,
            z_order,
            visible,
            pts,
            duration,
            width,
            height,
        }
    }
    
    /// Create a new layer with simple parameters
    pub fn new_simple(id: &str) -> Self {
        Self {
            id: id.to_string(),
            texture: None,
            texture_view: None,
            position: Vec2::ZERO,
            scale: Vec2::ONE,
            rotation: 0.0,
            opacity: 1.0,
            z_order: 0,
            visible: true,
            pts: 0,
            duration: 0,
            width: 0,
            height: 0,
        }
    }
    
    /// Update the texture with new data
    pub fn update_texture(
        &mut self, 
        device: &Device, 
        queue: &Queue, 
        data: &[u8], 
        width: u32, 
        height: u32,
        format: wgpu::TextureFormat,
    ) {
        // Create a new texture with the given dimensions
        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("Layer_{}_Texture", self.id)),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        
        // Write the pixel data to the texture
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * width), // 4 bytes per pixel for RGBA
                rows_per_image: Some(height),
            },
            texture_size,
        );
        
        // Create a view into the texture
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        // Update the layer's texture and dimensions
        self.texture = Some(texture);
        self.texture_view = Some(texture_view);
        self.width = width;
        self.height = height;
    }
    
    /// Calculate the transformation matrix for this layer
    pub fn transform_matrix(&self, output_aspect_ratio: f32) -> Mat4 {
        let source_aspect_ratio = self.width as f32 / self.height as f32;
        
        // Start with identity matrix
        let mut transform = Mat4::IDENTITY;
        
        // Apply translation
        transform = transform * Mat4::from_translation(Vec3::new(self.position.x, self.position.y, 0.0));
        
        // Apply rotation around the Z axis
        transform = transform * Mat4::from_rotation_z(self.rotation);
        
        // Apply scale with aspect ratio correction
        let scale_x = self.scale.x;
        let scale_y = self.scale.y * (source_aspect_ratio / output_aspect_ratio);
        transform = transform * Mat4::from_scale(Vec3::new(scale_x, scale_y, 1.0));
        
        transform
    }
    
    /// Create instance data for the GPU
    pub fn create_instance_data(&self, output_aspect_ratio: f32) -> LayerInstanceRaw {
        LayerInstanceRaw {
            transform: self.transform_matrix(output_aspect_ratio).to_cols_array_2d(),
            opacity: self.opacity,
            _padding: [0.0, 0.0, 0.0],
        }
    }
}

/// Raw instance data to pass to the GPU
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LayerInstanceRaw {
    /// 4x4 transformation matrix
    pub transform: [[f32; 4]; 4],
    /// Opacity value
    pub opacity: f32,
    /// Padding to ensure alignment
    pub _padding: [f32; 3],
}

use glam::{Vec2, Vec3, Quat, Mat4};
use wgpu::Texture;

/// Represents a single compositing layer with transform, opacity, and texture
#[derive(Debug, Clone)]
pub struct Layer {
    pub id: String,
    pub texture: Option<wgpu::Texture>,
    pub position: Vec2,        // x, y position
    pub scale: Vec2,           // width, height scale
    pub rotation: f32,         // rotation in radians
    pub opacity: f32,          // 0.0 to 1.0
    pub z_order: i32,          // drawing order (higher = on top)
    pub visible: bool,
    pub chroma_key: Option<[f32; 3]>, // RGB color to key out (0.0-1.0 range)
    pub chroma_tolerance: f32, // tolerance for chroma keying
}

impl Layer {
    pub fn new(id: String) -> Self {
        Self {
            id,
            texture: None,
            position: Vec2::ZERO,
            scale: Vec2::ONE,
            rotation: 0.0,
            opacity: 1.0,
            z_order: 0,
            visible: true,
            chroma_key: None,
            chroma_tolerance: 0.1,
        }
    }

    /// Create a layer with a texture
    pub fn with_texture(id: String, texture: wgpu::Texture) -> Self {
        Self {
            id,
            texture: Some(texture),
            position: Vec2::ZERO,
            scale: Vec2::ONE,
            rotation: 0.0,
            opacity: 1.0,
            z_order: 0,
            visible: true,
            chroma_key: None,
            chroma_tolerance: 0.1,
        }
    }

    /// Set position (x, y)
    pub fn with_position(mut self, x: f32, y: f32) -> Self {
        self.position = Vec2::new(x, y);
        self
    }

    /// Set scale (width, height multipliers)
    pub fn with_scale(mut self, width: f32, height: f32) -> Self {
        self.scale = Vec2::new(width, height);
        self
    }

    /// Set rotation in degrees
    pub fn with_rotation_degrees(mut self, degrees: f32) -> Self {
        self.rotation = degrees.to_radians();
        self
    }

    /// Set rotation in radians
    pub fn with_rotation_radians(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }

    /// Set opacity (0.0 = transparent, 1.0 = opaque)
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Set z-order for layering
    pub fn with_z_order(mut self, z_order: i32) -> Self {
        self.z_order = z_order;
        self
    }

    /// Set visibility
    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Set chroma key color (RGB 0.0-1.0)
    pub fn with_chroma_key(mut self, r: f32, g: f32, b: f32, tolerance: f32) -> Self {
        self.chroma_key = Some([r, g, b]);
        self.chroma_tolerance = tolerance;
        self
    }

    /// Update the texture
    pub fn update_texture(&mut self, texture: wgpu::Texture) {
        self.texture = Some(texture);
    }

    /// Get the transformation matrix for this layer
    pub fn transform_matrix(&self, output_width: f32, output_height: f32) -> Mat4 {
        // Start with identity
        let mut transform = Mat4::IDENTITY;

        // Translate to position (convert from pixel coordinates to NDC)
        let ndc_x = (self.position.x / output_width) * 2.0 - 1.0;
        let ndc_y = 1.0 - (self.position.y / output_height) * 2.0; // Flip Y for screen coordinates
        transform = Mat4::from_translation(Vec3::new(ndc_x, ndc_y, 0.0)) * transform;

        // Apply rotation
        transform = Mat4::from_rotation_z(self.rotation) * transform;

        // Apply scale
        transform = Mat4::from_scale(Vec3::new(self.scale.x, self.scale.y, 1.0)) * transform;

        transform
    }

    /// Get texture dimensions if available
    pub fn texture_size(&self) -> Option<(u32, u32)> {
        self.texture.as_ref().map(|tex| {
            let size = tex.size();
            (size.width, size.height)
        })
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self::new("default".to_string())
    }
}

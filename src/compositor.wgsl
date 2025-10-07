// WGSL Shader for GPU Compositor - Alpha blending camera + FX layers

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

struct FxParams {
    fx_rect: vec4<f32>, // x, y, width, height (normalized 0-1)
    fx_alpha: f32,
    _padding: vec3<f32>,
}

@group(0) @binding(0)
var camera_texture: texture_2d<f32>;

@group(0) @binding(1)
var camera_sampler: sampler;

@group(0) @binding(2)
var fx_texture: texture_2d<f32>;

@group(0) @binding(3)
var fx_sampler: sampler;

@group(0) @binding(4)
var<uniform> fx_params: FxParams;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.tex_coords = input.tex_coords;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Sample camera texture (full screen)
    let camera_color = textureSample(camera_texture, camera_sampler, input.tex_coords);

    // Check if we're inside the FX rectangle
    let fx_rect = fx_params.fx_rect;
    let in_fx_rect = input.tex_coords.x >= fx_rect.x &&
                     input.tex_coords.x <= (fx_rect.x + fx_rect.z) &&
                     input.tex_coords.y >= fx_rect.y &&
                     input.tex_coords.y <= (fx_rect.y + fx_rect.w);

    if (in_fx_rect) {
        // Convert screen coordinates to FX texture coordinates
        let fx_uv = vec2<f32>(
            (input.tex_coords.x - fx_rect.x) / fx_rect.z,
            (input.tex_coords.y - fx_rect.y) / fx_rect.w
        );

        // Sample FX texture
        let fx_color = textureSample(fx_texture, fx_sampler, fx_uv);

        // Alpha blend: FX over camera
        // Result = FX.rgb * FX.a * alpha + Camera.rgb * (1 - FX.a * alpha)
        let blended_rgb = fx_color.rgb * fx_color.a * fx_params.fx_alpha +
                         camera_color.rgb * (1.0 - fx_color.a * fx_params.fx_alpha);

        return vec4<f32>(blended_rgb, 1.0);
    } else {
        // Outside FX rect, just show camera
        return camera_color;
    }
}

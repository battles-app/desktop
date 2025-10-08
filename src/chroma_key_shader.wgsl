// Chroma Key Shader for WGPU

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = vec4<f32>(input.position, 0.0, 1.0);
    output.tex_coords = input.tex_coords;
    return output;
}

struct Uniforms {
    key_color: vec3<f32>,
    _padding1: f32,
    tolerance: f32,
    similarity: f32,
    use_chroma_key: f32,
    _padding2: f32,
}

@group(0) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(0) @binding(1)
var s_diffuse: sampler;
@group(0) @binding(2)
var<uniform> uniforms: Uniforms;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_diffuse, s_diffuse, input.tex_coords);
    
    // Skip chroma keying if not enabled
    if (uniforms.use_chroma_key < 0.5) {
        return color;
    }
    
    // Chroma key algorithm
    let diff = abs(color.rgb - uniforms.key_color);
    let distance = length(diff);
    
    // Calculate alpha based on distance from key color
    var alpha = 1.0;
    if (distance < uniforms.tolerance) {
        alpha = 0.0;
    } else if (distance < uniforms.tolerance + uniforms.similarity) {
        alpha = (distance - uniforms.tolerance) / uniforms.similarity;
    }
    
    return vec4<f32>(color.rgb, color.a * alpha);
}


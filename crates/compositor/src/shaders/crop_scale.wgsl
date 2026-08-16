// Pipeline minima de Fase 1: crop + scale, sin motion blur ni cursor
// reconstruido (eso es Fase 3, ver ARQUITECTURA.md seccion 6).
//
// Truco de "fullscreen triangle": un solo triangulo de 3 vertices que cubre
// de sobra el viewport; lo que cae fuera de la pantalla se clipea gratis.

struct CropRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
};

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var input_sampler: sampler;
@group(0) @binding(2) var<uniform> crop: CropRect;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);

    var out: VertexOutput;
    out.clip_position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let sample_uv = vec2<f32>(crop.x + in.uv.x * crop.w, crop.y + in.uv.y * crop.h);
    return textureSample(input_texture, input_sampler, sample_uv);
}

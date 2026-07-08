// prelude.wgsl — Common preamble prepended to every shader visualizer.
//
// A shader visualizer is a WGSL *fragment shader* that defines:
//
//     @fragment
//     fn fs_main(in: VsOut) -> @location(0) vec4<f32> { ... }
//
// The engine supplies everything below: the uniform block, the audio-data
// texture, the previous frame (for phosphor/feedback effects), a linear
// sampler, and the fullscreen-triangle vertex shader.
//
// Binding contract (group 0):
//   @binding(0)  uniform Uniforms
//   @binding(1)  audio_tex   — R32Float, AUDIO_SAMPLES x 4 texels
//                  row 0 = left PCM, row 1 = right PCM, row 2 = mono PCM
//                  row 3 = FFT magnitude spectrum (bin i = i * sample_rate / 4096 Hz)
//   @binding(2)  prev_frame  — this visualizer's previous rendered frame
//   @binding(3)  tex_sampler — linear-filtering sampler (for prev_frame)
//
// u.params is 16 free-form floats (4 vec4s) filled by the visualizer's
// Rust wrapper each frame — the meaning is defined per shader.

struct Uniforms {
    resolution:  vec2<f32>,   // render-target size in pixels
    time:        f32,         // seconds since start
    dt:          f32,         // seconds since previous frame
    rms:         vec2<f32>,   // (left, right) RMS level
    beat:        f32,         // beat intensity, 0 when no beat this frame
    sample_rate: f32,         // Hz
    params:      array<vec4<f32>, 4>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var audio_tex: texture_2d<f32>;
@group(0) @binding(2) var prev_frame: texture_2d<f32>;
@group(0) @binding(3) var tex_sampler: sampler;

const AUDIO_SAMPLES: i32 = 4096;
const ROW_LEFT: i32 = 0;
const ROW_RIGHT: i32 = 1;
const ROW_MONO: i32 = 2;
const ROW_FFT: i32 = 3;

// Fetch one audio texel. `i` is clamped to the valid range.
fn audio_sample(row: i32, i: i32) -> f32 {
    return textureLoad(audio_tex, vec2<i32>(clamp(i, 0, AUDIO_SAMPLES - 1), row), 0).r;
}

// Sample the previous frame at uv (0..1, y down).
fn prev_pixel(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(prev_frame, tex_sampler, uv);
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,   // 0..1, y increases downward
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let xy = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return out;
}

// fx_prelude.wgsl — prepended to every post-effect shader (src/gpu/fx/).
//
// A post effect is a fragment shader that samples the previous stage of the
// chain (`src` — the visualizer output or the preceding effect) and returns
// the filtered colour.  The engine provides the bindings and the fullscreen
// vertex shader; the effect defines only:
//
//   @fragment fn fs_main(in: VsOut) -> @location(0) vec4<f32>
//
// Available:
//   src / src_sampler — previous stage (linear filtering, clamp-to-edge)
//   fx.resolution     — output target size in pixels (the final pass runs at
//                       display resolution inside the viz rect)
//   fx.time           — seconds since app start
//   fx.params         — 8 free-form floats (two vec4s), meaning per effect;
//                       fed from the effect's parameter specs in src/fx.rs

struct FxUniforms {
    resolution: vec2<f32>,
    time: f32,
    _pad: f32,
    params: array<vec4<f32>, 2>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> fx: FxUniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let xy = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return out;
}

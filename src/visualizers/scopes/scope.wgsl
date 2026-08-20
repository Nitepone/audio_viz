// scope.wgsl — Dual-channel oscilloscope fragment shader.
//
// Params (u.params[0]):
//   .x  gain       — amplitude multiplier
//   .y  n_samples  — number of trailing samples shown across the width
//   .z  mode       — 0 = stereo (two panels), 1 = mono (single averaged trace)
//   .w  thickness  — line core width in pixels
//   u.params[1].x  trig_base — sample index mapped to the left edge (x=0).
//                   The CPU sets this to a rising zero-crossing so periodic
//                   signals hold still; without trigger it is the plain
//                   trailing-window start.
//
// Each pixel estimates its distance to the waveform polyline by sampling the
// waveform at several x offsets around itself, then shades a Gaussian core
// plus a wider glow.  A faint trail persists via the feedback texture.

// x-offsets sampled around the pixel when estimating distance to the trace.
const TAPS: i32 = 12;
// Trail retention after one second (short phosphor-like fade).
const TRAIL: f32 = 0.01;

// y position (in pixels) of the waveform for channel `row` at pixel column x.
// The fractional sample index is interpolated with a Catmull-Rom spline so
// the trace stays smooth when few samples span the width — truncating to the
// nearest sample would draw stair-steps.
fn wave_y_px(row: i32, x_px: f32, n_show: f32, gain: f32,
             panel_top: f32, panel_h: f32) -> f32 {
    let base = u.params[1].x;
    let fidx = base + clamp(x_px / u.resolution.x, 0.0, 1.0) * (n_show - 1.0);
    let i1 = i32(floor(fidx));
    let t  = fract(fidx);
    let s0 = audio_sample(row, i1 - 1);
    let s1 = audio_sample(row, i1);
    let s2 = audio_sample(row, i1 + 1);
    let s3 = audio_sample(row, i1 + 2);
    let a = 0.5 * (2.0 * s1
        + (s2 - s0) * t
        + (2.0 * s0 - 5.0 * s1 + 4.0 * s2 - s3) * t * t
        + (3.0 * (s1 - s2) + s3 - s0) * t * t * t);
    let amp = clamp(a * gain, -1.0, 1.0);
    return panel_top + (1.0 - amp) * 0.5 * panel_h;
}

// Shaded intensity (core, glow) for one channel in one panel.
fn trace_intensity(row: i32, p_px: vec2<f32>, n_show: f32, gain: f32,
                   panel_top: f32, panel_h: f32, thickness: f32) -> vec2<f32> {
    // Estimate distance to the polyline from a fan of nearby x samples.
    var d2_min = 1e12;
    for (var j = 0; j <= TAPS; j = j + 1) {
        let xo = (f32(j) / f32(TAPS) - 0.5) * 4.0 * max(thickness, 1.0);
        let sx = p_px.x + xo;
        let sy = wave_y_px(row, sx, n_show, gain, panel_top, panel_h);
        let d2 = xo * xo + (sy - p_px.y) * (sy - p_px.y);
        d2_min = min(d2_min, d2);
    }
    let core = exp(-d2_min / (thickness * thickness));
    let gw   = thickness * 3.0;
    let glow = exp(-d2_min / (gw * gw));
    return vec2<f32>(core, glow);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let gain      = u.params[0].x;
    let n_show    = max(u.params[0].y, 2.0);
    let mono      = i32(u.params[0].z + 0.5) == 1;
    let thickness = max(u.params[0].w, 0.25);

    let res  = u.resolution;
    let p_px = in.uv * res;

    let cyan   = vec3<f32>(0.15, 0.9, 1.0);
    let orange = vec3<f32>(1.0, 0.62, 0.15);

    var col = vec3<f32>(0.0);
    var center_y = 0.0;

    if (mono) {
        let t = trace_intensity(ROW_MONO, p_px, n_show, gain, 0.0, res.y, thickness);
        col = cyan * (t.x * 1.1 + t.y * 0.08) + vec3<f32>(1.0) * t.x * t.x * 0.2;
        center_y = 0.5 * res.y;
        // Zero line
        let dz = abs(p_px.y - center_y);
        col = col + cyan * exp(-dz * dz / 2.0) * 0.05;
    } else {
        let half_h = 0.5 * res.y;
        let lt = trace_intensity(ROW_LEFT, p_px, n_show, gain, 0.0, half_h, thickness);
        let rt = trace_intensity(ROW_RIGHT, p_px, n_show, gain, half_h, half_h, thickness);
        col = cyan   * (lt.x * 1.1 + lt.y * 0.08) + vec3<f32>(1.0) * lt.x * lt.x * 0.2
            + orange * (rt.x * 1.1 + rt.y * 0.08) + vec3<f32>(1.0) * rt.x * rt.x * 0.2;

        // Zero lines for each panel + separator between panels.
        let dz_l = abs(p_px.y - 0.25 * res.y);
        let dz_r = abs(p_px.y - 0.75 * res.y);
        let dsep = abs(p_px.y - 0.5 * res.y);
        col = col + cyan * exp(-dz_l * dz_l / 2.0) * 0.05
                  + orange * exp(-dz_r * dz_r / 2.0) * 0.05
                  + vec3<f32>(0.5) * exp(-dsep * dsep / 0.5) * 0.10;
    }

    // Faint trail from the previous frame; phosphor_fade (prelude.wgsl)
    // accelerates the decay as it dims so ghosts clear quickly.
    let trail = phosphor_fade(prev_pixel(in.uv).rgb, TRAIL, u.dt);
    col = max(col, trail);

    return vec4<f32>(min(col, vec3<f32>(4.0)), 1.0);
}

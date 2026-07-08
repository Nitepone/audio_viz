// polar.wgsl — Polar waveform oscilloscope fragment shader.
//
// Params:
//   u.params[0].x  gain         — amplitude multiplier
//   u.params[0].y  persistence  — phosphor brightness retained per second (0..0.99)
//   u.params[0].z  theme        — 0 = green (P31), 1 = amber (P3), 2 = white (P4)
//   u.params[0].w  focus        — beam width multiplier (lower = sharper)
//   u.params[1].x  base_radius  — zero-amplitude ring radius as a fraction of
//                                 the maximum usable radius
//
// The mono waveform is bent into a circle: time maps to angle (one full audio
// window per revolution) and amplitude modulates the radius around a base
// ring — silence draws a perfect circle, loud passages push the perimeter in
// and out.  The beam is drawn analytically like classic_lissajous: each pixel
// accumulates Gaussian falloff from every consecutive polar sample segment,
// weighted by inverse beam speed.  A dim reference ring marks zero amplitude
// and persistence uses the engine's feedback texture.

const N_POINTS: i32 = 512;
const TAU: f32 = 6.28318530718;

fn beam_color(theme: i32) -> vec3<f32> {
    if (theme == 1) { return vec3<f32>(1.0, 0.62, 0.12); }   // amber
    if (theme == 2) { return vec3<f32>(0.92, 0.96, 1.0); }   // white
    return vec3<f32>(0.20, 1.0, 0.35);                       // green
}

// Polar sample point i in centered square coordinates.
fn polar_pt(i: i32, stride: i32, gain: f32, r_base: f32, r_amp: f32) -> vec2<f32> {
    let theta = TAU * f32(i) / f32(N_POINTS);
    let amp   = clamp(audio_sample(ROW_MONO, i * stride) * gain, -1.0, 1.0);
    let r     = r_base + amp * r_amp;
    return vec2<f32>(cos(theta), sin(theta)) * r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let gain        = u.params[0].x;
    let persistence = max(u.params[0].y, 0.001);
    let theme       = i32(u.params[0].z + 0.5);
    let focus       = u.params[0].w;
    let base_frac   = clamp(u.params[1].x, 0.05, 0.95);

    // Centered square coordinates: shorter window axis spans [-1, 1].
    let res  = u.resolution;
    let p_px = in.uv * res;
    let half = 0.5 * min(res.x, res.y);
    let sq   = (p_px - 0.5 * res) / half;

    // Ring geometry: keep the full-scale excursion inside the pane.
    let r_max  = 0.93;
    let r_base = base_frac * r_max;
    let r_amp  = (r_max - r_base) * 0.85;

    // Beam width in square-space units (focus 1.0 ≈ 2.5 px on a 1080p target).
    let sigma      = focus * 5.0 / half;
    let glow_sigma = sigma * 3.0;

    // ── Accumulate beam energy over the closed polar path ──────────────────
    let stride = AUDIO_SAMPLES / N_POINTS;
    var energy = 0.0;
    var glow   = 0.0;
    var prev_pt = polar_pt(0, stride, gain, r_base, r_amp);
    for (var i = 1; i <= N_POINTS; i = i + 1) {
        // i == N_POINTS wraps to point 0, closing the ring.
        let pt = polar_pt(i % N_POINTS, stride, gain, r_base, r_amp);

        // Distance from this pixel to the segment prev_pt → pt.
        let pa = sq - prev_pt;
        let ba = pt - prev_pt;
        let h  = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-9), 0.0, 1.0);
        let d2 = dot(pa - ba * h, pa - ba * h);

        // Slow beam ⇒ more dwell time ⇒ brighter deposit.
        let seg_len = length(ba);
        let dwell   = 0.004 / (0.004 + seg_len);

        energy = energy + exp(-d2 / (sigma * sigma)) * dwell;
        glow   = glow   + exp(-d2 / (glow_sigma * glow_sigma)) * dwell;

        prev_pt = pt;
    }

    let base = beam_color(theme);
    // White-hot core on strong deposits, coloured halo around it.  Scaled
    // well below classic_lissajous: the polar path concentrates the whole
    // beam onto a ring, so per-pixel dwell energy runs several times higher.
    let deposit = base * (energy * 1.2 + glow * 0.06)
                + vec3<f32>(1.0, 1.0, 1.0) * energy * energy * 0.15;

    // ── Phosphor persistence via feedback ──────────────────────────────────
    // max() model: fresh deposits stamp at their true brightness and the
    // previous frame only fades — additive feedback would accumulate a
    // static trace to deposit/(1-decay) ≈ 90× and blow out the ring.
    // phosphor_fade (prelude.wgsl) accelerates the decay as the trace dims.
    let prev = phosphor_fade(prev_pixel(in.uv).rgb, persistence, u.dt);

    // Dim zero-amplitude reference ring.
    let ring_w = 1.5 / half;
    let dr = length(sq) - r_base;
    let ring = exp(-dr * dr / (ring_w * ring_w)) * 0.05;
    let floor_light = base * ring;

    let col = max(prev, deposit + floor_light);
    return vec4<f32>(min(col, vec3<f32>(4.0)), 1.0);
}

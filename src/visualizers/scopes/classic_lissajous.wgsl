// classic_lissajous.wgsl — XY phosphor oscilloscope fragment shader.
//
// Params (u.params[0]):
//   .x  gain         — amplitude multiplier for both channels
//   .y  persistence  — phosphor brightness retained per second (0..0.99)
//   .z  theme        — 0 = green (P31), 1 = amber (P3), 2 = white (P4)
//   .w  focus        — beam width multiplier (lower = sharper)
//
// The beam is drawn analytically: for each pixel we accumulate a Gaussian
// falloff from every consecutive (left[i], right[i]) → (left[i+1], right[i+1])
// segment.  Contributions are weighted by inverse beam speed, so slow
// passages deposit more "phosphor" — the behaviour of a real CRT.
// Persistence uses the engine's previous-frame feedback texture.

const N_POINTS: i32 = 512;

fn beam_color(theme: i32) -> vec3<f32> {
    if (theme == 1) { return vec3<f32>(1.0, 0.62, 0.12); }   // amber
    if (theme == 2) { return vec3<f32>(0.92, 0.96, 1.0); }   // white
    return vec3<f32>(0.20, 1.0, 0.35);                       // green
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let gain        = u.params[0].x;
    let persistence = max(u.params[0].y, 0.001);
    let theme       = i32(u.params[0].z + 0.5);
    let focus       = u.params[0].w;

    // Centered square coordinates: shorter window axis spans [-1, 1].
    let res  = u.resolution;
    let p_px = in.uv * res;
    let half = 0.5 * min(res.x, res.y);
    let sq   = (p_px - 0.5 * res) / half;   // y increases downward

    // Beam width in square-space units (focus 1.0 ≈ 2.5 px on a 1080p target).
    let sigma      = focus * 5.0 / half;
    let glow_sigma = sigma * 6.0;

    // ── Accumulate beam energy over the sample path ────────────────────────
    let stride = AUDIO_SAMPLES / N_POINTS;
    var energy = 0.0;
    var glow   = 0.0;
    var prev_pt = vec2<f32>(
        audio_sample(ROW_LEFT, 0) * gain,
        -audio_sample(ROW_RIGHT, 0) * gain,
    );
    for (var i = 1; i < N_POINTS; i = i + 1) {
        let pt = vec2<f32>(
            audio_sample(ROW_LEFT, i * stride) * gain,
            -audio_sample(ROW_RIGHT, i * stride) * gain,
        );

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
    // White-hot core on strong deposits, coloured halo around it.
    let deposit = base * (energy * 3.0 + glow * 0.35)
                + vec3<f32>(1.0, 1.0, 1.0) * energy * energy * 0.8;

    // ── Phosphor persistence via feedback ──────────────────────────────────
    let decay = pow(persistence, u.dt);
    let prev  = prev_pixel(in.uv).rgb * decay;

    // Dim crosshair, injected with (1 - decay) compensation so its steady
    // state brightness is frame-rate independent.
    let axis_w = 1.5 / half;
    let axis = (exp(-sq.y * sq.y / (axis_w * axis_w))
              + exp(-sq.x * sq.x / (axis_w * axis_w))) * 0.05;
    let floor_light = base * axis;

    let col = max(prev + floor_light * (1.0 - decay), vec3<f32>(0.0)) + deposit;
    return vec4<f32>(min(col, vec3<f32>(4.0)), 1.0);
}

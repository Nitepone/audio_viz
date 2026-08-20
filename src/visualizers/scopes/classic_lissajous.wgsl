// classic_lissajous.wgsl — XY phosphor oscilloscope fragment shader.
//
// Params:
//   u.params[0].x  gain         — amplitude multiplier for both channels
//   u.params[0].y  persistence  — phosphor brightness retained per second
//   u.params[0].z  theme        — 0 = green (P31), 1 = amber (P3), 2 = white (P4)
//   u.params[0].w  focus        — beam width multiplier (lower = sharper)
//   u.params[1].x  n_samples    — length of the shown window, in samples
//   u.params[1].y  orientation  — 0 = left→X / right→Y, 1 = mid/side (rotated 45°)
//   u.params[1].zw bbox_min     — trace bounding box (min) in square coords
//   u.params[2].xy bbox_max     — trace bounding box (max) in square coords
//
// The beam is drawn analytically: a Gaussian-profile line integral is
// accumulated from every consecutive sample segment.  Unlike per-point
// sampling the integral telescopes across shared vertices, so joints neither
// double-count nor bead and segment ends fall off smoothly.  Contributions
// are weighted by inverse beam speed, so slow passages deposit more
// "phosphor" — the behaviour of a real CRT.  Persistence uses the engine's
// previous-frame feedback texture.

const N_POINTS: i32 = 512;
// Segments longer than this (square-coord units) are faded out: a big jump
// between consecutive samples is a transient, not a curve the beam traced, so
// drawing it as a straight chord would be a false line across the figure.
const RETRACE_MAX: f32 = 0.35;

fn beam_color(theme: i32) -> vec3<f32> {
    if (theme == 1) { return vec3<f32>(1.0, 0.62, 0.12); }   // amber
    if (theme == 2) { return vec3<f32>(0.92, 0.96, 1.0); }   // white
    return vec3<f32>(0.20, 1.0, 0.35);                       // green
}

// tanh-based erf approximation (|error| < 5e-4 over the range we use).
// The argument must be clamped: erf saturates to ±1 by |x| ≈ 4 anyway, and
// larger inputs overflow Metal's fast-math tanh (exp(2x) → inf/inf = NaN),
// which paints NaN-black over every pixel near a segment's extended line.
fn erf_approx(x: f32) -> f32 {
    let t = clamp(x, -4.0, 4.0);
    return tanh(1.128379167 * t + 0.10281 * t * t * t);
}

// Path point i (0 = oldest shown) in centered square coordinates.  The window
// spans n_samples ending at the newest sample; N_POINTS control points are
// spread across it (fractional stride).  y increases downward on screen.
fn path_pt(i: i32, gain: f32, n_samples: f32, orient: i32) -> vec2<f32> {
    let stride_f = n_samples / f32(N_POINTS);
    let base     = f32(AUDIO_SAMPLES) - n_samples;
    let j        = i32(base + f32(i) * stride_f);
    let l = audio_sample(ROW_LEFT, j) * gain;
    let r = audio_sample(ROW_RIGHT, j) * gain;
    if (orient == 1) {
        // Mid/side: mono (mid) runs vertically, stereo width horizontally.
        return vec2<f32>((l - r) * 0.70710678, -(l + r) * 0.70710678);
    }
    return vec2<f32>(l, -r);
}

// Gaussian line-integral coverage of segment a→b at pixel p, for the core
// (sigma) and glow (glow_sigma) widths.  Returns (core, glow, seg_len); each
// coverage term is ~[0,1] and telescopes across shared vertices.  Segments
// whose nearest point is beyond `cut` contribute nothing (cheap early-out
// before the transcendental calls).
fn seg_cover(a: vec2<f32>, b: vec2<f32>, p: vec2<f32>,
             sigma: f32, glow_sigma: f32, cut: f32) -> vec3<f32> {
    let ba      = b - a;
    let len     = length(ba);
    let inv_len = 1.0 / max(len, 1e-9);
    let dir     = ba * inv_len;
    let pa      = p - a;
    let hlen    = dot(pa, dir);
    let perp    = pa - dir * hlen;
    let d2      = dot(perp, perp);
    if (d2 >= cut) {
        return vec3<f32>(0.0, 0.0, len);
    }
    let ua = -hlen;
    let ub = len - hlen;
    let is = 1.0 / sigma;
    let ig = 1.0 / glow_sigma;
    let core = exp(-d2 * is * is) * 0.5 * (erf_approx(ub * is) - erf_approx(ua * is));
    let glow = exp(-d2 * ig * ig) * 0.5 * (erf_approx(ub * ig) - erf_approx(ua * ig));
    return vec3<f32>(core, glow, len);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let gain        = u.params[0].x;
    let persistence = max(u.params[0].y, 0.001);
    let theme       = i32(u.params[0].z + 0.5);
    let focus       = u.params[0].w;
    let n_samples   = clamp(u.params[1].x, 64.0, f32(AUDIO_SAMPLES));
    let orient      = i32(u.params[1].y + 0.5);
    let bbox_min    = u.params[1].zw;
    let bbox_max    = u.params[2].xy;

    // Centered square coordinates: shorter window axis spans [-1, 1].
    let res  = u.resolution;
    let p_px = in.uv * res;
    let half = 0.5 * min(res.x, res.y);
    let sq   = (p_px - 0.5 * res) / half;   // y increases downward

    // Beam width in square-space units (focus 1.0 ≈ 2.5 px on a 1080p target).
    let sigma      = focus * 5.0 / half;
    let glow_sigma = sigma * 3.0;

    // Beyond this squared distance the glow Gaussian is invisible (exp(-12)).
    let cut = 12.0 * glow_sigma * glow_sigma;
    let pad = sqrt(cut);

    // ── Accumulate beam energy over the sample path ────────────────────────
    // Skip the whole loop when the pixel is outside the trace's bounding box
    // (padded by the glow radius) — near-free frames on quiet passages.
    var energy = 0.0;
    var glow   = 0.0;
    if (all(sq >= bbox_min - pad) && all(sq <= bbox_max + pad)) {
        var prev_pt = path_pt(0, gain, n_samples, orient);
        for (var i = 1; i < N_POINTS; i = i + 1) {
            let pt  = path_pt(i, gain, n_samples, orient);
            let sc  = seg_cover(prev_pt, pt, sq, sigma, glow_sigma, cut);
            let len = sc.z;
            // Slow beam ⇒ more dwell time ⇒ brighter deposit; long jumps blank.
            let dwell = 0.004 / (0.004 + len);
            // Fade out long jumps (transients).  smoothstep needs edge0<edge1
            // — reversed edges are undefined in WGSL — so invert it instead.
            let vis   = 1.0 - smoothstep(RETRACE_MAX * 0.4, RETRACE_MAX, len);
            energy = energy + sc.x * dwell * vis;
            glow   = glow   + sc.y * dwell * vis;
            prev_pt = pt;
        }
    }

    let base = beam_color(theme);
    // White-hot core on strong deposits, coloured halo around it.  The analytic
    // coverage peaks at erf(seg_len/2σ) — below 1 for the short segments that
    // dominate — so these multipliers run above the old point-sampled tuning.
    let deposit = base * (energy * 6.0 + glow * 0.14)
                + vec3<f32>(1.0, 1.0, 1.0) * energy * energy * 0.7;

    // ── Phosphor persistence via feedback ──────────────────────────────────
    // max() model: fresh deposits stamp at their true brightness and the
    // previous frame only fades.  Additive feedback (prev + deposit) would
    // amplify any slow-moving trace to deposit/(1-decay) — tens of times
    // over — which read as runaway bloom.  phosphor_fade (prelude.wgsl)
    // accelerates the decay as the trace dims, killing faint ghosts fast.
    let prev = phosphor_fade(prev_pixel(in.uv).rgb, persistence, u.dt);

    // Dim crosshair.
    let axis_w = 1.5 / half;
    let axis = (exp(-sq.y * sq.y / (axis_w * axis_w))
              + exp(-sq.x * sq.x / (axis_w * axis_w))) * 0.05;
    let floor_light = base * axis;

    let col = max(prev, deposit + floor_light);
    return vec4<f32>(min(col, vec3<f32>(4.0)), 1.0);
}

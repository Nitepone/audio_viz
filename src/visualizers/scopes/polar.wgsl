// polar.wgsl — Polar waveform / spectrum oscilloscope fragment shader.
//
// Params:
//   u.params[0].x  gain         — amplitude multiplier
//   u.params[0].y  persistence  — phosphor brightness retained per second
//   u.params[0].z  theme        — 0 = green (P31), 1 = amber (P3), 2 = white (P4)
//   u.params[0].w  focus        — beam width multiplier (lower = sharper)
//   u.params[1].x  base_radius  — zero-amplitude ring radius as a fraction of
//                                 the maximum usable radius
//   u.params[1].y  n_samples    — length of the shown window, in samples
//   u.params[1].z  source       — 0 = waveform, 1 = spectrum (radar)
//   u.params[1].w  r_inner      — smallest ring radius reached, square coords
//   u.params[2].x  r_outer      — largest ring radius reached, square coords
//
// Waveform mode bends the mono signal into a circle: time maps to angle (the
// most recent n_samples per revolution) and amplitude modulates the radius
// around a base ring.  Spectrum mode maps angle to log-frequency and radius
// to magnitude, mirrored so the ring closes — a circular spectrum "radar".
// The beam is drawn analytically like classic_lissajous: a Gaussian-profile
// line integral per segment, dwell-weighted, with feedback persistence.

const N_POINTS: i32 = 512;
const TAU: f32 = 6.28318530718;
// Segments longer than this (square-coord units) fade out — see lissajous.
const RETRACE_MAX: f32 = 0.5;
// Spectrum radar sweep range.
const SPEC_LO_HZ: f32 = 30.0;
const SPEC_HI_HZ: f32 = 16000.0;

fn beam_color(theme: i32) -> vec3<f32> {
    if (theme == 1) { return vec3<f32>(1.0, 0.62, 0.12); }   // amber
    if (theme == 2) { return vec3<f32>(0.92, 0.96, 1.0); }   // white
    return vec3<f32>(0.20, 1.0, 0.35);                       // green
}

// The argument must be clamped: erf saturates to ±1 by |x| ≈ 4 anyway, and
// larger inputs overflow Metal's fast-math tanh (exp(2x) → inf/inf = NaN),
// which paints NaN-black over every pixel near a segment's extended line.
fn erf_approx(x: f32) -> f32 {
    let t = clamp(x, -4.0, 4.0);
    return tanh(1.128379167 * t + 0.10281 * t * t * t);
}

// Normalise a linear FFT magnitude to [0,1] over a dB window.  The engine's
// magnitudes are windowed and 1/N-scaled, so a full-scale tone peaks near
// -12 dB; -60 dB is effectively the noise floor.
fn mag_to_frac(v: f32) -> f32 {
    let db = 6.0205999 * log2(max(v, 1e-9));   // 20*log10(v)
    return clamp((db + 60.0) / 48.0, 0.0, 1.0);
}

// Polar sample point i (0..N_POINTS) in centered square coordinates.
fn polar_pt(i: i32, gain: f32, r_base: f32, r_amp: f32,
            n_samples: f32, source: i32) -> vec2<f32> {
    let theta = TAU * f32(i) / f32(N_POINTS);
    var r: f32;
    if (source == 1) {
        // Spectrum: sweep log-frequency out and back so the ring is seamless.
        let frac = f32(i) / f32(N_POINTS);
        let m    = 1.0 - abs(2.0 * frac - 1.0);            // 0 → 1 → 0 triangle
        let freq = SPEC_LO_HZ * pow(SPEC_HI_HZ / SPEC_LO_HZ, m);
        let bin  = i32(freq * f32(AUDIO_SAMPLES) / u.sample_rate);
        r = r_base + mag_to_frac(audio_sample(ROW_FFT, bin)) * r_amp;
    } else {
        let stride_f = n_samples / f32(N_POINTS);
        let base     = f32(AUDIO_SAMPLES) - n_samples;
        let j        = i32(base + f32(i) * stride_f);
        let amp      = clamp(audio_sample(ROW_MONO, j) * gain, -1.0, 1.0);
        r = r_base + amp * r_amp;
    }
    return vec2<f32>(cos(theta), sin(theta)) * r;
}

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
    let base_frac   = clamp(u.params[1].x, 0.05, 0.95);
    let n_samples   = clamp(u.params[1].y, 64.0, f32(AUDIO_SAMPLES));
    let source      = i32(u.params[1].z + 0.5);
    let r_inner     = u.params[1].w;
    let r_outer     = u.params[2].x;

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

    let cut = 12.0 * glow_sigma * glow_sigma;
    let pad = sqrt(cut);

    // ── Accumulate beam energy over the closed polar path ──────────────────
    // Radial cull: the trace lives in an annulus, so skip the loop for pixels
    // inside the hole or outside the rim (padded by the glow radius).
    var energy = 0.0;
    var glow   = 0.0;
    let rr = length(sq);
    if (rr >= r_inner - pad && rr <= r_outer + pad) {
        var prev_pt = polar_pt(0, gain, r_base, r_amp, n_samples, source);
        for (var i = 1; i <= N_POINTS; i = i + 1) {
            // i == N_POINTS wraps to point 0, closing the ring.
            let pt  = polar_pt(i % N_POINTS, gain, r_base, r_amp, n_samples, source);
            let sc  = seg_cover(prev_pt, pt, sq, sigma, glow_sigma, cut);
            let len = sc.z;
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
    let prev = phosphor_fade(prev_pixel(in.uv).rgb, persistence, u.dt);

    // Dim zero-amplitude reference ring.
    let ring_w = 1.5 / half;
    let dr = rr - r_base;
    let ring = exp(-dr * dr / (ring_w * ring_w)) * 0.05;
    let floor_light = base * ring;

    let col = max(prev, deposit + floor_light);
    return vec4<f32>(min(col, vec3<f32>(4.0)), 1.0);
}

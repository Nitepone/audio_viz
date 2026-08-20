// radial.wgsl — Polar spectrum radiating from the centre.
//
// Angle selects a log-spaced frequency band; the normalised radius is
// compared against that band's energy so the spectrum fans out from the
// core.  A soft rim glow rides each band's edge, a beat flash brightens the
// whole disc, and the previous frame is faded back in for phosphor-style
// persistence.  See prelude.wgsl for the uniform / audio-texture contract.
//
// params[0] = (gain, persistence, color_scheme, symmetry)
// params[1] = (rotate, _, _, _)

const PI: f32 = 3.14159265359;
const TAU: f32 = 6.28318530718;

fn log10(x: f32) -> f32 {
    return log2(max(x, 1e-9)) * 0.30102999566;
}

// Linear FFT magnitude → normalised 0..1 over a −72…−12 dB window.
fn db_frac(mag: f32) -> f32 {
    let db = 20.0 * log10(mag);
    return clamp((db + 72.0) / 60.0, 0.0, 1.0);
}

// Interpolated FFT magnitude at a normalised angle (log-spaced 30 Hz–18 kHz).
fn fft_at(ang01: f32) -> f32 {
    let freq = 30.0 * pow(600.0, clamp(ang01, 0.0, 1.0)); // 30 * (18000/30)^ang
    let bin = freq * f32(AUDIO_SAMPLES) / u.sample_rate;
    let i0 = i32(floor(bin));
    let t = bin - floor(bin);
    return mix(audio_sample(ROW_FFT, i0), audio_sample(ROW_FFT, i0 + 1), t);
}

fn mix4(t: f32, c0: vec3<f32>, c1: vec3<f32>, c2: vec3<f32>, c3: vec3<f32>) -> vec3<f32> {
    let s = clamp(t, 0.0, 1.0) * 3.0;
    if (s < 1.0) { return mix(c0, c1, s); }
    if (s < 2.0) { return mix(c1, c2, s - 1.0); }
    return mix(c2, c3, s - 2.0);
}

fn palette(scheme: f32, t: f32) -> vec3<f32> {
    if (scheme < 0.5) {
        // spectrum — smooth cosine rainbow
        return 0.5 + 0.5 * cos(TAU * (t + vec3<f32>(0.0, 0.33, 0.67)));
    } else if (scheme < 1.5) {
        // heat
        return mix4(t, vec3<f32>(0.0), vec3<f32>(0.7, 0.0, 0.0),
                       vec3<f32>(1.0, 0.5, 0.0), vec3<f32>(1.0, 1.0, 0.85));
    } else if (scheme < 2.5) {
        // ice
        return mix4(t, vec3<f32>(0.0), vec3<f32>(0.0, 0.2, 0.6),
                       vec3<f32>(0.0, 0.7, 0.95), vec3<f32>(0.85, 0.98, 1.0));
    }
    // phosphor
    return mix4(t, vec3<f32>(0.0), vec3<f32>(0.05, 0.5, 0.15),
                   vec3<f32>(0.3, 0.9, 0.35), vec3<f32>(0.85, 1.0, 0.8));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let gain        = u.params[0].x;
    let persistence = u.params[0].y;
    let scheme      = u.params[0].z;
    let symmetry    = u.params[0].w;
    let rot         = u.params[1].x;

    let res = u.resolution;
    var p = in.uv - vec2<f32>(0.5, 0.5);
    p.x = p.x * (res.x / res.y);       // keep the disc circular
    let r = length(p) / 0.5;           // 0 at centre, 1 at the vertical edge

    let ang = atan2(p.y, p.x) + rot * u.time;
    var a01 = fract((ang + PI) / TAU);
    if (symmetry > 0.5) {
        a01 = abs(a01 * 2.0 - 1.0);    // fold left↔right for a symmetric bloom
    }

    let energy = clamp(db_frac(fft_at(a01)) * gain, 0.0, 1.0);

    // Colour by frequency (rainbow) for spectrum, by depth for the others.
    let ct = select(1.0 - clamp(r, 0.0, 1.0), a01, scheme < 0.5);
    let base = palette(scheme, ct);

    var col = vec3<f32>(0.0);
    if (r < energy && r < 1.0) {
        let bright = mix(0.4, 1.0, 1.0 - r) + u.beat * 0.25;
        col = base * bright;
    }

    // Soft rim glow riding the edge of each band.
    if (energy > 0.02) {
        col += base * exp(-abs(r - energy) * 14.0) * 0.7;
    }

    // Feedback persistence for smooth temporal motion.
    let faded = phosphor_fade(prev_pixel(in.uv).rgb, persistence, u.dt);
    return vec4<f32>(max(col, faded), 1.0);
}

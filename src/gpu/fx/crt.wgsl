// crt.wgsl — CRT monitor post effect.
//
// Params (fx.params[0]):
//   .x  curvature — 0..1: barrel distortion, edge vignette, chromatic
//                   aberration and the black tube bezel outside the face
//   .y  scanlines — 0..1: horizontal scanline darkening (~3 px pitch)
//   .z  mask      — 0..1: aperture-grille RGB stripe strength

const PI: f32 = 3.14159265;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let curvature = fx.params[0].x;
    let scanlines = fx.params[0].y;
    let mask      = fx.params[0].z;

    // ── Barrel distortion: bow the sample coordinates outward ──────────────
    let cc  = in.uv * 2.0 - 1.0;
    let r2  = dot(cc, cc);
    let bow = cc * (1.0 + curvature * 0.16 * r2);
    let uv  = bow * 0.5 + 0.5;

    // ── Chromatic aberration, growing towards the edges ────────────────────
    let ab = cc * r2 * curvature * 0.004;
    var col = vec3<f32>(
        textureSample(src, src_sampler, uv + ab).r,
        textureSample(src, src_sampler, uv).g,
        textureSample(src, src_sampler, uv - ab).b,
    );

    // ── Scanlines: follow the curved tube face ──────────────────────────────
    let line_pitch = max(fx.resolution.y / 3.0, 8.0);
    let s = 0.5 - 0.5 * cos(uv.y * line_pitch * 2.0 * PI);
    col = col * (1.0 - scanlines * 0.45 * s);

    // ── Aperture grille: RGB stripe triads on screen columns ────────────────
    let stripe = u32(in.uv.x * fx.resolution.x) % 3u;
    var grille = vec3<f32>(1.0 - mask * 0.65);
    if (stripe == 0u) {
        grille.r = 1.0;
    } else if (stripe == 1u) {
        grille.g = 1.0;
    } else {
        grille.b = 1.0;
    }
    col = col * grille;

    // Compensate the brightness lost to scanlines and the grille.
    col = col * (1.0 + scanlines * 0.30 + mask * 0.40);

    // ── Vignette + tube bezel (black outside the curved face) ──────────────
    col = col * (1.0 - curvature * 0.35 * r2 * r2);
    let edge = max(abs(bow.x), abs(bow.y));
    col = col * (1.0 - smoothstep(0.996, 1.0, edge));

    return vec4<f32>(col, 1.0);
}

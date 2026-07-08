/// palette.rs — RGB colour gradients for software visualizers.
///
/// The terminal app used 256-colour ANSI palette indices; here palettes are
/// true-colour gradient stops interpolated linearly in sRGB space.

/// A palette is a list of sRGB stops, evenly spaced over [0, 1].
pub type Palette = &'static [[u8; 3]];

/// Black → deep red → orange → yellow → white ("heat").
pub const PALETTE_HEAT: Palette = &[
    [0, 0, 0],
    [80, 0, 0],
    [160, 10, 0],
    [220, 60, 0],
    [255, 130, 0],
    [255, 200, 40],
    [255, 245, 150],
    [255, 255, 255],
];

/// Black → deep blue → cyan → white ("ice").
pub const PALETTE_ICE: Palette = &[
    [0, 0, 0],
    [0, 10, 70],
    [0, 40, 140],
    [0, 90, 200],
    [0, 160, 240],
    [80, 220, 255],
    [190, 245, 255],
    [255, 255, 255],
];

/// Black → green → yellow-green → white ("phosphor", classic CRT).
pub const PALETTE_PHOSPHOR: Palette = &[
    [0, 0, 0],
    [0, 40, 0],
    [0, 90, 10],
    [0, 160, 30],
    [40, 220, 60],
    [140, 255, 120],
    [230, 255, 210],
];

/// Full rainbow sweep, red → violet ("spectrum").
pub const PALETTE_SPECTRUM: Palette = &[
    [0, 0, 0],
    [120, 0, 0],
    [255, 40, 0],
    [255, 160, 0],
    [255, 255, 0],
    [60, 220, 60],
    [0, 180, 220],
    [40, 60, 255],
    [140, 60, 255],
];

/// Greyscale ("mono").
pub const PALETTE_MONO: Palette = &[[0, 0, 0], [255, 255, 255]];

/// Interpolated palette lookup by fractional position [0, 1].
#[inline]
pub fn palette_lookup(frac: f32, palette: Palette) -> [u8; 3] {
    let frac = frac.clamp(0.0, 1.0);
    let n = palette.len();
    if n == 1 {
        return palette[0];
    }
    let pos = frac * (n - 1) as f32;
    let i = (pos as usize).min(n - 2);
    let t = pos - i as f32;
    let a = palette[i];
    let b = palette[i + 1];
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t) as u8,
    ]
}

/// Resolve a palette by config name; defaults to heat.
pub fn palette_by_name(name: &str) -> Palette {
    match name {
        "ice" => PALETTE_ICE,
        "phosphor" => PALETTE_PHOSPHOR,
        "spectrum" => PALETTE_SPECTRUM,
        "mono" => PALETTE_MONO,
        _ => PALETTE_HEAT,
    }
}

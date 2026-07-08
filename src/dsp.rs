/// dsp.rs — Shared audio DSP helpers (ported from the terminal app's
/// visualizer_utils.rs, minus the ANSI rendering primitives).

use crate::visualizer::{FFT_SIZE, SAMPLE_RATE};

/// Root-mean-square of a sample slice.
#[inline]
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum = samples.iter().fold(0.0f32, |acc, &v| acc + v * v);
    (sum / samples.len() as f32).sqrt()
}

/// Convert a frequency in Hz to an FFT bin index, clamped to valid range.
#[inline]
pub fn freq_to_bin(freq_hz: f32, n_bins: usize) -> usize {
    let freq_res = SAMPLE_RATE as f32 / FFT_SIZE as f32;
    ((freq_hz / freq_res) as usize).clamp(1, n_bins - 1)
}

/// Compute RMS energy of an FFT slice between two frequencies.
#[inline]
#[allow(dead_code)]
pub fn band_energy(fft: &[f32], lo_hz: f32, hi_hz: f32) -> f32 {
    let n = fft.len();
    let lo = freq_to_bin(lo_hz, n);
    let hi = freq_to_bin(hi_hz, n).max(lo + 1);
    rms(&fft[lo..hi.min(n)])
}

/// Convert a linear magnitude to a normalised [0, 1] fraction via dB scale.
#[inline]
pub fn mag_to_frac(v: f32, db_floor: f32, db_ceil: f32) -> f32 {
    let db = 20.0 * v.max(1e-9).log10();
    ((db - db_floor) / (db_ceil - db_floor)).clamp(0.0, 1.0)
}

/// Exponential moving average with asymmetric rise/fall coefficients.
#[inline]
#[allow(dead_code)]
pub fn smooth_asymmetric(current: f32, target: f32, rise: f32, fall: f32) -> f32 {
    let a = if target > current { rise } else { fall };
    a * current + (1.0 - a) * target
}

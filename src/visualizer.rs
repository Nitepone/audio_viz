/// visualizer.rs — The Visualizer trait and all shared data types.
///
/// This file is intentionally kept free of application logic.  Its only job
/// is to define the stable interface between the core engine (main.rs) and
/// individual visualizers (src/visualizers/*.rs).
///
/// Future runtime-plugin path
/// ──────────────────────────
/// When runtime plugins are desired, extract this file into a separate
/// `audio_viz_core` crate in a Cargo workspace.  Both the main binary and
/// plugin dylibs depend on that crate.  Plugins export:
///
///   #[no_mangle]
///   pub extern "C" fn viz_register() -> *mut Vec<Box<dyn Visualizer>> {
///       Box::into_raw(Box::new(register()))
///   }
///
/// and main.rs uses `libloading` to dlopen them at startup.  Nothing about
/// the trait itself needs to change.

// ── Shared constants ──────────────────────────────────────────────────────────

/// Audio sample rate used throughout the application.
pub const SAMPLE_RATE: u32 = 44_100;

/// FFT window size.  Must be a power of two for rustfft efficiency.
/// 4096 samples @ 44100 Hz → ~93 ms of audio per analysis frame,
/// giving ~10.8 Hz frequency resolution.
pub const FFT_SIZE: usize = 4_096;

/// Number of audio channels captured (stereo).
pub const CHANNELS: usize = 2;

/// Target render rate in frames per second.
pub const FPS_TARGET: f32 = 45.0;

// ── Spectrum bar dynamics (used by the shared SpectrumBars helper) ────────────

/// EMA coefficient applied when a bar is rising (fast attack).
pub const RISE_COEFF: f32 = 0.80;
/// EMA coefficient applied when a bar is falling (slower decay).
pub const FALL_COEFF: f32 = 0.55;
/// Seconds a peak marker stays at its maximum before starting to fall.
pub const PEAK_HOLD_SECS: f32 = 1.2;
/// Normalised units per frame that a peak marker falls after hold expires.
pub const PEAK_DROP_RATE: f32 = 0.018;
/// dB floor for the bar normalisation range.
pub const DB_MIN: f32 = -72.0;
/// dB ceiling for the bar normalisation range.
pub const DB_MAX: f32 = -12.0;

// ── Colour palette (shared, mirrors Python _SPEC) ─────────────────────────────

/// 256-colour gradient: red (bass) → yellow/green (mid) → cyan/blue (treble).
/// Index with specgrad(frac) where frac ∈ [0, 1].
pub const SPEC_GRADIENT: &[u8] = &[
    196, 202, 208, 214, 220, 226, 190, 154, 118, 82, 46, 47, 48, 49, 50, 51,
    45, 39, 33, 27, 21, 57, 93, 129,
];

/// Map a frequency-position fraction [0,1] to a 256-colour code.
#[inline]
pub fn specgrad(frac: f32) -> u8 {
    let i = ((frac * (SPEC_GRADIENT.len() - 1) as f32) as usize)
        .min(SPEC_GRADIENT.len() - 1);
    SPEC_GRADIENT[i]
}

// ── Terminal size ─────────────────────────────────────────────────────────────

/// Terminal dimensions at the time of a render call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    pub rows: u16,
    pub cols: u16,
}

// ── Audio frame ───────────────────────────────────────────────────────────────

/// One frame of audio data passed to every visualizer tick.
///
/// The FFT magnitude spectrum (`fft`) is computed **once** in the audio
/// pipeline and shared here so individual visualizers never need to import
/// rustfft directly.  The spectrum has `FFT_SIZE / 2 + 1` bins (rfft output).
pub struct AudioFrame {
    /// Left channel samples, normalised to [-1, 1].  Length = FFT_SIZE.
    pub left: Vec<f32>,
    /// Right channel samples, normalised to [-1, 1].  Length = FFT_SIZE.
    pub right: Vec<f32>,
    /// Mono mix = (left + right) * 0.5.  Length = FFT_SIZE.
    pub mono: Vec<f32>,
    /// Real FFT magnitude spectrum of `mono`, Hann-windowed.
    /// Length = FFT_SIZE / 2 + 1.
    pub fft: Vec<f32>,
    /// Sample rate (always SAMPLE_RATE; carried for convenience).
    pub sample_rate: u32,
}

// ── The core trait ────────────────────────────────────────────────────────────

/// Every visualizer implements this trait.
///
/// Lifecycle
/// ─────────
/// 1. `new()` (not part of the trait — each visualizer defines its own)
/// 2. `tick()` is called every render frame with fresh audio
/// 3. `render()` is called immediately after `tick()` to produce output lines
/// 4. `on_resize()` is called whenever the terminal size changes
///
/// Thread safety
/// ─────────────
/// Visualizers are not called from multiple threads simultaneously.
/// The `Send` bound is required so the `Box<dyn Visualizer>` can be moved
/// into the render thread.
pub trait Visualizer: Send {
    /// Short lowercase identifier, e.g. `"spectrum"`.
    /// Used for CLI selection and the `--list` output.
    fn name(&self) -> &str;

    /// One-line human description shown in `--list`.
    fn description(&self) -> &str;

    /// Advance simulation state by `dt` seconds using the provided audio frame.
    /// Called once per render frame before `render()`.
    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: TermSize);

    /// Produce one String per terminal row.  The returned Vec must have
    /// exactly `size.rows as usize` entries (pad with spaces if needed).
    /// Each string must be exactly `size.cols as usize` display-columns wide
    /// (ANSI escape codes do not count toward display width).
    fn render(&self, size: TermSize, fps: f32) -> Vec<String>;

    /// Called when the terminal is resized before the next `tick`.
    /// Default implementation is a no-op; override to invalidate caches.
    fn on_resize(&mut self, _size: TermSize) {}
}

// ── Shared DSP helpers ────────────────────────────────────────────────────────

/// Compute log-spaced FFT bin ranges for `n_bars` bars.
///
/// Returns `(lo_bins, hi_bins)` where each entry is a bin index into the
/// rfft magnitude array (length = FFT_SIZE / 2 + 1).
///
/// Mirrors Python `build_binmap(n, fmin, fmax)`.
pub fn build_binmap(n_bars: usize, fmin: f32, fmax: f32) -> (Vec<usize>, Vec<usize>) {
    let n_bins   = FFT_SIZE / 2 + 1;
    let freq_res = SAMPLE_RATE as f32 / FFT_SIZE as f32; // Hz per bin

    let log_lo = fmin.log10();
    let log_hi = fmax.log10();

    let mut lo_bins = Vec::with_capacity(n_bars);
    let mut hi_bins = Vec::with_capacity(n_bars);

    for i in 0..n_bars {
        let edge_lo = 10f32.powf(log_lo + (log_hi - log_lo) * i as f32 / n_bars as f32);
        let edge_hi = 10f32.powf(log_lo + (log_hi - log_lo) * (i + 1) as f32 / n_bars as f32);

        let lo = ((edge_lo / freq_res) as usize).clamp(1, n_bins - 2);
        let hi = ((edge_hi / freq_res) as usize).clamp(2, n_bins - 1);
        let hi = hi.max(lo + 1);

        lo_bins.push(lo);
        hi_bins.push(hi);
    }

    (lo_bins, hi_bins)
}

/// Convert FFT magnitude spectrum to normalised bar heights [0, 1].
///
/// Each bar takes the RMS of its bin range, converts to dB, and normalises
/// to [DB_MIN, DB_MAX].
///
/// Mirrors Python `spec_to_bars(spec, lo, hi)`.
pub fn spec_to_bars(fft: &[f32], lo_bins: &[usize], hi_bins: &[usize]) -> Vec<f32> {
    lo_bins
        .iter()
        .zip(hi_bins.iter())
        .map(|(&lo, &hi)| {
            let slice = &fft[lo..hi.min(fft.len())];
            if slice.is_empty() {
                return 0.0;
            }
            let rms = (slice.iter().map(|v| v * v).sum::<f32>() / slice.len() as f32).sqrt();
            let db  = 20.0 * rms.max(1e-9).log10();
            ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0)
        })
        .collect()
}

// ── Shared per-visualizer spectrum bar state ──────────────────────────────────

/// Maintains smoothed bar heights and peak-hold markers for one set of bars.
///
/// Most visualizers hold one of these in their struct and call
/// `update(fft, n_bars, dt)` each tick to get the current bar values.
pub struct SpectrumBars {
    pub smoothed: Vec<f32>, // EMA-smoothed normalised bar heights [0,1]
    pub peaks:    Vec<f32>, // Peak-hold values [0,1]
    peak_timers:  Vec<f32>, // Seconds since each peak was last refreshed
    lo_bins:      Vec<usize>,
    hi_bins:      Vec<usize>,
    n_bars:       usize,
}

impl SpectrumBars {
    pub fn new(n_bars: usize) -> Self {
        let (lo, hi) = build_binmap(n_bars, 30.0, 18_000.0);
        Self {
            smoothed:    vec![0.0; n_bars],
            peaks:       vec![0.0; n_bars],
            peak_timers: vec![0.0; n_bars],
            lo_bins:     lo,
            hi_bins:     hi,
            n_bars,
        }
    }

    /// Rebuild bin mappings when the number of bars changes (terminal resize).
    pub fn resize(&mut self, n_bars: usize) {
        if n_bars == self.n_bars {
            return;
        }
        *self = Self::new(n_bars);
    }

    /// Update smoothed heights and peak markers from a fresh FFT frame.
    pub fn update(&mut self, fft: &[f32], dt: f32) {
        let norm = spec_to_bars(fft, &self.lo_bins, &self.hi_bins);

        for i in 0..self.n_bars {
            let n = norm[i];
            // Asymmetric EMA: rise fast, fall slower
            let a = if n > self.smoothed[i] { RISE_COEFF } else { FALL_COEFF };
            self.smoothed[i] = a * self.smoothed[i] + (1.0 - a) * n;

            // Peak hold
            if self.smoothed[i] > self.peaks[i] {
                self.peaks[i]       = self.smoothed[i];
                self.peak_timers[i] = 0.0;
            } else {
                self.peak_timers[i] += dt;
                if self.peak_timers[i] > PEAK_HOLD_SECS {
                    self.peaks[i] = (self.peaks[i] - PEAK_DROP_RATE).max(0.0);
                }
            }
        }
    }
}

// ── ANSI rendering helpers ────────────────────────────────────────────────────

/// Build the status bar string (bottom row) common to all visualizers.
pub fn status_bar(cols: usize, fps: f32, name: &str, source: &str, extra: &str) -> String {
    let raw = format!(
        " {:4.0} fps | {}{} | {}",
        fps,
        name,
        extra,
        &source[..source.len().min(cols.saturating_sub(30))],
    );
    let displayed = raw.chars().take(cols).collect::<String>();
    let padded    = format!("{:<width$}", displayed, width = cols);
    format!("\x1b[2m\x1b[38;5;240m{}\x1b[0m", padded)
}

/// A full-width horizontal rule in a dim colour.
pub fn hline(cols: usize, color: u8) -> String {
    format!("\x1b[2m\x1b[38;5;{color}m{}\x1b[0m", "-".repeat(cols))
}

/// A centred title string.
pub fn title_line(cols: usize, text: &str, color: u8) -> String {
    let pad = cols.saturating_sub(text.len()) / 2;
    format!("\x1b[1m\x1b[38;5;{color}m{}{}\x1b[0m", " ".repeat(pad), text)
}

/// Pad or truncate a Vec<String> to exactly `rows` entries of width `cols`.
pub fn pad_frame(mut lines: Vec<String>, rows: usize, cols: usize) -> Vec<String> {
    let blank = " ".repeat(cols);
    lines.truncate(rows);
    while lines.len() < rows {
        lines.push(blank.clone());
    }
    lines
}

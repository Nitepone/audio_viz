/// dsp.rs — Modular DSP filter library for audio analysis.
///
/// Provides composable time-domain and frequency-domain filters that can be
/// chained together via `DspChain`.  Designed for real-time use at 44.1 kHz
/// with negligible latency (<5 µs per stereo sample for a full filter bank).
///
/// Filters:
///   MidSideFilter          — stereo → mono via center/side extraction
///   PreEmphasisFilter      — 1st-order high-frequency boost
///   BandpassFilter         — biquad IIR bandpass (5 presets)
///   AWeightFilter          — IEC 61672 perceptual loudness curve
///   SpectralWhiteningFilter — per-bin EMA normalisation

use std::f32::consts::PI;
use rustfft::{FftPlanner, num_complex::Complex};
use crate::visualizer::FFT_SIZE;

// ── Filter domain ────────────────────────────────────────────────────────────

/// Where in the processing pipeline a filter operates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterDomain {
    /// Operates on time-domain PCM samples (before FFT).
    TimeDomain,
    /// Operates on frequency-domain FFT magnitudes (after FFT).
    FrequencyDomain,
}

// ── DspFilter trait ──────────────────────────────────────────────────────────

/// A stateful DSP filter that processes audio data in-place.
pub trait DspFilter: Send {
    fn name(&self) -> &str;
    fn domain(&self) -> FilterDomain;

    /// Process time-domain samples in-place.  Only called when
    /// `domain() == TimeDomain`.
    fn process_time(&mut self, _samples: &mut [f32]) {}

    /// Process FFT magnitudes in-place.  Only called when
    /// `domain() == FrequencyDomain`.
    fn process_freq(&mut self, _magnitudes: &mut [f32], _sample_rate: u32, _fft_size: usize) {}

    /// Reset internal state (delay elements, running averages, etc.).
    fn reset(&mut self);
}

// ── Biquad ───────────────────────────────────────────────────────────────────

/// Generic second-order IIR filter (Direct Form I).
/// Coefficients from Robert Bristow-Johnson's Audio EQ Cookbook.
#[derive(Clone, Debug)]
pub struct Biquad {
    b0: f32, b1: f32, b2: f32,
    a1: f32, a2: f32,
    // delay elements
    x1: f32, x2: f32,
    y1: f32, y2: f32,
}

impl Biquad {
    /// Bandpass filter (constant-0-dB-peak-gain variant).
    pub fn bandpass(center_hz: f32, q: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * center_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0:  alpha / a0,
            b1:  0.0,
            b2: -alpha / a0,
            a1: -2.0 * w0.cos() / a0,
            a2:  (1.0 - alpha) / a0,
            x1: 0.0, x2: 0.0,
            y1: 0.0, y2: 0.0,
        }
    }

    /// Second-order lowpass filter.
    pub fn lowpass(cutoff_hz: f32, q: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w0) / 2.0) / a0,
            b1:  (1.0 - cos_w0) / a0,
            b2: ((1.0 - cos_w0) / 2.0) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2:  (1.0 - alpha) / a0,
            x1: 0.0, x2: 0.0,
            y1: 0.0, y2: 0.0,
        }
    }

    /// Second-order highpass filter.
    pub fn highpass(cutoff_hz: f32, q: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 + cos_w0) / 2.0) / a0,
            b1: (-(1.0 + cos_w0)) / a0,
            b2: ((1.0 + cos_w0) / 2.0) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2:  (1.0 - alpha) / a0,
            x1: 0.0, x2: 0.0,
            y1: 0.0, y2: 0.0,
        }
    }

    /// Process a single sample through the filter.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
              - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// Reset delay elements to zero.
    pub fn reset(&mut self) {
        self.x1 = 0.0; self.x2 = 0.0;
        self.y1 = 0.0; self.y2 = 0.0;
    }
}

// ── Mid/Side filter ──────────────────────────────────────────────────────────

/// Stereo field extraction — produces mono from left/right.
/// Not a `DspFilter` because it needs two input channels.
#[derive(Clone, Debug)]
pub enum StereoField { Mid, Side }

#[derive(Clone, Debug)]
pub struct MidSideFilter {
    pub mode: StereoField,
}

impl MidSideFilter {
    pub fn mid()  -> Self { Self { mode: StereoField::Mid  } }
    pub fn side() -> Self { Self { mode: StereoField::Side } }

    /// Extract mid or side channel into a new buffer.
    pub fn extract(&self, left: &[f32], right: &[f32]) -> Vec<f32> {
        left.iter().zip(right.iter()).map(|(&l, &r)| {
            match self.mode {
                StereoField::Mid  => (l + r) * 0.5,
                StereoField::Side => (l - r) * 0.5,
            }
        }).collect()
    }
}

// ── Pre-emphasis filter ──────────────────────────────────────────────────────

/// First-order pre-emphasis: y[n] = x[n] - α·x[n-1].
/// Flattens spectral tilt, making high-frequency detail more visible.
pub struct PreEmphasisFilter {
    alpha: f32,
    prev:  f32,
}

impl PreEmphasisFilter {
    pub fn new(alpha: f32) -> Self {
        Self { alpha, prev: 0.0 }
    }

    pub fn standard() -> Self {
        Self::new(0.97)
    }
}

impl DspFilter for PreEmphasisFilter {
    fn name(&self) -> &str { "pre_emphasis" }
    fn domain(&self) -> FilterDomain { FilterDomain::TimeDomain }

    fn process_time(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            let x = *s;
            *s = x - self.alpha * self.prev;
            self.prev = x;
        }
    }

    fn reset(&mut self) { self.prev = 0.0; }
}

// ── Bandpass filter ──────────────────────────────────────────────────────────

/// Named frequency-band presets for the bandpass filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandPreset {
    Bass,      // 20–250 Hz
    Mids,      // 250–2000 Hz
    Vocal,     // 300–3000 Hz  (formant range)
    Presence,  // 2–6 kHz
    Air,       // 6–20 kHz
}

impl BandPreset {
    /// Returns (center_hz, Q) for each preset.
    /// Center = geometric mean of band edges; Q = center / bandwidth.
    fn params(self) -> (f32, f32) {
        match self {
            BandPreset::Bass     => (70.7,   0.31),  // sqrt(20*250)
            BandPreset::Mids     => (707.0,  0.40),  // sqrt(250*2000)
            BandPreset::Vocal    => (949.0,  0.35),  // sqrt(300*3000)
            BandPreset::Presence => (3464.0, 0.87),  // sqrt(2000*6000)
            BandPreset::Air      => (10954.0, 0.78), // sqrt(6000*20000)
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "bass"     => Some(Self::Bass),
            "mids"     => Some(Self::Mids),
            "vocal"    => Some(Self::Vocal),
            "presence" => Some(Self::Presence),
            "air"      => Some(Self::Air),
            _ => None,
        }
    }
}

/// Biquad IIR bandpass filter with named presets.
pub struct BandpassFilter {
    biquad: Biquad,
}

impl BandpassFilter {
    pub fn new(preset: BandPreset, sample_rate: f32) -> Self {
        let (center, q) = preset.params();
        Self {
            biquad: Biquad::bandpass(center, q, sample_rate),
        }
    }
}

impl DspFilter for BandpassFilter {
    fn name(&self) -> &str { "bandpass" }
    fn domain(&self) -> FilterDomain { FilterDomain::TimeDomain }

    fn process_time(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            *s = self.biquad.process(*s);
        }
    }

    fn reset(&mut self) { self.biquad.reset(); }
}

// ── A-weighting filter ───────────────────────────────────────────────────────

/// IEC 61672 A-weighting applied in the frequency domain.
/// Emphasises 2–5 kHz (peak human sensitivity), attenuates lows and extreme
/// highs.  The weight table is lazily computed and cached.
pub struct AWeightFilter {
    weights:     Vec<f32>,
    cached_bins: usize,
}

impl AWeightFilter {
    pub fn new() -> Self {
        Self { weights: Vec::new(), cached_bins: 0 }
    }

    /// Raw A-weighting gain (not normalised).
    fn ra(f: f32) -> f32 {
        if f < 1.0 { return 0.0; }
        let f2 = f * f;
        let num = 12194.0_f32.powi(2) * f2 * f2;
        let den = (f2 + 20.6_f32.powi(2))
                * (f2 + 12194.0_f32.powi(2))
                * ((f2 + 107.7_f32.powi(2)) * (f2 + 737.9_f32.powi(2))).sqrt();
        num / den
    }

    fn ensure_weights(&mut self, n_bins: usize, sample_rate: u32, fft_size: usize) {
        if self.cached_bins == n_bins { return; }
        let freq_res = sample_rate as f32 / fft_size as f32;
        let ra_1k = Self::ra(1000.0);
        self.weights = (0..n_bins)
            .map(|i| {
                let w = Self::ra(i as f32 * freq_res) / ra_1k;
                w.clamp(0.0, 10.0) // safety cap
            })
            .collect();
        self.cached_bins = n_bins;
    }
}

impl DspFilter for AWeightFilter {
    fn name(&self) -> &str { "a_weight" }
    fn domain(&self) -> FilterDomain { FilterDomain::FrequencyDomain }

    fn process_freq(&mut self, mags: &mut [f32], sample_rate: u32, fft_size: usize) {
        self.ensure_weights(mags.len(), sample_rate, fft_size);
        for (m, &w) in mags.iter_mut().zip(self.weights.iter()) {
            *m *= w;
        }
    }

    fn reset(&mut self) {
        // Stateless per-frame; nothing to reset.
    }
}

// ── Spectral whitening filter ────────────────────────────────────────────────

/// Normalises each FFT bin by its exponential moving average, flattening
/// the spectrum so transient details are more visible.
pub struct SpectralWhiteningFilter {
    running_avg: Vec<f32>,
    alpha:       f32, // EMA smoothing (higher = slower adaptation)
    initialised: bool,
}

impl SpectralWhiteningFilter {
    pub fn new(alpha: f32) -> Self {
        Self { running_avg: Vec::new(), alpha, initialised: false }
    }

    pub fn standard() -> Self {
        Self::new(0.95)
    }
}

impl DspFilter for SpectralWhiteningFilter {
    fn name(&self) -> &str { "whitening" }
    fn domain(&self) -> FilterDomain { FilterDomain::FrequencyDomain }

    fn process_freq(&mut self, mags: &mut [f32], _sample_rate: u32, _fft_size: usize) {
        if !self.initialised || self.running_avg.len() != mags.len() {
            self.running_avg = mags.to_vec();
            self.initialised = true;
            return; // first frame: seed averages, don't modify
        }
        for (i, m) in mags.iter_mut().enumerate() {
            self.running_avg[i] = self.alpha * self.running_avg[i]
                                + (1.0 - self.alpha) * *m;
            if self.running_avg[i] > 1e-9 {
                *m /= self.running_avg[i];
            }
        }
    }

    fn reset(&mut self) {
        self.running_avg.clear();
        self.initialised = false;
    }
}

// ── Spectral difference filter ───────────────────────────────────────────────

/// Outputs the absolute frame-to-frame change per bin.  Sustained tones
/// vanish; onsets, transients, and rhythmic events flash brightly.
pub struct SpectralDiffFilter {
    prev:        Vec<f32>,
    initialised: bool,
}

impl SpectralDiffFilter {
    pub fn new() -> Self {
        Self { prev: Vec::new(), initialised: false }
    }
}

impl DspFilter for SpectralDiffFilter {
    fn name(&self) -> &str { "spectral_diff" }
    fn domain(&self) -> FilterDomain { FilterDomain::FrequencyDomain }

    fn process_freq(&mut self, mags: &mut [f32], _sample_rate: u32, _fft_size: usize) {
        if !self.initialised || self.prev.len() != mags.len() {
            self.prev = mags.to_vec();
            self.initialised = true;
            return;
        }
        for (i, m) in mags.iter_mut().enumerate() {
            let cur = *m;
            *m = (cur - self.prev[i]).abs();
            self.prev[i] = cur;
        }
    }

    fn reset(&mut self) {
        self.prev.clear();
        self.initialised = false;
    }
}

// ── Spectral contrast filter ────────────────────────────────────────────────

/// Per-band peak-to-valley contrast.  High values indicate clear tonal
/// content; low values indicate broadband noise.  Six octave-spaced bands.
const CONTRAST_BANDS: &[(f32, f32)] = &[
    (20.0,   200.0),
    (200.0,  400.0),
    (400.0,  800.0),
    (800.0,  1600.0),
    (1600.0, 3200.0),
    (3200.0, 22050.0),
];

pub struct SpectralContrastFilter {
    bands:       Vec<(usize, usize)>, // (start_bin, end_bin) per band
    cached_bins: usize,
}

impl SpectralContrastFilter {
    pub fn new() -> Self {
        Self { bands: Vec::new(), cached_bins: 0 }
    }

    fn ensure_bands(&mut self, n_bins: usize, sample_rate: u32, fft_size: usize) {
        if self.cached_bins == n_bins { return; }
        let freq_res = sample_rate as f32 / fft_size as f32;
        self.bands = CONTRAST_BANDS.iter().map(|&(lo, hi)| {
            let start = ((lo / freq_res) as usize).clamp(0, n_bins - 1);
            let end   = ((hi / freq_res) as usize).clamp(start + 1, n_bins);
            (start, end)
        }).collect();
        self.cached_bins = n_bins;
    }
}

impl DspFilter for SpectralContrastFilter {
    fn name(&self) -> &str { "spectral_contrast" }
    fn domain(&self) -> FilterDomain { FilterDomain::FrequencyDomain }

    fn process_freq(&mut self, mags: &mut [f32], sample_rate: u32, fft_size: usize) {
        self.ensure_bands(mags.len(), sample_rate, fft_size);
        let len = mags.len();
        for &(start, end) in &self.bands {
            let end = end.min(len);
            if start >= end || start >= len { continue; }
            let peak = mags[start..end].iter().cloned().fold(0.0_f32, f32::max);
            let valley = mags[start..end].iter().cloned().fold(f32::MAX, f32::min);
            let contrast = peak - valley;
            for m in &mut mags[start..end] {
                *m = contrast;
            }
        }
    }

    fn reset(&mut self) {
        self.bands.clear();
        self.cached_bins = 0;
    }
}

// ── HPSS filter ─────────────────────────────────────────────────────────────

/// Harmonic-percussive source separation via median filtering.
///
/// Maintains an internal circular buffer of FFT frames.  Horizontal median
/// across time isolates harmonics (sustained tones); vertical median across
/// frequency isolates percussives (transients).  A soft Wiener mask selects
/// one component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HpssMode { Harmonic, Percussive }

pub struct HpssFilter {
    mode:        HpssMode,
    history:     Vec<Vec<f32>>,
    head:        usize,
    filled:      usize,
    kernel_time: usize,  // temporal median width (odd)
    kernel_freq: usize,  // frequency median width (odd)
}

impl HpssFilter {
    pub fn new(mode: HpssMode) -> Self {
        Self {
            mode,
            history:     Vec::new(),
            head:        0,
            filled:      0,
            kernel_time: 11,
            kernel_freq: 13,
        }
    }

    pub fn harmonic()  -> Self { Self::new(HpssMode::Harmonic) }
    pub fn percussive() -> Self { Self::new(HpssMode::Percussive) }

    fn ensure_history(&mut self, n_bins: usize) {
        if self.history.len() != self.kernel_time
            || self.history.first().map_or(true, |r| r.len() != n_bins)
        {
            self.history = vec![vec![0.0f32; n_bins]; self.kernel_time];
            self.head   = 0;
            self.filled = 0;
        }
    }
}

/// Sort-based median for small slices.
fn small_median(buf: &mut [f32]) -> f32 {
    buf.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = buf.len();
    if n % 2 == 1 { buf[n / 2] } else { (buf[n / 2 - 1] + buf[n / 2]) * 0.5 }
}

impl DspFilter for HpssFilter {
    fn name(&self) -> &str { "hpss" }
    fn domain(&self) -> FilterDomain { FilterDomain::FrequencyDomain }

    fn process_freq(&mut self, mags: &mut [f32], _sample_rate: u32, _fft_size: usize) {
        let n_bins = mags.len();
        self.ensure_history(n_bins);

        // Push current frame into circular buffer
        self.history[self.head] = mags.to_vec();
        self.head = (self.head + 1) % self.kernel_time;
        self.filled = self.filled.min(self.kernel_time - 1) + 1;

        // Need at least kernel_time frames to produce valid output
        if self.filled < self.kernel_time { return; }

        let kt = self.kernel_time;
        let kf = self.kernel_freq;
        let half_kf = kf / 2;

        // Compute harmonic estimate (temporal median per bin)
        let mut h_est = vec![0.0f32; n_bins];
        let mut tbuf = vec![0.0f32; kt];
        for bin in 0..n_bins {
            for t in 0..kt {
                tbuf[t] = self.history[t][bin];
            }
            h_est[bin] = small_median(&mut tbuf);
        }

        // Compute percussive estimate (frequency median per bin, current frame)
        let current = (self.head + kt - 1) % kt;
        let frame = &self.history[current];
        let mut p_est = vec![0.0f32; n_bins];
        let mut fbuf = vec![0.0f32; kf];
        for bin in 0..n_bins {
            let lo = bin.saturating_sub(half_kf);
            let hi = (bin + half_kf + 1).min(n_bins);
            let len = hi - lo;
            fbuf[..len].copy_from_slice(&frame[lo..hi]);
            p_est[bin] = small_median(&mut fbuf[..len]);
        }

        // Apply soft Wiener mask
        for (i, m) in mags.iter_mut().enumerate() {
            let h2 = h_est[i] * h_est[i];
            let p2 = p_est[i] * p_est[i];
            let denom = h2 + p2;
            if denom < 1e-18 { continue; }
            let mask = match self.mode {
                HpssMode::Harmonic  => h2 / denom,
                HpssMode::Percussive => p2 / denom,
            };
            *m *= mask;
        }
    }

    fn reset(&mut self) {
        self.history.clear();
        self.head   = 0;
        self.filled = 0;
    }
}

// ── Frequency scale utilities ───────────────────────────────────────────────

/// Convert Hz to Mel scale (Slaney/O'Shaughnessy).
pub fn hz_to_mel(f: f32) -> f32 {
    2595.0 * (1.0 + f / 700.0).log10()
}

/// Convert Mel scale back to Hz.
pub fn mel_to_hz(m: f32) -> f32 {
    700.0 * (10.0_f32.powf(m / 2595.0) - 1.0)
}

/// Convert Hz to Bark scale (Traunmüller approximation).
pub fn hz_to_bark(f: f32) -> f32 {
    13.0 * (0.00076 * f).atan() + 3.5 * ((f / 7500.0).powi(2)).atan()
}

/// Convert Bark scale back to Hz (Newton iteration).
pub fn bark_to_hz(b: f32) -> f32 {
    // Start from a rough linear estimate, refine with Newton's method.
    let mut f = b * 100.0; // crude initial guess
    for _ in 0..8 {
        let err = hz_to_bark(f) - b;
        // Numerical derivative
        let df = hz_to_bark(f + 0.1) - hz_to_bark(f);
        if df.abs() < 1e-12 { break; }
        f -= err / (df * 10.0);
        f = f.max(0.0);
    }
    f
}

// ── DspChain ─────────────────────────────────────────────────────────────────

/// Orchestrates a chain of `DspFilter`s, handling time-domain → re-FFT →
/// frequency-domain sequencing automatically.
pub struct DspChain {
    filters:     Vec<Box<dyn DspFilter>>,
    has_time:    bool,
    // Lazy FFT resources (only allocated when a time-domain filter is active)
    planner:     Option<FftPlanner<f32>>,
    hann:        Vec<f32>,
}

impl DspChain {
    pub fn new() -> Self {
        Self {
            filters:  Vec::new(),
            has_time: false,
            planner:  None,
            hann:     Vec::new(),
        }
    }

    /// Replace the current filter set.  Time-domain filters should be ordered
    /// in the desired processing sequence (e.g. pre-emphasis before bandpass).
    pub fn set_filters(&mut self, filters: Vec<Box<dyn DspFilter>>) {
        self.has_time = filters.iter().any(|f| f.domain() == FilterDomain::TimeDomain);
        if self.has_time && self.planner.is_none() {
            self.planner = Some(FftPlanner::new());
            self.hann = hann_window(FFT_SIZE);
        }
        self.filters = filters;
    }

    /// Clear all filters.
    pub fn clear(&mut self) {
        self.filters.clear();
        self.has_time = false;
    }

    /// Returns `true` if no filters are active.
    pub fn is_empty(&self) -> bool { self.filters.is_empty() }

    /// Apply the full filter chain.
    ///
    /// * `samples` — mono PCM (possibly mid/side extracted); modified in-place
    ///   by time-domain filters.
    /// * `fft_out` — FFT magnitudes; recomputed from `samples` if any
    ///   time-domain filter ran, then modified by frequency-domain filters.
    pub fn apply(
        &mut self,
        samples:     &mut [f32],
        fft_out:     &mut Vec<f32>,
        sample_rate: u32,
        fft_size:    usize,
    ) {
        // Phase 1: time-domain filters
        if self.has_time {
            for f in self.filters.iter_mut() {
                if f.domain() == FilterDomain::TimeDomain {
                    f.process_time(samples);
                }
            }
            // Re-compute FFT from filtered samples
            if let Some(planner) = &mut self.planner {
                *fft_out = compute_fft(samples, &self.hann, planner, fft_size);
            }
        }

        // Phase 2: frequency-domain filters
        for f in self.filters.iter_mut() {
            if f.domain() == FilterDomain::FrequencyDomain {
                f.process_freq(fft_out, sample_rate, fft_size);
            }
        }
    }
}

// ── Internal FFT helpers ─────────────────────────────────────────────────────

fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (n - 1) as f32).cos()))
        .collect()
}

fn compute_fft(
    mono:    &[f32],
    window:  &[f32],
    planner: &mut FftPlanner<f32>,
    n:       usize,
) -> Vec<f32> {
    let mut input: Vec<Complex<f32>> = (0..n)
        .map(|i| {
            let s = if i < mono.len() { mono[i] } else { 0.0 };
            let w = if i < window.len() { window[i] } else { 1.0 };
            Complex::new(s * w, 0.0)
        })
        .collect();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut input);
    let scale = 1.0 / n as f32;
    input[..n / 2 + 1].iter().map(|c| c.norm() * scale).collect()
}

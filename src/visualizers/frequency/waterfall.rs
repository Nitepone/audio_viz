/// waterfall.rs — Scrolling spectrogram: frequency on X, time flowing downward.
///
/// Each new frame the spectrum is captured as a row of colour-coded intensity
/// values. Rows scroll downward; the newest row is always at the top.
///
/// Config:
///   speed          — 1–4: how many rows to advance per frame (1=slow, 4=fast)
///   color_scheme   — heat / ice / spectrum / mono / phosphor
///   frequency_scale — linear / log
///   peak_hold      — 0–3 s: time a peak marker stays lit before fading

// ── Index: palettes@30 · WaterfallViz@58 · new@88 · freq_to_col@171 · col_to_bin@196 · rebuild_dsp@229 · impl@273 · config@277 · set_config@387 · tick@463 · render@518 · register@586
use crate::visualizer::{
    merge_config,
    pad_frame, specgrad, status_bar,
    AudioFrame, TermSize, Visualizer, FFT_SIZE,
};
use crate::visualizer_utils::{
    palette_lookup, mag_to_frac as mag_to_frac_generic,
};
use crate::dsp::{
    AWeightFilter, BandPreset, BandpassFilter, DspChain, HpssFilter,
    MidSideFilter, PreEmphasisFilter, SpectralContrastFilter,
    SpectralDiffFilter, SpectralWhiteningFilter,
};

const CONFIG_VERSION: u64 = 2;

// ── Colour palettes ────────────────────────────────────────────────────────────

// Waterfall-specific palettes with leading black (232) for dark background
const HEAT:  &[u8] = &[232, 52, 88, 124, 160, 196, 202, 208, 214, 220, 226, 227, 228, 229, 230, 231];
const W_ICE: &[u8] = &[232, 17, 18, 19, 20, 21, 27, 33, 39, 45, 51, 87, 123, 159, 195, 231];
const PHOS:  &[u8] = &[232, 22, 28, 34, 40, 46, 82, 118, 154, 190, 226, 229, 231];

fn palette_mono(frac: f32) -> u8 {
    let level = (frac.clamp(0.0, 1.0) * 23.0) as u8;
    232 + level
}

fn color_for(frac: f32, scheme: &str) -> u8 {
    match scheme {
        "heat"     => palette_lookup(frac, HEAT),
        "ice"      => palette_lookup(frac, W_ICE),
        "mono"     => palette_mono(frac),
        "phosphor" => palette_lookup(frac, PHOS),
        _          => specgrad(frac),
    }
}

/// Convert linear FFT magnitude to dB-normalised 0..1 frac.
fn mag_to_frac(v: f32) -> f32 {
    mag_to_frac_generic(v, -72.0, -12.0)
}

// ── Struct ────────────────────────────────────────────────────────────────────

pub struct WaterfallViz {
    /// Circular buffer of rows. Each row is `cols` frac values (0..1).
    history:  Vec<Vec<f32>>,
    /// Parallel peak buffer (frac per column).
    peaks:    Vec<f32>,
    peak_age: Vec<f32>,
    head:     usize,
    cached_cols: usize,
    source: String,
    // ── Config ────────────────────────────────────────────────────────────────
    gain:            f32,
    speed:           usize,  // 1–4 rows/frame
    color_scheme:    String,
    frequency_scale: String, // "linear" | "log"
    peak_hold:       f32,    // seconds
    freq_axis:       bool,
    // ── DSP filters ──────────────────────────────────────────────────────────
    dsp_chain:        DspChain,
    mid_side:         Option<MidSideFilter>,
    cfg_stereo_field: String,   // "stereo" | "mid" | "side"
    cfg_band_filter:  String,   // "off" | "bass" | "mids" | "vocal" | "presence" | "air"
    cfg_a_weight:         bool,
    cfg_whitening:        bool,
    cfg_pre_emphasis:     bool,
    cfg_spectral_diff:    bool,
    cfg_spectral_contrast: bool,
    cfg_hpss:             String,  // "off" | "harmonic" | "percussive"
}

impl WaterfallViz {
    pub fn new(source: &str) -> Self {
        Self {
            history:         Vec::new(),
            peaks:           Vec::new(),
            peak_age:        Vec::new(),
            head:            0,
            cached_cols:     0,
            source:          source.to_string(),
            gain:            1.0,
            speed:           1,
            color_scheme:    "heat".to_string(),
            frequency_scale: "log".to_string(),
            peak_hold:       1.0,
            freq_axis:       false,
            dsp_chain:        DspChain::new(),
            mid_side:         None,
            cfg_stereo_field: "stereo".to_string(),
            cfg_band_filter:  "off".to_string(),
            cfg_a_weight:         false,
            cfg_whitening:        false,
            cfg_pre_emphasis:     false,
            cfg_spectral_diff:    false,
            cfg_spectral_contrast: false,
            cfg_hpss:             "off".to_string(),
        }
    }

    fn ensure_buffers(&mut self, rows: usize, cols: usize) {
        if self.history.len() != rows || self.cached_cols != cols {
            self.history  = vec![vec![0.0f32; cols]; rows];
            self.peaks    = vec![0.0f32; cols];
            self.peak_age = vec![999.0f32; cols];
            self.head     = 0;
            self.cached_cols = cols;
        }
    }

    /// Build a labelled frequency axis row for display at the top.
    fn build_freq_axis(&self, cols: usize) -> String {
        use crate::visualizer::SAMPLE_RATE;
        // Key frequencies to label
        const LABELS: &[(f32, &str)] = &[
            (50.0,    "50"),
            (100.0,   "100"),
            (250.0,   "250"),
            (500.0,   "500"),
            (1_000.0, "1k"),
            (2_000.0, "2k"),
            (4_000.0, "4k"),
            (8_000.0, "8k"),
            (16_000.0,"16k"),
        ];

        let n_bins = FFT_SIZE / 2 + 1;
        let nyquist = SAMPLE_RATE as f32 / 2.0;
        let freq_res = nyquist / n_bins as f32;
        let scale = self.frequency_scale.as_str();

        // Build plain character buffer first
        let mut buf = vec![b' '; cols];

        // Draw tick marks at key freq positions
        for &(freq, label) in LABELS {
            if freq > nyquist { break; }
            // Column for this frequency (inverse of col_to_bin)
            let col = Self::freq_to_col(freq, cols, n_bins, freq_res, nyquist, scale);
            if col >= cols { continue; }
            // Write the label left-aligned from the tick column
            let bytes = label.as_bytes();
            for (i, &b) in bytes.iter().enumerate() {
                if col + i < cols { buf[col + i] = b; }
            }
        }

        // Wrap in dim ANSI colour
        let mut line = String::with_capacity(cols * 8);
        line.push_str("\x1b[2m\x1b[38;5;240m");
        line.push_str(std::str::from_utf8(&buf).unwrap_or(""));
        line.push_str("\x1b[0m");
        line
    }

    /// Map a frequency (Hz) to a column index (inverse of col_to_bin).
    fn freq_to_col(freq: f32, cols: usize, n_bins: usize, freq_res: f32, nyquist: f32, scale: &str) -> usize {
        use crate::dsp::{hz_to_mel, hz_to_bark};
        let t = match scale {
            "log" => {
                let lo = 1.0f32.ln();
                let hi = (n_bins as f32).ln();
                let bin = (freq / freq_res).max(1.0);
                (bin.ln() - lo) / (hi - lo)
            }
            "mel" => {
                let mel_lo = hz_to_mel(freq_res);
                let mel_hi = hz_to_mel(nyquist);
                (hz_to_mel(freq) - mel_lo) / (mel_hi - mel_lo)
            }
            "bark" => {
                let bark_lo = hz_to_bark(freq_res);
                let bark_hi = hz_to_bark(nyquist);
                (hz_to_bark(freq) - bark_lo) / (bark_hi - bark_lo)
            }
            _ => freq / nyquist, // linear
        };
        (t.clamp(0.0, 1.0) * (cols - 1) as f32) as usize
    }

    /// Map a column index (0..cols) to an FFT bin index, honouring freq scale.
    fn col_to_bin(c: usize, cols: usize, n_bins: usize, scale: &str) -> usize {
        use crate::dsp::{hz_to_mel, mel_to_hz, hz_to_bark, bark_to_hz};
        use crate::visualizer::SAMPLE_RATE;
        let t = c as f32 / cols.max(1) as f32;
        let nyquist = SAMPLE_RATE as f32 / 2.0;
        let freq_res = nyquist / n_bins.max(1) as f32;
        match scale {
            "log" => {
                let lo = 1.0f32.ln();
                let hi = (n_bins as f32).ln();
                ((lo + t * (hi - lo)).exp() as usize).clamp(1, n_bins - 1)
            }
            "mel" => {
                let mel_lo = hz_to_mel(freq_res);        // ~1 bin
                let mel_hi = hz_to_mel(nyquist);
                let mel = mel_lo + t * (mel_hi - mel_lo);
                let hz = mel_to_hz(mel);
                ((hz / freq_res) as usize).clamp(1, n_bins - 1)
            }
            "bark" => {
                let bark_lo = hz_to_bark(freq_res);
                let bark_hi = hz_to_bark(nyquist);
                let bark = bark_lo + t * (bark_hi - bark_lo);
                let hz = bark_to_hz(bark);
                ((hz / freq_res) as usize).clamp(1, n_bins - 1)
            }
            _ => { // "linear"
                (c * n_bins / cols.max(1)).clamp(0, n_bins - 1)
            }
        }
    }

    /// Rebuild the filter chain from current `cfg_*` fields.
    fn rebuild_dsp_chain(&mut self) {
        use crate::visualizer::SAMPLE_RATE;
        let mut filters: Vec<Box<dyn crate::dsp::DspFilter>> = Vec::new();

        // Time-domain filters (order matters: pre-emphasis before bandpass)
        if self.cfg_pre_emphasis {
            filters.push(Box::new(PreEmphasisFilter::standard()));
        }
        if let Some(preset) = BandPreset::from_str(&self.cfg_band_filter) {
            filters.push(Box::new(BandpassFilter::new(preset, SAMPLE_RATE as f32)));
        }

        // Frequency-domain filters (order: HPSS → contrast → diff → a-weight → whitening)
        match self.cfg_hpss.as_str() {
            "harmonic"  => filters.push(Box::new(HpssFilter::harmonic())),
            "percussive" => filters.push(Box::new(HpssFilter::percussive())),
            _ => {}
        }
        if self.cfg_spectral_contrast {
            filters.push(Box::new(SpectralContrastFilter::new()));
        }
        if self.cfg_spectral_diff {
            filters.push(Box::new(SpectralDiffFilter::new()));
        }
        if self.cfg_a_weight {
            filters.push(Box::new(AWeightFilter::new()));
        }
        if self.cfg_whitening {
            filters.push(Box::new(SpectralWhiteningFilter::standard()));
        }

        self.dsp_chain.set_filters(filters);

        // Mid/side (handled separately, needs stereo input)
        self.mid_side = match self.cfg_stereo_field.as_str() {
            "mid"  => Some(MidSideFilter::mid()),
            "side" => Some(MidSideFilter::side()),
            _      => None,
        };
    }
}

// ── Visualizer impl ───────────────────────────────────────────────────────────

impl Visualizer for WaterfallViz {
    fn name(&self)        -> &str { "waterfall" }
    fn description(&self) -> &str { "Scrolling spectrogram — frequency vs time" }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "waterfall",
            "version": CONFIG_VERSION,
            "config": [
                {
                    "name": "gain",
                    "display_name": "Gain",
                    "type": "float",
                    "value": 1.0,
                    "min": 0.0,
                    "max": 4.0
                },
                {
                    "name": "speed",
                    "display_name": "Speed",
                    "type": "int",
                    "value": 1,
                    "min": 1,
                    "max": 4
                },
                {
                    "name": "color_scheme",
                    "display_name": "Color Scheme",
                    "type": "enum",
                    "value": "heat",
                    "variants": ["heat", "ice", "spectrum", "mono", "phosphor"]
                },
                {
                    "name": "frequency_scale",
                    "display_name": "Frequency Scale",
                    "type": "enum",
                    "value": "log",
                    "variants": ["linear", "log", "mel", "bark"]
                },
                {
                    "name": "peak_hold",
                    "display_name": "Peak Hold (s)",
                    "type": "float",
                    "value": 1.0,
                    "min": 0.0,
                    "max": 3.0
                },
                {
                    "name": "freq_axis",
                    "display_name": "Freq Axis",
                    "type": "enum",
                    "value": "off",
                    "variants": ["off", "on"]
                },
                {
                    "name": "stereo_field",
                    "display_name": "Stereo Field",
                    "type": "enum",
                    "value": "stereo",
                    "variants": ["stereo", "mid", "side"]
                },
                {
                    "name": "band_filter",
                    "display_name": "Band Filter",
                    "type": "enum",
                    "value": "off",
                    "variants": ["off", "bass", "mids", "vocal", "presence", "air"]
                },
                {
                    "name": "a_weight",
                    "display_name": "A-Weight",
                    "type": "enum",
                    "value": "off",
                    "variants": ["off", "on"]
                },
                {
                    "name": "whitening",
                    "display_name": "Spectral Whitening",
                    "type": "enum",
                    "value": "off",
                    "variants": ["off", "on"]
                },
                {
                    "name": "pre_emphasis",
                    "display_name": "Pre-Emphasis",
                    "type": "enum",
                    "value": "off",
                    "variants": ["off", "on"]
                },
                {
                    "name": "spectral_diff",
                    "display_name": "Spectral Diff",
                    "type": "enum",
                    "value": "off",
                    "variants": ["off", "on"]
                },
                {
                    "name": "spectral_contrast",
                    "display_name": "Spectral Contrast",
                    "type": "enum",
                    "value": "off",
                    "variants": ["off", "on"]
                },
                {
                    "name": "hpss",
                    "display_name": "HPSS",
                    "type": "enum",
                    "value": "off",
                    "variants": ["off", "harmonic", "percussive"]
                }
            ]
        }).to_string()
    }

    fn set_config(&mut self, json: &str) -> Result<String, String> {
        let merged = merge_config(&self.get_default_config(), json);
        let val: serde_json::Value = serde_json::from_str(&merged)
            .map_err(|e| format!("JSON parse error: {e}"))?;
        if let Some(config) = val["config"].as_array() {
            for entry in config {
                match entry["name"].as_str().unwrap_or("") {
                    "speed" => {
                        let v = entry["value"].as_i64()
                            .or_else(|| entry["value"].as_f64().map(|f| f as i64))
                            .unwrap_or(1);
                        self.speed = (v as usize).clamp(1, 4);
                    }
                    "color_scheme" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.color_scheme = s.to_string();
                        }
                    }
                    "frequency_scale" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.frequency_scale = s.to_string();
                        }
                    }
                    "gain" => {
                        self.gain = entry["value"].as_f64().unwrap_or(1.0) as f32;
                    }
                    "peak_hold" => {
                        self.peak_hold = entry["value"].as_f64().unwrap_or(1.0) as f32;
                    }
                    "freq_axis" => {
                        self.freq_axis = entry["value"].as_str() == Some("on");
                    }
                    "stereo_field" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.cfg_stereo_field = s.to_string();
                        }
                    }
                    "band_filter" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.cfg_band_filter = s.to_string();
                        }
                    }
                    "a_weight" => {
                        self.cfg_a_weight = entry["value"].as_str() == Some("on");
                    }
                    "whitening" => {
                        self.cfg_whitening = entry["value"].as_str() == Some("on");
                    }
                    "pre_emphasis" => {
                        self.cfg_pre_emphasis = entry["value"].as_str() == Some("on");
                    }
                    "spectral_diff" => {
                        self.cfg_spectral_diff = entry["value"].as_str() == Some("on");
                    }
                    "spectral_contrast" => {
                        self.cfg_spectral_contrast = entry["value"].as_str() == Some("on");
                    }
                    "hpss" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.cfg_hpss = s.to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
        self.rebuild_dsp_chain();
        Ok(merged)
    }

    fn on_resize(&mut self, size: TermSize) {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        self.ensure_buffers(rows.saturating_sub(1).max(1), cols);
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: TermSize) {
        let rows = (size.rows as usize).saturating_sub(1).max(1);
        let cols = size.cols as usize;
        self.ensure_buffers(rows, cols);

        // ── DSP filter pipeline ───────────────────────────────────────────
        let fft: Vec<f32>;
        if !self.dsp_chain.is_empty() || self.mid_side.is_some() {
            let mut work_samples = match &self.mid_side {
                Some(ms) => ms.extract(&audio.left, &audio.right),
                None     => audio.mono.clone(),
            };
            let mut work_fft = audio.fft.clone();
            self.dsp_chain.apply(
                &mut work_samples, &mut work_fft,
                audio.sample_rate, FFT_SIZE,
            );
            fft = work_fft;
        } else {
            fft = audio.fft.clone();
        }
        let n_bins  = fft.len();
        let scale   = self.frequency_scale.as_str();

        // Build one row of frac values
        let new_row: Vec<f32> = (0..cols)
            .map(|c| {
                let bin = Self::col_to_bin(c, cols, n_bins, scale);
                (mag_to_frac(fft[bin]) * self.gain).min(1.0)
            })
            .collect();

        // Advance peak markers
        for c in 0..cols {
            if self.peak_age[c] < self.peak_hold {
                self.peak_age[c] += dt;
            } else {
                // fade after hold
                self.peaks[c] = (self.peaks[c] - dt * 0.3).max(0.0);
            }
            if new_row[c] >= self.peaks[c] {
                self.peaks[c]    = new_row[c];
                self.peak_age[c] = 0.0;
            }
        }

        // Write `speed` copies of the new row into the circular buffer
        for _ in 0..self.speed {
            if rows > 0 {
                self.history[self.head] = new_row.clone();
                self.head = (self.head + 1) % rows;
            }
        }
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = rows.saturating_sub(1).max(1);

        let mut lines = Vec::with_capacity(rows);

        // ── Optional frequency axis (row 0) ──────────────────────────────────
        let data_start = if self.freq_axis {
            let axis_row = self.build_freq_axis(cols);
            lines.push(axis_row);
            1
        } else {
            0
        };

        let n_hist = self.history.len();

        for r in data_start..vis {
            let data_r = r - data_start; // row index into history
            // Row 0 = newest data; head-1 = most recently written row.
            let hist_idx = if n_hist > 0 {
                (self.head + n_hist - 1 - data_r % n_hist) % n_hist
            } else {
                0
            };

            let mut line = String::with_capacity(cols * 12);

            let row_data = if hist_idx < self.history.len() {
                &self.history[hist_idx]
            } else {
                &[] as &[f32]
            };

            for c in 0..cols {
                let frac = if c < row_data.len() { row_data[c] } else { 0.0 };

                // Peak marker on the newest data row only
                let is_peak = data_r == 0
                    && c < self.peaks.len()
                    && self.peaks[c] > 0.02
                    && (self.peaks[c] - frac).abs() < 0.12;

                if is_peak {
                    let code = color_for(self.peaks[c], &self.color_scheme);
                    line.push_str(&format!("\x1b[1m\x1b[38;5;{code}m▲\x1b[0m"));
                } else if frac < 0.04 {
                    line.push(' ');
                } else {
                    let code = color_for(frac, &self.color_scheme);
                    let ch = if frac < 0.25 { '░' }
                             else if frac < 0.50 { '▒' }
                             else if frac < 0.75 { '▓' }
                             else { '█' };
                    line.push_str(&format!("\x1b[38;5;{code}m{ch}\x1b[0m"));
                }
            }
            lines.push(line);
        }

        lines.push(status_bar(cols, fps, self.name(), &self.source, ""));
        pad_frame(lines, rows, cols)
    }
}

// ── Registration ──────────────────────────────────────────────────────────────

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(WaterfallViz::new(""))]
}

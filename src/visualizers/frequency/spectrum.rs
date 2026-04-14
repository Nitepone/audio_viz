/// spectrum.rs — Vertical frequency bar visualizer with DSP filter support.

// ── Index: SpectrumViz@79 · new@103 · col_to_bin@127 · rebuild_dsp@150 · render_band_frame@190 · impl@304 · config@308 · set_config@395 · tick@452 · render@508 · register@638
use crate::visualizer::{
    merge_config,
    pad_frame, status_bar, hline, title_line,
    AudioFrame, SpectrumBars, TermSize, Visualizer, FFT_SIZE,
    RISE_COEFF, FALL_COEFF, PEAK_HOLD_SECS, PEAK_DROP_RATE, SAMPLE_RATE,
};
use crate::visualizer_utils::with_gained_fft;
use crate::dsp::{
    hz_to_bark, hz_to_mel, bark_to_hz, mel_to_hz,
    AWeightFilter, BandPreset, BandpassFilter, DspChain, HpssFilter,
    MidSideFilter, PreEmphasisFilter, SpectralContrastFilter,
    SpectralDiffFilter, SpectralWhiteningFilter,
};

const CONFIG_VERSION: u64 = 3;

// ── HiFi / LED shared band definitions ───────────────────────────────────────

const HIFI_BANDS: &[(f32, &str)] = &[
    (25.0,    "25"),
    (40.0,    "40"),
    (63.0,    "63"),
    (100.0,  "100"),
    (160.0,  "160"),
    (250.0,  "250"),
    (500.0,  "500"),
    (1000.0,  "1k"),
    (2000.0,  "2k"),
    (4000.0,  "4k"),
    (8000.0,  "8k"),
    (16000.0,"16k"),
];

// ── Theme ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
enum Theme {
    HiFi,     // vintage VFD teal — 12 fixed bands, half-block segments
    Led,      // red LED bar graph — 12 fixed bands, sparse bar / solid peak
    Phosphor, // mysterious green phosphor CRT
    Mono,     // utilitarian monochrome
}

impl Theme {
    fn from_str(s: &str) -> Self {
        match s {
            "hifi" => Theme::HiFi,
            "led"  => Theme::Led,
            "mono" => Theme::Mono,
            _      => Theme::Phosphor,
        }
    }

    fn is_band_layout(self) -> bool {
        matches!(self, Theme::HiFi | Theme::Led)
    }
}

// ── Shared 12-band layout config ──────────────────────────────────────────────

/// Everything that differs between the HiFi and LED themes.
struct BandLayout {
    title:       &'static str,
    title_color: u8,
    rule_color:  u8,
    label_color: u8,
    /// Colour for a lit bar segment at the given normalised height [0,1].
    bar_color:   fn(f32) -> u8,
    bar_char:    &'static str,
    peak_char:   &'static str,
    peak_color:  u8,
}

// ── Visualizer struct ─────────────────────────────────────────────────────────

pub struct SpectrumViz {
    bars:         SpectrumBars,
    source:       String,
    gain:         f32,
    theme:        Theme,
    frequency_scale: String,
    /// Smoothed bar heights for non-log scales (one per column).
    scale_bars:       Vec<f32>,
    scale_peaks:      Vec<f32>,
    scale_peak_timers: Vec<f32>,
    // ── DSP filters ──────────────────────────────────────────────────────────
    dsp_chain:             DspChain,
    mid_side:              Option<MidSideFilter>,
    cfg_stereo_field:      String,
    cfg_band_filter:       String,
    cfg_a_weight:          bool,
    cfg_whitening:         bool,
    cfg_pre_emphasis:      bool,
    cfg_spectral_diff:     bool,
    cfg_spectral_contrast: bool,
    cfg_hpss:              String,
}

impl SpectrumViz {
    pub fn new(source: &str) -> Self {
        Self {
            bars:            SpectrumBars::new(80),
            source:          source.to_string(),
            gain:            1.0,
            theme:           Theme::Phosphor,
            frequency_scale: "log".to_string(),
            scale_bars:       Vec::new(),
            scale_peaks:      Vec::new(),
            scale_peak_timers: Vec::new(),
            dsp_chain:             DspChain::new(),
            mid_side:              None,
            cfg_stereo_field:      "stereo".to_string(),
            cfg_band_filter:       "off".to_string(),
            cfg_a_weight:          false,
            cfg_whitening:         false,
            cfg_pre_emphasis:      false,
            cfg_spectral_diff:     false,
            cfg_spectral_contrast: false,
            cfg_hpss:              "off".to_string(),
        }
    }

    /// Map column index to FFT bin for non-log scales.
    fn col_to_bin(c: usize, cols: usize, n_bins: usize, scale: &str) -> usize {
        let t        = c as f32 / cols.max(1) as f32;
        let nyquist  = SAMPLE_RATE as f32 / 2.0;
        let freq_res = nyquist / n_bins.max(1) as f32;
        match scale {
            "mel" => {
                let lo = hz_to_mel(freq_res);
                let hi = hz_to_mel(nyquist);
                let hz = mel_to_hz(lo + t * (hi - lo));
                ((hz / freq_res) as usize).clamp(1, n_bins - 1)
            }
            "bark" => {
                let lo = hz_to_bark(freq_res);
                let hi = hz_to_bark(nyquist);
                let hz = bark_to_hz(lo + t * (hi - lo));
                ((hz / freq_res) as usize).clamp(1, n_bins - 1)
            }
            _ => { // "linear"
                (c * n_bins / cols.max(1)).clamp(0, n_bins - 1)
            }
        }
    }

    fn rebuild_dsp_chain(&mut self) {
        use crate::visualizer::SAMPLE_RATE;
        let mut filters: Vec<Box<dyn crate::dsp::DspFilter>> = Vec::new();

        if self.cfg_pre_emphasis {
            filters.push(Box::new(PreEmphasisFilter::standard()));
        }
        if let Some(preset) = BandPreset::from_str(&self.cfg_band_filter) {
            filters.push(Box::new(BandpassFilter::new(preset, SAMPLE_RATE as f32)));
        }

        match self.cfg_hpss.as_str() {
            "harmonic"   => filters.push(Box::new(HpssFilter::harmonic())),
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

        self.mid_side = match self.cfg_stereo_field.as_str() {
            "mid"  => Some(MidSideFilter::mid()),
            "side" => Some(MidSideFilter::side()),
            _      => None,
        };
    }

    // ── Band-layout rendering (HiFi + LED) ───────────────────────────────────

    fn render_band_frame(&self, layout: &BandLayout, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = (rows.saturating_sub(4)).max(4);

        let n   = HIFI_BANDS.len(); // 12
        let gap = 1usize;

        let bar_w    = ((cols.saturating_sub((n - 1) * gap)) / n).clamp(3, 9);
        let total_w  = n * bar_w + (n - 1) * gap;
        let left_pad = cols.saturating_sub(total_w) / 2;

        // Sample smoothed/peak at each band's log-spaced position.
        let n_bars = self.bars.smoothed.len().max(1);
        let log_lo = 30f32.log10();
        let log_hi = 18_000f32.log10();

        let band_vals: Vec<(f32, f32)> = HIFI_BANDS.iter().map(|(freq, _)| {
            let frac = (freq.log10() - log_lo) / (log_hi - log_lo);
            let idx  = ((frac * (n_bars - 1) as f32) as usize).min(n_bars - 1);
            (self.bars.smoothed[idx], self.bars.peaks[idx])
        }).collect();

        let mut lines = Vec::with_capacity(rows);
        lines.push(title_line(cols, layout.title, layout.title_color));
        lines.push(hline(cols, layout.rule_color));

        for row in (0..vis).rev() {
            let threshold = row as f32 / vis as f32;
            let mut line  = String::with_capacity(cols * 14);

            for _ in 0..left_pad { line.push(' '); }

            for (bi, &(bh, ph)) in band_vals.iter().enumerate() {
                if bi > 0 { line.push(' '); }

                let pkr  = (ph * vis as f32) as usize;
                let cell = if bh >= threshold {
                    let color = (layout.bar_color)(threshold);
                    format!("\x1b[38;5;{color}m{}\x1b[0m", layout.bar_char)
                } else if pkr > 0 && row == pkr - 1 && ph > 0.03 {
                    format!("\x1b[1m\x1b[38;5;{}m{}\x1b[0m", layout.peak_color, layout.peak_char)
                } else {
                    String::from(" ")
                };

                for _ in 0..bar_w { line.push_str(&cell); }
            }
            lines.push(line);
        }

        lines.push(hline(cols, layout.rule_color));

        // Frequency labels centred under each bar.
        let mut label_line = String::with_capacity(cols * 10);
        for _ in 0..left_pad { label_line.push(' '); }
        for (bi, &(_, lbl)) in HIFI_BANDS.iter().enumerate() {
            if bi > 0 { label_line.push(' '); }
            let lbl_len = lbl.len();
            let pad_l   = (bar_w.saturating_sub(lbl_len)) / 2;
            let pad_r   = bar_w.saturating_sub(lbl_len + pad_l);
            for _ in 0..pad_l { label_line.push(' '); }
            label_line.push_str(&format!("\x1b[38;5;{}m{lbl}\x1b[0m", layout.label_color));
            for _ in 0..pad_r { label_line.push(' '); }
        }
        lines.push(label_line);
        lines.push(status_bar(cols, fps, self.name(), &self.source, ""));

        pad_frame(lines, rows, cols)
    }

    // ── Band colour functions ─────────────────────────────────────────────────

    /// VFD teal: uniform colour throughout the bar.
    fn hifi_bar_color(_threshold: f32) -> u8 { 30 }

    /// Red LED: uniform pure red throughout.
    fn led_bar_color(_threshold: f32) -> u8 { 160 }

    // ── Per-cell renderers for the full-width themes ──────────────────────────

    fn render_phosphor(row: usize, vis: usize, bh: f32, ph: f32, threshold: f32) -> Option<String> {
        let pkr   = (ph * vis as f32) as usize;
        let color: u8 = if threshold >= 0.75 { 82 } else if threshold >= 0.35 { 40 } else { 22 };

        if bh >= threshold {
            let pfx = if threshold >= 0.75 { "\x1b[1m" } else { "" };
            Some(format!("{pfx}\x1b[38;5;{color}m|\x1b[0m"))
        } else if pkr > 0 && row == pkr - 1 && ph > 0.03 {
            Some(format!("\x1b[1m\x1b[38;5;82m•\x1b[0m"))
        } else if threshold < 0.04 {
            Some(format!("\x1b[38;5;22m·\x1b[0m"))
        } else {
            None
        }
    }

    fn render_mono(row: usize, vis: usize, bh: f32, ph: f32, threshold: f32) -> Option<String> {
        let pkr   = (ph * vis as f32) as usize;
        let color: u8 = if threshold >= 0.80 { 255 } else if threshold >= 0.50 { 245 } else { 238 };

        if bh >= threshold {
            let ch = if threshold >= 0.80 { "▓" } else if threshold >= 0.50 { "▒" } else { "░" };
            Some(format!("\x1b[38;5;{color}m{ch}\x1b[0m"))
        } else if pkr > 0 && row == pkr - 1 && ph > 0.03 {
            Some(format!("\x1b[38;5;250m-\x1b[0m"))
        } else {
            None
        }
    }
}

// ── Visualizer impl ───────────────────────────────────────────────────────────

impl Visualizer for SpectrumViz {
    fn name(&self)        -> &str { "spectrum" }
    fn description(&self) -> &str { "Classic log-spaced frequency bars" }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "spectrum",
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
                    "name": "theme",
                    "display_name": "Theme",
                    "type": "enum",
                    "value": "phosphor",
                    "variants": ["hifi", "led", "phosphor", "mono"]
                },
                {
                    "name": "frequency_scale",
                    "display_name": "Frequency Scale",
                    "type": "enum",
                    "value": "log",
                    "variants": ["linear", "log", "mel", "bark"]
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
                    "gain"  => self.gain  = entry["value"].as_f64().unwrap_or(1.0) as f32,
                    "theme" => self.theme = Theme::from_str(entry["value"].as_str().unwrap_or("phosphor")),
                    "frequency_scale" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.frequency_scale = s.to_string();
                        }
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
        self.bars.resize(size.cols as usize);
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: TermSize) {
        self.bars.resize(size.cols as usize);

        // Resolve working FFT (apply DSP chain if active)
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

        let use_scale = !self.theme.is_band_layout() && self.frequency_scale != "log";

        if use_scale {
            // Non-log full-width path: map each column directly via scale function
            let cols   = size.cols as usize;
            let n_bins = fft.len();
            let scale  = self.frequency_scale.as_str();

            if self.scale_bars.len() != cols {
                self.scale_bars        = vec![0.0; cols];
                self.scale_peaks       = vec![0.0; cols];
                self.scale_peak_timers = vec![0.0; cols];
            }

            for c in 0..cols {
                let bin = Self::col_to_bin(c, cols, n_bins, scale);
                use crate::visualizer_utils::mag_to_frac;
                let raw = (mag_to_frac(fft[bin] * self.gain, -72.0, -12.0)).clamp(0.0, 1.0);
                let a = if raw > self.scale_bars[c] { RISE_COEFF } else { FALL_COEFF };
                self.scale_bars[c] = a * self.scale_bars[c] + (1.0 - a) * raw;

                if self.scale_bars[c] > self.scale_peaks[c] {
                    self.scale_peaks[c]       = self.scale_bars[c];
                    self.scale_peak_timers[c] = 0.0;
                } else {
                    self.scale_peak_timers[c] += dt;
                    if self.scale_peak_timers[c] > PEAK_HOLD_SECS {
                        self.scale_peaks[c] = (self.scale_peaks[c] - PEAK_DROP_RATE).max(0.0);
                    }
                }
            }
        } else {
            with_gained_fft(&fft, self.gain, |f| self.bars.update(f, dt));
        }
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        match self.theme {
            Theme::HiFi => return self.render_band_frame(&BandLayout {
                title:       " SPECTRUM ANALYZER ",
                title_color: 44,
                rule_color:  23,
                label_color: 37,
                bar_color:   Self::hifi_bar_color,
                bar_char:    "▄",
                peak_char:   "▀",
                peak_color:  255,
            }, size, fps),

            Theme::Led => return self.render_band_frame(&BandLayout {
                title:       " SPECTRUM ANALYZER ",
                title_color: 196,
                rule_color:  88,
                label_color: 160,
                bar_color:   Self::led_bar_color,
                bar_char:    "░",
                peak_char:   "▄",
                peak_color:  196,
            }, size, fps),

            _ => {}
        }

        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = (rows.saturating_sub(4)).max(4);

        let use_scale = self.frequency_scale != "log"
            && self.scale_bars.len() == cols;

        let (title_color, rule_color): (u8, u8) = match self.theme {
            Theme::Phosphor => (40,  22),
            Theme::Mono     => (250, 236),
            _               => unreachable!(),
        };

        let title_label = match self.theme {
            Theme::Phosphor => " ◈ SPECTRUM ◈ ",
            Theme::Mono     => " SPECTRUM ",
            _               => unreachable!(),
        };

        let mut lines = Vec::with_capacity(rows);
        lines.push(title_line(cols, title_label, title_color));
        lines.push(hline(cols, rule_color));

        for row in (0..vis).rev() {
            let threshold = row as f32 / vis as f32;
            let mut line  = String::with_capacity(cols * 12);

            for bi in 0..cols {
                let (bh, ph) = if use_scale {
                    (
                        self.scale_bars [bi.min(self.scale_bars.len()  - 1)],
                        self.scale_peaks[bi.min(self.scale_peaks.len() - 1)],
                    )
                } else {
                    (
                        self.bars.smoothed[bi.min(self.bars.smoothed.len() - 1)],
                        self.bars.peaks   [bi.min(self.bars.peaks.len()    - 1)],
                    )
                };

                let cell = match self.theme {
                    Theme::Phosphor => Self::render_phosphor(row, vis, bh, ph, threshold),
                    Theme::Mono     => Self::render_mono(row, vis, bh, ph, threshold),
                    _               => unreachable!(),
                };
                line.push_str(cell.as_deref().unwrap_or(" "));
            }
            lines.push(line);
        }

        lines.push(hline(cols, rule_color));

        // Frequency axis labels — position depends on scale
        let label_color: u8 = match self.theme {
            Theme::Phosphor => 40,
            Theme::Mono     => 244,
            _               => unreachable!(),
        };
        const FREQ_LABELS: &[(f32, &str)] = &[
            (30.0, "30"), (60.0, "60"), (125.0, "125"), (250.0, "250"),
            (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (4000.0, "4k"),
            (8000.0, "8k"), (16000.0, "16k"),
        ];
        let nyquist  = SAMPLE_RATE as f32 / 2.0;
        let n_bins   = FFT_SIZE / 2 + 1;
        let freq_res = nyquist / n_bins as f32;
        let mut label_row: Vec<u8> = vec![b' '; cols];
        for &(freq, lbl) in FREQ_LABELS {
            let col = match self.frequency_scale.as_str() {
                "mel" => {
                    let lo = hz_to_mel(freq_res);
                    let hi = hz_to_mel(nyquist);
                    let t  = (hz_to_mel(freq) - lo) / (hi - lo);
                    (t.clamp(0.0, 1.0) * (cols - 1) as f32) as usize
                }
                "bark" => {
                    let lo = hz_to_bark(freq_res);
                    let hi = hz_to_bark(nyquist);
                    let t  = (hz_to_bark(freq) - lo) / (hi - lo);
                    (t.clamp(0.0, 1.0) * (cols - 1) as f32) as usize
                }
                "linear" => {
                    ((freq / nyquist).clamp(0.0, 1.0) * (cols - 1) as f32) as usize
                }
                _ => { // log
                    let lo = 30f32.log10();
                    let hi = 18_000f32.log10();
                    let t  = (freq.log10() - lo) / (hi - lo);
                    (t.clamp(0.0, 1.0) * (cols - 1) as f32) as usize
                }
            };
            for (i, ch) in lbl.bytes().enumerate() {
                if col + i < cols { label_row[col + i] = ch; }
            }
        }
        let label_str = String::from_utf8(label_row).unwrap_or_default();
        lines.push(format!("\x1b[38;5;{label_color}m{label_str}\x1b[0m"));
        lines.push(status_bar(cols, fps, self.name(), &self.source, ""));

        pad_frame(lines, rows, cols)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(SpectrumViz::new(""))]
}

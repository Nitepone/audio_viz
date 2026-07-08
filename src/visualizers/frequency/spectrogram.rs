/// spectrogram.rs — Scrolling spectrogram (software-rendered).
///
/// Frequency on X, time flowing downward: each frame the current spectrum is
/// painted as `speed` pixel rows at the top of the framebuffer while older
/// rows scroll down.  A direct port of the terminal "waterfall" visualizer to
/// per-pixel RGB rendering.
///
/// Software-render reference implementation: owns a persistent `Framebuffer`
/// and scrolls it in place, so per-frame CPU cost is one memmove plus the
/// newly painted rows.
///
/// Config:
///   gain            — spectrum amplitude multiplier
///   speed           — pixel rows advanced per frame (1–8)
///   color_scheme    — heat / ice / spectrum / mono / phosphor
///   frequency_scale — linear / log

use crate::config::merge_config;
use crate::dsp::mag_to_frac;
use crate::palette::{palette_by_name, palette_lookup};
use crate::visualizer::{AudioFrame, Framebuffer, PixelSize, RenderMode, Visualizer};

const CONFIG_VERSION: u64 = 1;

pub struct SpectrogramViz {
    fb: Framebuffer,
    /// Latest spectrum as palette fractions, one per framebuffer column.
    row: Vec<f32>,
    // ── Config ────────────────────────────────────────────────────────────────
    gain: f32,
    speed: u32,
    color_scheme: String,
    frequency_scale: String,
}

impl SpectrogramViz {
    pub fn new() -> Self {
        Self {
            fb: Framebuffer::new(1, 1),
            row: Vec::new(),
            gain: 1.0,
            speed: 2,
            color_scheme: "heat".to_string(),
            frequency_scale: "log".to_string(),
        }
    }

    /// Map a column index (0..cols) to an FFT bin index, honouring freq scale.
    fn col_to_bin(c: usize, cols: usize, n_bins: usize, log: bool) -> usize {
        if log {
            let lo = 1.0f32.ln();
            let hi = (n_bins as f32).ln();
            let t = c as f32 / cols.max(1) as f32;
            ((lo + t * (hi - lo)).exp() as usize).clamp(1, n_bins - 1)
        } else {
            (c * n_bins / cols.max(1)).clamp(0, n_bins - 1)
        }
    }
}

impl Visualizer for SpectrogramViz {
    fn name(&self) -> &str {
        "spectrogram"
    }
    fn description(&self) -> &str {
        "Scrolling spectrogram — frequency vs time (software-rendered)"
    }
    fn mode(&self) -> RenderMode {
        RenderMode::Software
    }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "spectrogram",
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
                    "display_name": "Speed (px/frame)",
                    "type": "int",
                    "value": 2,
                    "min": 1,
                    "max": 8
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
                    "variants": ["linear", "log"]
                }
            ]
        })
        .to_string()
    }

    fn set_config(&mut self, json: &str) -> Result<String, String> {
        let merged = merge_config(&self.get_default_config(), json);
        let val: serde_json::Value =
            serde_json::from_str(&merged).map_err(|e| format!("JSON parse error: {e}"))?;
        if let Some(config) = val["config"].as_array() {
            for entry in config {
                match entry["name"].as_str().unwrap_or("") {
                    "gain" => self.gain = entry["value"].as_f64().unwrap_or(1.0) as f32,
                    "speed" => {
                        let v = entry["value"]
                            .as_i64()
                            .or_else(|| entry["value"].as_f64().map(|f| f as i64))
                            .unwrap_or(2);
                        self.speed = (v as u32).clamp(1, 8);
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
                    _ => {}
                }
            }
        }
        Ok(merged)
    }

    fn tick(&mut self, audio: &AudioFrame, _dt: f32, size: PixelSize) {
        let cols = size.width as usize;
        if cols == 0 {
            return;
        }
        if self.row.len() != cols {
            self.row = vec![0.0; cols];
        }

        let fft = &audio.fft;
        let n_bins = fft.len();
        let log = self.frequency_scale == "log";

        for c in 0..cols {
            let bin = Self::col_to_bin(c, cols, n_bins, log);
            self.row[c] = (mag_to_frac(fft[bin], -72.0, -12.0) * self.gain).min(1.0);
        }
    }

    fn render_software(&mut self, size: PixelSize) -> Option<&Framebuffer> {
        self.fb.ensure_size(size);
        let palette = palette_by_name(&self.color_scheme);

        self.fb.scroll_down(self.speed);

        let cols = (size.width as usize).min(self.row.len());
        for y in 0..self.speed.min(size.height) {
            for c in 0..cols {
                let rgb = palette_lookup(self.row[c], palette);
                self.fb.put(c as u32, y, rgb);
            }
        }
        Some(&self.fb)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(SpectrogramViz::new())]
}

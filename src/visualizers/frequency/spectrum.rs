/// spectrum.rs — Classic log-spaced vertical frequency bars (software-rendered).
///
/// A native port of the terminal "spectrum" visualizer.  Frequency runs along
/// X (log-spaced, 30 Hz–18 kHz); each bar's height is its smoothed band
/// energy, with a peak-hold cap that lingers then decays.  The per-bar
/// smoothing and peak logic is shared with the terminal app via the vendored
/// `SpectrumBars` helper, so behaviour matches the original exactly — only the
/// rendering (true-colour RGB rectangles instead of ANSI cells) is new.
///
/// Config:
///   gain          — band-energy multiplier before clamping to 1.0
///   bars          — number of frequency bars (8–128)
///   color_scheme  — spectrum (rainbow by frequency) / heat / ice / phosphor / mono
///   style         — solid bars or segmented LED blocks

use crate::config::merge_config;
use crate::palette::{palette_by_name, palette_lookup, PALETTE_SPECTRUM};
use crate::tui::visualizer::SpectrumBars;
use crate::visualizer::{AudioFrame, Framebuffer, PixelSize, RenderMode, Visualizer};

const CONFIG_VERSION: u64 = 1;

pub struct SpectrumViz {
    fb: Framebuffer,
    bars: SpectrumBars,
    // ── Config ────────────────────────────────────────────────────────────────
    gain: f32,
    bar_count: usize,
    color_scheme: String,
    style: String,
}

impl SpectrumViz {
    pub fn new() -> Self {
        Self {
            fb: Framebuffer::new(1, 1),
            bars: SpectrumBars::new(64),
            gain: 1.0,
            bar_count: 64,
            color_scheme: "spectrum".to_string(),
            style: "solid".to_string(),
        }
    }
}

impl Visualizer for SpectrumViz {
    fn name(&self) -> &str {
        "spectrum"
    }
    fn description(&self) -> &str {
        "Log-spaced vertical frequency bars (software-rendered)"
    }
    fn mode(&self) -> RenderMode {
        RenderMode::Software
    }

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
                    "name": "bars",
                    "display_name": "Bars",
                    "type": "int",
                    "value": 64,
                    "min": 8,
                    "max": 128
                },
                {
                    "name": "color_scheme",
                    "display_name": "Color Scheme",
                    "type": "enum",
                    "value": "spectrum",
                    "variants": ["spectrum", "heat", "ice", "phosphor", "mono"]
                },
                {
                    "name": "style",
                    "display_name": "Style",
                    "type": "enum",
                    "value": "solid",
                    "variants": ["solid", "segmented"]
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
                    "bars" => {
                        let v = entry["value"]
                            .as_i64()
                            .or_else(|| entry["value"].as_f64().map(|f| f as i64))
                            .unwrap_or(64);
                        self.bar_count = (v as usize).clamp(8, 128);
                    }
                    "color_scheme" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.color_scheme = s.to_string();
                        }
                    }
                    "style" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.style = s.to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(merged)
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, _size: PixelSize) {
        self.bars.resize(self.bar_count);
        self.bars.update(&audio.fft, dt);
    }

    fn render_software(&mut self, size: PixelSize) -> Option<&Framebuffer> {
        self.fb.ensure_size(size);
        self.fb.clear();

        let w = size.width as usize;
        let h = size.height as usize;
        if w == 0 || h == 0 {
            return Some(&self.fb);
        }

        let n = self.bar_count.max(1).min(self.bars.smoothed.len());
        let palette = palette_by_name(&self.color_scheme);
        let rainbow = self.color_scheme == "spectrum";
        let segmented = self.style == "segmented";

        // Vertical brightness gradient LUT (0 = bottom … h-1 = top), reused by
        // every bar in the height-coloured schemes.
        let grad: Vec<[u8; 3]> = (0..h)
            .map(|yy| palette_lookup(yy as f32 / (h - 1).max(1) as f32, palette))
            .collect();

        let slot = (w / n).max(1);
        let gap = (slot / 8).max(1);
        let bar_w = slot.saturating_sub(gap).max(1);
        let seg_h = (h / 48).max(3); // segmented-block period in pixels

        for i in 0..n {
            let x0 = (i * slot) as u32;
            if i * slot >= w {
                break;
            }
            let bw = bar_w as u32;

            let bar_color = if rainbow {
                palette_lookup(i as f32 / (n - 1).max(1) as f32, PALETTE_SPECTRUM)
            } else {
                [0, 0, 0]
            };

            let bar_px = ((self.bars.smoothed[i] * self.gain).min(1.0) * h as f32) as usize;
            for yy in 0..bar_px {
                // Segmented (LED) style leaves a 1px dark gap between blocks.
                if segmented && (yy % seg_h) == seg_h - 1 {
                    continue;
                }
                let y = (h - 1 - yy) as u32;
                let color = if rainbow {
                    // Fade toward the base so bars have a little depth.
                    let f = 0.55 + 0.45 * (yy as f32 / bar_px.max(1) as f32);
                    [
                        (bar_color[0] as f32 * f) as u8,
                        (bar_color[1] as f32 * f) as u8,
                        (bar_color[2] as f32 * f) as u8,
                    ]
                } else {
                    grad[yy.min(h - 1)]
                };
                self.fb.fill_rect(x0, y, bw, 1, color);
            }

            // Peak-hold cap: a bright 2px marker at the held peak height.
            let ppx = ((self.bars.peaks[i] * self.gain).min(1.0) * h as f32) as usize;
            if ppx > 0 {
                let base = if rainbow { bar_color } else { grad[(ppx - 1).min(h - 1)] };
                let cap = [
                    (base[0] as u16 + 120).min(255) as u8,
                    (base[1] as u16 + 120).min(255) as u8,
                    (base[2] as u16 + 120).min(255) as u8,
                ];
                let top = h.saturating_sub(ppx) as u32;
                self.fb.fill_rect(x0, top, bw, 2, cap);
            }
        }

        Some(&self.fb)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(SpectrumViz::new())]
}

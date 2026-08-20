/// vu.rs — Stereo / mono VU meter (software-rendered).
///
/// A native port of the terminal "vu" visualizer.  Each channel's RMS level
/// is smoothed with a fast attack / slow release envelope and drawn as a
/// horizontal bar with a green→yellow→orange→red gradient.  A peak-hold
/// marker rides the loudest recent level, holding briefly before decaying.
///
/// Config:
///   gain  — level multiplier before the envelope
///   mode  — stereo (separate L/R bars) or mono (single averaged bar)

use crate::config::merge_config;
use crate::dsp::rms;
use crate::palette::{palette_lookup, Palette};
use crate::visualizer::{AudioFrame, Framebuffer, PixelSize, RenderMode, Visualizer};

const CONFIG_VERSION: u64 = 1;

// Envelope coefficients (fraction of the *old* value retained each frame).
const RISE: f32 = 0.30;
const FALL: f32 = 0.85;
const PEAK_HOLD: f32 = 1.5;
const PEAK_FALL: f32 = 0.40;

/// Green → yellow → orange → red, indexed by position along the bar.
const VU_PALETTE: Palette = &[
    [0, 220, 60],
    [180, 220, 0],
    [255, 170, 0],
    [255, 40, 0],
];

pub struct VuViz {
    fb: Framebuffer,
    level_l: f32,
    level_r: f32,
    peak_l: f32,
    peak_r: f32,
    timer_l: f32,
    timer_r: f32,
    // ── Config ────────────────────────────────────────────────────────────────
    gain: f32,
    mono: bool,
}

impl VuViz {
    pub fn new() -> Self {
        Self {
            fb: Framebuffer::new(1, 1),
            level_l: 0.0,
            level_r: 0.0,
            peak_l: 0.0,
            peak_r: 0.0,
            timer_l: 0.0,
            timer_r: 0.0,
            gain: 1.0,
            mono: false,
        }
    }

    fn update_channel(level: &mut f32, peak: &mut f32, timer: &mut f32, raw: f32, dt: f32) {
        let alpha = if raw > *level { RISE } else { FALL };
        *level = alpha * *level + (1.0 - alpha) * raw;
        if *level > *peak {
            *peak = *level;
            *timer = 0.0;
        } else {
            *timer += dt;
            if *timer > PEAK_HOLD {
                *peak = (*peak - PEAK_FALL * dt).max(0.0);
            }
        }
    }

    /// Draw one horizontal meter into the framebuffer.
    fn draw_bar(&mut self, x0: usize, y0: usize, bar_w: usize, thickness: usize, level: f32, peak: f32) {
        let y = y0 as u32;
        let th = thickness as u32;

        // Dim background track.
        self.fb.fill_rect(x0 as u32, y, bar_w as u32, th, [22, 22, 28]);

        // Filled portion — one vertical span per column so the gradient runs
        // left→right along the bar.
        let filled = (level.clamp(0.0, 1.0) * bar_w as f32) as usize;
        for xx in 0..filled {
            let frac = xx as f32 / bar_w.max(1) as f32;
            let color = palette_lookup(frac, VU_PALETTE);
            self.fb.fill_rect((x0 + xx) as u32, y, 1, th, color);
        }

        // Peak-hold marker (2px bright bar).
        if peak > 0.01 {
            let pk = (peak.clamp(0.0, 1.0) * bar_w as f32) as usize;
            let base = palette_lookup(pk as f32 / bar_w.max(1) as f32, VU_PALETTE);
            let color = [
                (base[0] as u16 + 90).min(255) as u8,
                (base[1] as u16 + 90).min(255) as u8,
                (base[2] as u16 + 90).min(255) as u8,
            ];
            let px = (x0 + pk).saturating_sub(1) as u32;
            self.fb.fill_rect(px, y, 2, th, color);
        }
    }
}

impl Visualizer for VuViz {
    fn name(&self) -> &str {
        "vu"
    }
    fn description(&self) -> &str {
        "Stereo / mono VU meter (software-rendered)"
    }
    fn mode(&self) -> RenderMode {
        RenderMode::Software
    }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "vu",
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
                    "name": "mode",
                    "display_name": "Mode",
                    "type": "enum",
                    "value": "stereo",
                    "variants": ["stereo", "mono"]
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
                    "mode" => self.mono = entry["value"].as_str() == Some("mono"),
                    _ => {}
                }
            }
        }
        Ok(merged)
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, _size: PixelSize) {
        let (raw_l, raw_r) = if self.mono {
            let m = rms(&audio.mono) * self.gain;
            (m, m)
        } else {
            (rms(&audio.left) * self.gain, rms(&audio.right) * self.gain)
        };
        Self::update_channel(&mut self.level_l, &mut self.peak_l, &mut self.timer_l, raw_l, dt);
        Self::update_channel(&mut self.level_r, &mut self.peak_r, &mut self.timer_r, raw_r, dt);
    }

    fn render_software(&mut self, size: PixelSize) -> Option<&Framebuffer> {
        self.fb.ensure_size(size);
        self.fb.clear();

        let w = size.width as usize;
        let h = size.height as usize;
        if w == 0 || h == 0 {
            return Some(&self.fb);
        }

        let margin_x = w / 12;
        let bar_w = w.saturating_sub(margin_x * 2).max(1);
        let count = if self.mono { 1 } else { 2 };
        let thickness = (h / 7).clamp(4, h);
        let gap = thickness / 2;
        let block_h = count * thickness + (count - 1) * gap;
        let mut y = h.saturating_sub(block_h) / 2;

        if self.mono {
            let (l, p) = (self.level_l, self.peak_l);
            self.draw_bar(margin_x, y, bar_w, thickness, l, p);
        } else {
            let (l, pl) = (self.level_l, self.peak_l);
            self.draw_bar(margin_x, y, bar_w, thickness, l, pl);
            y += thickness + gap;
            let (r, pr) = (self.level_r, self.peak_r);
            self.draw_bar(margin_x, y, bar_w, thickness, r, pr);
        }

        Some(&self.fb)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(VuViz::new())]
}

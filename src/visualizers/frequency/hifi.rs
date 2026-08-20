/// hifi.rs — Hi-Fi spectrum analyzer with labeled frequency bands (software).
///
/// A native port of the terminal "spectrum" visualizer's HiFi/LED themes:
/// twelve fixed, log-spaced octave bands drawn as segmented VFD-style bars
/// with the band's centre frequency printed underneath each one.  Band
/// energies come from the shared `SpectrumBars` helper (sampled at each band's
/// log position), so smoothing and peak-hold match the rest of the app; the
/// frequency labels are rasterised once with fontdue and blitted every frame.
///
/// Config:
///   gain   — band-energy multiplier before clamping to 1.0
///   color  — vfd (teal) / led (red) / amber / green
///   peaks  — show the peak-hold markers above each bar
///   title  — draw the "SPECTRUM ANALYZER" header

use crate::config::merge_config;
use crate::palette::{palette_lookup, Palette};
use crate::tui::visualizer::SpectrumBars;
use crate::visualizer::{AudioFrame, Framebuffer, PixelSize, RenderMode, Visualizer};

const CONFIG_VERSION: u64 = 1;

/// Embedded monospace face used for the frequency / title labels.
static FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSansMono.ttf");

/// Fixed ISO-ish octave band centres and their printed labels.
const BANDS: &[(f32, &str)] = &[
    (25.0, "25"),
    (40.0, "40"),
    (63.0, "63"),
    (100.0, "100"),
    (160.0, "160"),
    (250.0, "250"),
    (500.0, "500"),
    (1000.0, "1k"),
    (2000.0, "2k"),
    (4000.0, "4k"),
    (8000.0, "8k"),
    (16000.0, "16k"),
];

/// Per-scheme colour treatment.
struct Style {
    bar: Palette,      // vertical gradient, low → high
    peak: [u8; 3],
    label: [u8; 3],
    title: [u8; 3],
}

fn style_for(name: &str) -> Style {
    match name {
        "led" => Style {
            bar: &[[40, 0, 0], [150, 12, 0], [255, 60, 20]],
            peak: [255, 190, 130],
            label: [205, 45, 25],
            title: [255, 85, 45],
        },
        "amber" => Style {
            bar: &[[45, 25, 0], [160, 95, 0], [255, 195, 70]],
            peak: [255, 240, 190],
            label: [215, 155, 35],
            title: [255, 205, 95],
        },
        "green" => Style {
            bar: &[[0, 40, 0], [0, 150, 25], [130, 255, 130]],
            peak: [235, 255, 215],
            label: [45, 205, 65],
            title: [150, 255, 130],
        },
        // vfd (teal) — default
        _ => Style {
            bar: &[[0, 40, 45], [0, 125, 135], [90, 235, 245]],
            peak: [225, 255, 255],
            label: [0, 185, 195],
            title: [130, 235, 245],
        },
    }
}

// ── Text rasterisation ──────────────────────────────────────────────────────

/// A pre-rendered text run: greyscale coverage over a tight `w × h` box.
struct Label {
    w: usize,
    h: usize,
    cov: Vec<u8>,
}

/// Lay out and rasterise a short string into a coverage bitmap.
fn rasterize(font: &fontdue::Font, text: &str, px: f32) -> Label {
    let lm = font
        .horizontal_line_metrics(px)
        .unwrap_or(fontdue::LineMetrics {
            ascent: px,
            descent: 0.0,
            line_gap: 0.0,
            new_line_size: px,
        });
    let baseline = lm.ascent;
    let h = ((lm.ascent - lm.descent).ceil() as usize + 1).max(1);

    // First pass: rasterise each glyph and accumulate advances for the width.
    let mut pen = 0.0f32;
    let mut glyphs = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let (m, bmp) = font.rasterize(ch, px);
        glyphs.push((pen, m, bmp));
        pen += m.advance_width;
    }
    let w = (pen.ceil() as usize + 1).max(1);

    let mut cov = vec![0u8; w * h];
    for (gx, m, bmp) in glyphs {
        let x0 = (gx + m.xmin as f32).round() as i32;
        let y0 = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
        for j in 0..m.height {
            for i in 0..m.width {
                let px_ = x0 + i as i32;
                let py_ = y0 + j as i32;
                if px_ < 0 || py_ < 0 || px_ as usize >= w || py_ as usize >= h {
                    continue;
                }
                let idx = py_ as usize * w + px_ as usize;
                cov[idx] = cov[idx].max(bmp[j * m.width + i]);
            }
        }
    }
    Label { w, h, cov }
}

/// Blit a coverage label onto the framebuffer at (x, y), tinted `color`.
/// Coverage acts as alpha over the (already-black) background.
fn blit(fb: &mut Framebuffer, label: &Label, x: i32, y: i32, color: [u8; 3]) {
    for j in 0..label.h {
        for i in 0..label.w {
            let a = label.cov[j * label.w + i];
            if a == 0 {
                continue;
            }
            let px = x + i as i32;
            let py = y + j as i32;
            if px < 0 || py < 0 {
                continue;
            }
            let t = a as f32 / 255.0;
            let rgb = [
                (color[0] as f32 * t) as u8,
                (color[1] as f32 * t) as u8,
                (color[2] as f32 * t) as u8,
            ];
            fb.put(px as u32, py as u32, rgb);
        }
    }
}

// ── Visualizer ────────────────────────────────────────────────────────────────

pub struct HifiViz {
    fb: Framebuffer,
    bars: SpectrumBars,
    font: fontdue::Font,
    /// Cached band labels + title, rebuilt only when the font size changes.
    band_labels: Vec<Label>,
    title_label: Option<Label>,
    cached_label_px: u32,
    cached_title_px: u32,
    // ── Config ────────────────────────────────────────────────────────────────
    gain: f32,
    color: String,
    show_peaks: bool,
    show_title: bool,
}

impl HifiViz {
    pub fn new() -> Self {
        let font = fontdue::Font::from_bytes(FONT_BYTES, fontdue::FontSettings::default())
            .expect("embedded font must parse");
        Self {
            fb: Framebuffer::new(1, 1),
            // High enough resolution to sample twelve log-spaced positions.
            bars: SpectrumBars::new(96),
            font,
            band_labels: Vec::new(),
            title_label: None,
            cached_label_px: 0,
            cached_title_px: 0,
            gain: 1.0,
            color: "vfd".to_string(),
            show_peaks: true,
            show_title: true,
        }
    }

    /// Sample the twelve band energies (smoothed, peak) at their log positions.
    fn band_values(&self) -> Vec<(f32, f32)> {
        let n = self.bars.smoothed.len().max(1);
        let log_lo = 30f32.log10();
        let log_hi = 18_000f32.log10();
        BANDS
            .iter()
            .map(|(freq, _)| {
                let frac = (freq.log10() - log_lo) / (log_hi - log_lo);
                let idx = ((frac * (n - 1) as f32) as usize).min(n - 1);
                (
                    (self.bars.smoothed[idx] * self.gain).min(1.0),
                    (self.bars.peaks[idx] * self.gain).min(1.0),
                )
            })
            .collect()
    }

    /// Rebuild the cached labels if the requested pixel sizes changed.
    fn ensure_labels(&mut self, label_px: u32, title_px: u32) {
        if label_px != self.cached_label_px {
            self.band_labels = BANDS
                .iter()
                .map(|(_, lbl)| rasterize(&self.font, lbl, label_px as f32))
                .collect();
            self.cached_label_px = label_px;
        }
        if title_px != self.cached_title_px {
            self.title_label = if title_px > 0 {
                Some(rasterize(&self.font, "SPECTRUM ANALYZER", title_px as f32))
            } else {
                None
            };
            self.cached_title_px = title_px;
        }
    }
}

impl Visualizer for HifiViz {
    fn name(&self) -> &str {
        "hifi"
    }
    fn description(&self) -> &str {
        "Hi-Fi spectrum analyzer — labeled octave bands (software-rendered)"
    }
    fn mode(&self) -> RenderMode {
        RenderMode::Software
    }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "hifi",
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
                    "name": "color",
                    "display_name": "Color",
                    "type": "enum",
                    "value": "vfd",
                    "variants": ["vfd", "led", "amber", "green"]
                },
                {
                    "name": "peaks",
                    "display_name": "Peak Markers",
                    "type": "bool",
                    "value": true
                },
                {
                    "name": "title",
                    "display_name": "Title",
                    "type": "bool",
                    "value": true
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
                    "color" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.color = s.to_string();
                        }
                    }
                    "peaks" => self.show_peaks = entry["value"].as_bool().unwrap_or(true),
                    "title" => self.show_title = entry["value"].as_bool().unwrap_or(true),
                    _ => {}
                }
            }
        }
        Ok(merged)
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, _size: PixelSize) {
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

        let n = BANDS.len();

        // ── Layout: title strip on top, label strip on the bottom, bars between.
        let slot = (w * 9 / 10 / n).max(1);
        let gap = (slot / 6).max(1);
        let bar_w = slot.saturating_sub(gap).max(1);
        let total_w = slot * n;
        let left_pad = w.saturating_sub(total_w) / 2;

        let label_px = ((slot as f32 * 0.55).clamp(9.0, 22.0)) as u32;
        let title_px = if self.show_title {
            ((h as f32 * 0.035).clamp(12.0, 24.0)) as u32
        } else {
            0
        };
        self.ensure_labels(label_px, title_px);

        let title_h = self
            .title_label
            .as_ref()
            .map(|l| l.h + 12)
            .unwrap_or(0);
        let label_h = label_px as usize + 10;
        let vis_top = title_h;
        let vis_bottom = h.saturating_sub(label_h);
        if vis_bottom <= vis_top {
            return Some(&self.fb);
        }
        let vis_h = vis_bottom - vis_top;
        let seg = (vis_h / 22).max(3); // segmented-bar block height

        let style = style_for(&self.color);
        let bands = self.band_values();

        for (bi, &(bh, ph)) in bands.iter().enumerate() {
            let x0 = (left_pad + bi * slot) as u32;
            let bw = bar_w as u32;

            let bar_px = (bh * vis_h as f32) as usize;
            for yy in 0..bar_px {
                // Leave a 1px dark gap between segments for the VFD look.
                if yy % seg == seg - 1 {
                    continue;
                }
                let frac = yy as f32 / vis_h.max(1) as f32;
                let color = palette_lookup(frac, style.bar);
                let y = (vis_bottom - 1 - yy) as u32;
                self.fb.fill_rect(x0, y, bw, 1, color);
            }

            // Peak-hold marker (2px) above the bar.
            if self.show_peaks && ph > 0.02 {
                let ppx = (ph * vis_h as f32) as usize;
                let top = vis_bottom.saturating_sub(ppx) as u32;
                self.fb.fill_rect(x0, top, bw, 2, style.peak);
            }

            // Frequency label centred under the bar.
            if let Some(lbl) = self.band_labels.get(bi) {
                let cx = x0 as i32 + bar_w as i32 / 2;
                let lx = cx - lbl.w as i32 / 2;
                let ly = vis_bottom as i32 + 5;
                blit(&mut self.fb, lbl, lx, ly, style.label);
            }
        }

        // Title header, centred.
        if let Some(title) = self.title_label.as_ref() {
            let tx = (w as i32 - title.w as i32) / 2;
            blit(&mut self.fb, title, tx, 6, style.title);
        }

        Some(&self.fb)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(HifiViz::new())]
}

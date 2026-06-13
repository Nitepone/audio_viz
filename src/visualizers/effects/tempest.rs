/// tempest.rs — Audio-reactive thunderstorm with fractal lightning.
///
/// A layered storm where each part of the mix drives a different element:
/// mid energy churns a turbulent cloud deck, high-frequency energy controls
/// rain density and fall speed, stereo imbalance bends the rain with wind
/// shear, and detected beats hurl branching fractal lightning bolts (midpoint
/// jitter with recursive forks) from cloud base to ground, flash-illuminating
/// the whole scene. Weaker high-band onsets flicker as distant sheet
/// lightning inside the clouds.
///
/// Config:
///   gain             — 0–4:   input gain applied to the FFT
///   rain_density     — 0–2:   multiplier on rain drop count
///   bolt_sensitivity — 0.2–3: beat detector sensitivity (higher = more strikes)
///   color_scheme     — enum:  lightning palette (electric, neon, gold, arctic)

// ── Index: helpers@37 · gen_bolt@78 · TempestViz@125 · new@155 · palette@181 · impl@196 · config@200 · set_config@240 · tick@268 · render@406 · register@542

use rand::Rng;
use crate::beat::{BeatDetector, BeatDetectorConfig};
use crate::visualizer::{
    merge_config, pad_frame, status_bar,
    AudioFrame, TermSize, Visualizer,
};
use crate::visualizer_utils::{
    band_energy, brightness_char, mag_to_frac, palette_lookup, rms,
    smooth_asymmetric,
    PALETTE_ARCTIC, PALETTE_GOLD, PALETTE_ICE, PALETTE_NEON,
};

const CONFIG_VERSION: u64 = 1;

// ── Noise / terrain helpers ─────────────────────────────────────────────────

/// Deterministic integer-lattice hash → [0, 1].
#[inline]
fn hash01(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393)
        ^ (y as u32).wrapping_mul(668_265_263)
        ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65_535.0
}

/// Bilinear value noise over the integer lattice with smoothstep fade.
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let fx = x - x.floor();
    let fy = y - y.floor();
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let n00 = hash01(xi, yi, seed);
    let n10 = hash01(xi + 1, yi, seed);
    let n01 = hash01(xi, yi + 1, seed);
    let n11 = hash01(xi + 1, yi + 1, seed);
    let nx0 = n00 + (n10 - n00) * sx;
    let nx1 = n01 + (n11 - n01) * sx;
    nx0 + (nx1 - nx0) * sy
}

/// Two-octave fractal noise for the cloud deck.
fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    0.65 * value_noise(x, y, seed) + 0.35 * value_noise(x * 2.3 + 7.1, y * 2.1 + 3.7, seed ^ 0x9E37)
}

/// Row index of the topmost ground cell in column `c` (1–3 cells tall,
/// stepped every 4 columns so the silhouette reads as low hills).
#[inline]
fn ground_top(c: usize, vis: usize) -> usize {
    let h = 1 + (hash01((c / 4) as i32, 7, 0x00C0_FFEE) * 2.999) as usize;
    vis.saturating_sub(h)
}

/// Walk a lightning bolt downward from (x0, y0) to end_y with per-row jitter
/// and a slowly re-randomised horizontal bias, recursively forking dimmer
/// branches that terminate early. Cells are appended as (col, row, intensity).
fn gen_bolt<R: Rng>(
    rng: &mut R,
    x0: f32,
    y0: i32,
    end_y: i32,
    cols: i32,
    wind: f32,
    depth: u8,
    intensity: f32,
    cells: &mut Vec<(i16, i16, f32)>,
) {
    let mut x = x0;
    let mut bias = rng.gen_range(-0.5..0.5) + wind * 0.9;
    for y in y0..end_y {
        let xi = (x.round() as i32).clamp(0, cols - 1);
        cells.push((xi as i16, y as i16, intensity));
        if rng.gen_range(0.0..1.0f32) < 0.18 {
            bias = rng.gen_range(-1.0..1.0) + wind * 0.9;
        }
        x = (x + bias + rng.gen_range(-0.9..0.9)).clamp(0.0, (cols - 1) as f32);
        if depth > 0 && end_y - y > 3 && rng.gen_range(0.0..1.0f32) < 0.10 {
            let b_end = y + rng.gen_range(2..(end_y - y));
            gen_bolt(rng, x, y, b_end, cols, wind * 1.5, depth - 1, intensity * 0.55, cells);
        }
    }
}

// ── Entities ────────────────────────────────────────────────────────────────

struct Drop {
    x: f32,
    y: f32,
    speed: f32, // base fall speed, rows/sec
}

struct Splash {
    x: usize,
    y: usize,
    ttl: f32,
}

struct Bolt {
    cells: Vec<(i16, i16, f32)>,
    age: f32,
    ttl: f32,
}

pub struct TempestViz {
    t: f32,
    source: String,
    // smoothed, perceptually normalised audio bands ∈ [0, 1]
    bass: f32,
    mid: f32,
    high: f32,
    // wind ∈ [-1, 1]: stereo imbalance + slow ambient breeze
    wind: f32,
    cloud_phase: f32,
    beat: BeatDetector,
    strike_timer: f32,
    // sheet-lightning onset tracking (raw linear high-band flux)
    prev_high_raw: f32,
    high_flux_avg: f32,
    // scene state
    drops: Vec<Drop>,
    splashes: Vec<Splash>,
    bolts: Vec<Bolt>,
    flash: f32,
    sheet: f32,
    sheet_x: f32,
    // ── Config fields ──────────────────────────────────────────────────────
    gain: f32,
    rain_density: f32,
    bolt_sensitivity: f32,
    color_scheme: String,
}

impl TempestViz {
    pub fn new(source: &str) -> Self {
        Self {
            t: 0.0,
            source: source.to_string(),
            bass: 0.0,
            mid: 0.0,
            high: 0.0,
            wind: 0.0,
            cloud_phase: 0.0,
            beat: BeatDetector::new(BeatDetectorConfig::standard()),
            strike_timer: 1.0,
            prev_high_raw: 0.0,
            high_flux_avg: 0.0,
            drops: Vec::new(),
            splashes: Vec::new(),
            bolts: Vec::new(),
            flash: 0.0,
            sheet: 0.0,
            sheet_x: 0.0,
            gain: 1.0,
            rain_density: 1.0,
            bolt_sensitivity: 1.0,
            color_scheme: "electric".to_string(),
        }
    }

    fn palette(&self) -> &'static [u8] {
        match self.color_scheme.as_str() {
            "neon"   => PALETTE_NEON,
            "gold"   => PALETTE_GOLD,
            "arctic" => PALETTE_ARCTIC,
            _        => PALETTE_ICE,
        }
    }

    #[inline]
    fn cloud_rows(vis: usize) -> usize {
        (vis / 4).max(3).min(vis)
    }
}

impl Visualizer for TempestViz {
    fn name(&self)        -> &str { "tempest" }
    fn description(&self) -> &str { "Thunderstorm with beat-triggered fractal lightning" }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "tempest",
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
                    "name": "rain_density",
                    "display_name": "Rain Density",
                    "type": "float",
                    "value": 1.0,
                    "min": 0.0,
                    "max": 2.0
                },
                {
                    "name": "bolt_sensitivity",
                    "display_name": "Lightning Sensitivity",
                    "type": "float",
                    "value": 1.0,
                    "min": 0.2,
                    "max": 3.0
                },
                {
                    "name": "color_scheme",
                    "display_name": "Lightning Palette",
                    "type": "enum",
                    "value": "electric",
                    "variants": ["electric", "neon", "gold", "arctic"]
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
                    "gain" => {
                        self.gain = entry["value"].as_f64().unwrap_or(1.0) as f32;
                    }
                    "rain_density" => {
                        self.rain_density = entry["value"].as_f64().unwrap_or(1.0) as f32;
                    }
                    "bolt_sensitivity" => {
                        self.bolt_sensitivity = entry["value"].as_f64().unwrap_or(1.0) as f32;
                    }
                    "color_scheme" => {
                        if let Some(s) = entry["value"].as_str() {
                            self.color_scheme = s.to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(merged)
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: TermSize) {
        self.t += dt;
        let cols = size.cols as usize;
        let vis  = (size.rows as usize).saturating_sub(1).max(1);
        if cols == 0 {
            return;
        }

        // ── Band analysis (dB-normalised, asymmetric smoothing) ─────────────
        let fft = &audio.fft;
        let g   = self.gain;
        let bass_n = mag_to_frac(band_energy(fft, 20.0, 250.0) * g, -60.0, -16.0);
        let mid_n  = mag_to_frac(band_energy(fft, 250.0, 4_000.0) * g, -58.0, -18.0);
        let high_n = mag_to_frac(band_energy(fft, 4_000.0, 12_000.0) * g, -64.0, -22.0);
        self.bass = smooth_asymmetric(self.bass, bass_n, 0.30, 0.88);
        self.mid  = smooth_asymmetric(self.mid,  mid_n,  0.35, 0.90);
        self.high = smooth_asymmetric(self.high, high_n, 0.25, 0.92);

        // ── Wind: stereo imbalance plus a slow ambient breeze ───────────────
        let l = rms(&audio.left);
        let r = rms(&audio.right);
        let bal = if l + r > 1e-5 { ((r - l) / (l + r)).clamp(-1.0, 1.0) } else { 0.0 };
        let breeze = (self.t * 0.11).sin() * 0.30 + (self.t * 0.043).sin() * 0.20;
        let target = (bal * 1.4 + breeze).clamp(-1.0, 1.0);
        self.wind = self.wind * 0.96 + target * 0.04;

        // Clouds churn faster when the mids are busy.
        self.cloud_phase += dt * (0.25 + self.mid * 2.5);

        let cloud_rows = Self::cloud_rows(vis);
        let mut rng = rand::thread_rng();

        // ── Lightning: full bolts on beats, sheet flicker on weak onsets ────
        self.beat.set_sensitivity(self.bolt_sensitivity);
        self.beat.update(fft, dt);
        self.strike_timer += dt;

        if self.beat.is_beat() {
            let intensity = self.beat.beat_intensity();
            if intensity >= 0.18 && self.strike_timer > 0.22 && self.bolts.len() < 4 {
                let x0 = rng.gen_range(cols as f32 * 0.08..cols as f32 * 0.92);
                let mut cells = Vec::new();
                gen_bolt(
                    &mut rng,
                    x0,
                    cloud_rows as i32 - 1,
                    vis as i32 - 1,
                    cols as i32,
                    self.wind,
                    2,
                    1.0,
                    &mut cells,
                );
                self.bolts.push(Bolt {
                    cells,
                    age: 0.0,
                    ttl: 0.50 + rng.gen_range(0.0..0.25) + intensity.min(1.0) * 0.15,
                });
                self.flash = self.flash.max(0.55 + 0.45 * intensity.min(1.0));
                self.strike_timer = 0.0;
            } else {
                // Beat detected but no bolt budget: distant sheet flicker.
                self.sheet = self.sheet.max(0.5);
                self.sheet_x = rng.gen_range(0.0..cols as f32);
            }
        }

        // Sheet lightning from sharp high-band transients between beats
        // (adaptive flux threshold so it works at any signal level).
        let raw_high = band_energy(fft, 4_000.0, 12_000.0) * g;
        let flux = (raw_high - self.prev_high_raw).max(0.0);
        self.prev_high_raw = raw_high;
        self.high_flux_avg = self.high_flux_avg * 0.92 + flux * 0.08;
        if flux > self.high_flux_avg * 2.6 + 5e-5 && self.beat.time_since_beat() > 0.25 {
            self.sheet = self.sheet.max(0.65);
            self.sheet_x = rng.gen_range(0.0..cols as f32);
        }

        self.flash = (self.flash - dt * 2.2).max(0.0);
        self.sheet = (self.sheet - dt * 4.5).max(0.0);

        // ── Rain: drop count tracks high-frequency energy ───────────────────
        let target_drops = ((cols as f32 * self.rain_density * (0.10 + self.high * 1.10))
            .min(cols as f32 * 1.6)) as usize;
        let mut spawned = 0;
        while self.drops.len() < target_drops && spawned < 8 {
            self.drops.push(Drop {
                x: rng.gen_range(0.0..cols as f32),
                y: rng.gen_range(0.0..vis as f32),
                speed: rng.gen_range(9.0..22.0),
            });
            spawned += 1;
        }
        if self.drops.len() > target_drops {
            let excess = (self.drops.len() - target_drops).min(3);
            self.drops.truncate(self.drops.len() - excess);
        }

        for d in &mut self.drops {
            let v = d.speed * (0.55 + self.high * 0.90);
            d.y += v * dt;
            d.x += self.wind * v * 0.45 * dt;
            if d.x < 0.0 {
                d.x += cols as f32;
            } else if d.x >= cols as f32 {
                d.x -= cols as f32;
            }
        }

        // Ground impacts: splash, then respawn at the cloud base.
        for i in 0..self.drops.len() {
            let gx = (self.drops[i].x as usize).min(cols - 1);
            let gt = ground_top(gx, vis);
            if self.drops[i].y as usize >= gt {
                if self.splashes.len() < 96 {
                    self.splashes.push(Splash { x: gx, y: gt.saturating_sub(1), ttl: 0.18 });
                }
                self.drops[i].x = rng.gen_range(0.0..cols as f32);
                self.drops[i].y = rng.gen_range(cloud_rows as f32 * 0.6..cloud_rows as f32 + 2.0);
                self.drops[i].speed = rng.gen_range(9.0..22.0);
            }
        }

        for s in &mut self.splashes {
            s.ttl -= dt;
        }
        self.splashes.retain(|s| s.ttl > 0.0);

        for b in &mut self.bolts {
            b.age += dt;
        }
        self.bolts.retain(|b| b.age < b.ttl);
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = rows.saturating_sub(1).max(1);
        if cols == 0 {
            return pad_frame(Vec::new(), rows, cols);
        }

        let pal = self.palette();
        let cloud_rows = Self::cloud_rows(vis);
        // (char, ansi-256 colour, bold) per cell
        let mut buf: Vec<(char, u8, bool)> = vec![(' ', 0, false); vis * cols];

        // ── Cloud deck: fractal noise, lit by flash and sheet lightning ─────
        let jolt = (self.t * 97.0).sin() * self.flash * 1.6; // thunder shake
        for r in 0..cloud_rows.min(vis) {
            let falloff = 1.0 - (r as f32 / cloud_rows as f32) * 0.55;
            for c in 0..cols {
                let nx = (c as f32 + jolt) * 0.085 + self.cloud_phase;
                let ny = r as f32 * 0.35 + self.cloud_phase * 0.15;
                let n = fbm(nx, ny, 0x5EED);
                let thr = 0.52 - self.mid * 0.20;
                let val = ((n * falloff - thr) / (1.0 - thr)).clamp(0.0, 1.0);
                if val > 0.02 {
                    let dx = (c as f32 - self.sheet_x) / (cols as f32 * 0.16 + 1.0);
                    let sheet_boost = self.sheet * (-dx * dx).exp();
                    let grey = (234.0 + val * 6.0 + self.flash * 12.0 + sheet_boost * 12.0)
                        .min(253.0) as u8;
                    let ch = brightness_char(
                        (val * 0.75 + self.flash * 0.25 + sheet_boost * 0.30).min(1.0),
                    );
                    buf[r * cols + c] = (ch, grey, false);
                }
            }
        }

        // ── Rain: slanted by wind, never overdraws clouds ────────────────────
        let rain_ch = if self.wind > 0.30 {
            '\\'
        } else if self.wind < -0.30 {
            '/'
        } else {
            '|'
        };
        for d in &self.drops {
            let c = (d.x as usize).min(cols - 1);
            let r = d.y as usize;
            if r < vis && buf[r * cols + c].0 == ' ' {
                let frac = 0.20 + (d.speed - 9.0) / 13.0 * 0.18 + self.flash * 0.35;
                buf[r * cols + c] = (rain_ch, palette_lookup(frac, pal), false);
            }
        }

        for s in &self.splashes {
            if s.x < cols && s.y < vis && buf[s.y * cols + s.x].0 == ' ' {
                buf[s.y * cols + s.x] = ('.', palette_lookup(0.45, pal), false);
            }
        }

        // ── Ground silhouette: lit by flash, faint pulse from bass ──────────
        for c in 0..cols {
            let gt = ground_top(c, vis);
            for r in gt..vis {
                let grey = (233.0 + self.bass * 3.0 + self.flash * 14.0).min(250.0) as u8;
                let ch = if r == gt { '▄' } else { '█' };
                buf[r * cols + c] = (ch, grey, false);
            }
        }

        // ── Lightning bolts: stamped into an intensity layer, drawn on top ──
        let mut zap: Vec<f32> = vec![0.0; vis * cols];
        for bolt in &self.bolts {
            let ph = bolt.age / bolt.ttl;
            let alpha = if ph < 0.22 {
                1.0
            } else {
                ((1.0 - (ph - 0.22) / 0.78).max(0.0)).powf(1.6)
            };
            if alpha <= 0.02 {
                continue;
            }
            for &(cx, cy, ci) in &bolt.cells {
                let b = ci * alpha;
                let (cx, cy) = (cx as i32, cy as i32);
                for (sx, sb) in [(cx, b), (cx - 1, b * 0.35), (cx + 1, b * 0.35)] {
                    if sx >= 0 && (sx as usize) < cols && cy >= 0 && (cy as usize) < vis {
                        let idx = cy as usize * cols + sx as usize;
                        if sb > zap[idx] {
                            zap[idx] = sb;
                        }
                    }
                }
            }
        }
        for (idx, &b) in zap.iter().enumerate() {
            if b > 0.04 {
                let ch = if b > 0.85 {
                    '█'
                } else if b > 0.60 {
                    '▓'
                } else if b > 0.35 {
                    '▒'
                } else {
                    '░'
                };
                buf[idx] = (ch, palette_lookup(0.35 + b * 0.65, pal), b > 0.55);
            }
        }

        // ── Serialise ────────────────────────────────────────────────────────
        let mut lines = Vec::with_capacity(rows);
        for r in 0..vis {
            let mut line = String::with_capacity(cols * 14);
            for c in 0..cols {
                let (ch, code, bold) = buf[r * cols + c];
                if ch == ' ' {
                    line.push(' ');
                } else {
                    let bold = if bold { "\x1b[1m" } else { "" };
                    line.push_str(&format!("{bold}\x1b[38;5;{code}m{ch}\x1b[0m"));
                }
            }
            lines.push(line);
        }

        let bpm = self.beat.estimated_bpm();
        let extra = if bpm > 0.0 {
            format!(" | {bpm:.0} bpm | wind {:+.1}", self.wind)
        } else {
            format!(" | wind {:+.1}", self.wind)
        };
        lines.push(status_bar(cols, fps, self.name(), &self.source, &extra));
        pad_frame(lines, rows, cols)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(TempestViz::new(""))]
}


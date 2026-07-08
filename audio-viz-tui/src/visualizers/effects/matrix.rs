/// matrix.rs — Dreams of the Machine.
///
/// Omnidirectional streams of machine-native glyphs — binary, mathematical,
/// and box-drawing symbols — flow across the terminal, driven by audio energy.
///
/// • Streams fall, rise, or move laterally; direction chosen at spawn.
/// • Character pools: binary (01…), math (∀∃∧∨…), box-drawing (╬┼═…).
/// • Color: frequency-banded — lows→deep blue, mids→teal, highs→violet.
///   Strong beats produce a momentary magenta surge on stream heads.
/// • Memory echoes: every cell a stream passes leaves a faint ghost that
///   lingers and slowly fades, giving the screen a smeared, ruminating quality.
/// • Stuck characters: occasionally a glyph freezes in place as an obsessive
///   loop, resisting scrambling until a strong beat dislodges it.
/// • Corruption bursts: clusters of block glyphs erupt on strong beats.
/// • Depth layers: background streams are dim and slow; foreground streams
///   are bold and fast, creating a sense of Z-axis depth.
/// • Attractor vortex: a drifting invisible point bends streams toward or
///   away from it; polarity oscillates and snaps on strong beats. The point
///   is rendered as a pulsing ⊙/⊗ with a faint halo ring.
/// • Ripple pulses: expanding rings of light radiate outward on every beat.
/// • Stream bifurcation: streams near the attractor occasionally fork,
///   spawning a child stream going perpendicular.
/// • Void zones: a region of the screen periodically goes dark, then
///   streams rush back in — death and rebirth.
/// • Cellular automata: small patches of Game of Life (B3/S23) evolve
///   beneath the streams, seeded with random noise.
/// • Cosmic ray bit flips: a single random cell briefly flashes an
///   unexpected color, as if struck by a stray particle.
/// • The great drain: on a very strong beat, all streams surge toward the
///   attractor before scattering outward in chaos.
/// • Standing wave sync: a row of the screen momentarily unifies to a
///   single repeating glyph — a clock cycle made visible.

// ── Index: CharPool@75 · pool_chars@77 · band_color@81 · ca_step@89 · make_drop@109 · is_offscreen@146 · VoidZone@154 · CaPatch@177 · SyncWave@189 · Drop@196 · StuckChar@210 · EchoCell@218 · Ripple@225 · MatrixViz@234 · new@268 · sync_drops@300 · sync_echo@308 · impl@316 · config@320 · set_config@331 · on_resize@345 · tick@350 · render@651 · register@826
use std::f32::consts::TAU;
use rand::Rng;
use crate::beat::{BeatDetector, BeatDetectorConfig};
use crate::visualizer::{
    merge_config, pad_frame, status_bar,
    AudioFrame, SpectrumBars, TermSize, Visualizer,
};
use crate::visualizer_utils::with_gained_fft;

const CONFIG_VERSION: u64 = 2;

// ── Character pools ──────────────────────────────────────────────────────────
const BINARY: &[char] = &['0','1','0','0','1','0','1','1','0','0'];
const MATH:   &[char] = &[
    '∀','∃','¬','∧','∨','⊕','∴','≡','∈','∅',
    '∞','∑','∂','∇','λ','μ','π','σ','φ','ψ','ω','⊗','⊞',
];
const BOX: &[char] = &[
    '╬','┼','╋','═','║','├','┤','┬','┴',
    '╔','╗','╚','╝','░','▒','─','│',
];
const CORRUPT: &[char] = &['▓','█','░','▒','■','□','▪','▫','?','!','×','▌','▐','◆'];
const SYNC_CHARS: &[char] = &['═','─','━','▬','≡','≣'];

// ── Color palettes (lows→blue, mids→teal, highs→violet) ─────────────────────
const BLUE_DEEP: &[u8] = &[17, 18, 19, 20, 21, 27];
const TEAL_CYAN: &[u8] = &[30, 37, 38, 44, 45, 51];
const VIOLET:    &[u8] = &[55, 93, 99, 105, 135, 141];
const BEAT_HUES: &[u8] = &[198, 199, 200, 163, 201, 165];
const DIM_TRAIL: u8    = 238;

// ── Stream directions (unit vectors) — weighted 3:1:1:1 toward downward ─────
const DIRS: &[(f32, f32)] = &[
    ( 0.0,  1.0), ( 0.0,  1.0), ( 0.0,  1.0),  // down ×3
    ( 0.0, -1.0),                                // up
    ( 1.0,  0.0),                                // right
    (-1.0,  0.0),                                // left
];

#[derive(Clone, Copy)]
enum CharPool { Binary, Math, Box }

fn pool_chars(p: CharPool) -> &'static [char] {
    match p { CharPool::Binary => BINARY, CharPool::Math => MATH, CharPool::Box => BOX }
}

fn band_color(x: usize, cols: usize) -> u8 {
    let t = x as f32 / cols.max(1) as f32;
    if t < 0.33      { BLUE_DEEP[(t * 18.0) as usize % BLUE_DEEP.len()] }
    else if t < 0.66 { TEAL_CYAN[((t-0.33)*18.0) as usize % TEAL_CYAN.len()] }
    else             { VIOLET[((t-0.66)*18.0) as usize % VIOLET.len()] }
}

/// Game of Life B3/S23 step — writes next generation from `src` into `dst`.
fn ca_step(w: usize, h: usize, src: &[bool], dst: &mut [bool]) {
    for r in 0..h {
        for c in 0..w {
            let mut n = 0u8;
            for dr in -1i32..=1 {
                for dc in -1i32..=1 {
                    if dr == 0 && dc == 0 { continue; }
                    let nr = r as i32 + dr;
                    let nc = c as i32 + dc;
                    if nr >= 0 && nr < h as i32 && nc >= 0 && nc < w as i32 {
                        if src[nr as usize * w + nc as usize] { n += 1; }
                    }
                }
            }
            let alive = src[r * w + c];
            dst[r * w + c] = if alive { n == 2 || n == 3 } else { n == 3 };
        }
    }
}

fn make_drop(rows: usize, cols: usize, rng: &mut impl Rng) -> Drop {
    let (dx, dy) = DIRS[rng.gen_range(0..DIRS.len())];

    // Depth: 0=background (dim/slow), 1=midground, 2=foreground (bold/fast)
    let depth: u8 = match rng.gen_range(0u8..10) {
        0..=2 => 0,
        3..=7 => 1,
        _     => 2,
    };
    let speed_mult  = [0.32f32, 1.0, 1.55][depth as usize];
    let trail_scale = [0.55f32, 1.0, 1.40][depth as usize];
    let speed = rng.gen_range(0.4f32..1.3) * speed_mult;
    let trail = ((rng.gen_range(6usize..22) as f32) * trail_scale) as usize;
    let pool  = match rng.gen_range(0u8..3) {
        0 => CharPool::Binary, 1 => CharPool::Math, _ => CharPool::Box,
    };
    let chars = pool_chars(pool);
    let seq: Vec<char> = (0..24).map(|_| chars[rng.gen_range(0..chars.len())]).collect();

    let (x, y) = if dy > 0.0 {
        (rng.gen_range(0.0..cols as f32), rng.gen_range(-(rows as f32)..0.0))
    } else if dy < 0.0 {
        (rng.gen_range(0.0..cols as f32), rows as f32 + rng.gen_range(0.0..rows as f32 * 0.5))
    } else if dx > 0.0 {
        (-(trail as f32 + 2.0), rng.gen_range(0.0..rows as f32))
    } else {
        (cols as f32 + trail as f32 + 2.0, rng.gen_range(0.0..rows as f32))
    };

    Drop {
        x, y, dx, dy, speed, trail, seq,
        flip_t: rng.gen_range(0.0..0.08),
        hue:    band_color(x.max(0.0) as usize, cols),
        pool, depth,
    }
}

fn is_offscreen(d: &Drop, rows: usize, cols: usize) -> bool {
    let m = d.trail as f32 + 2.0;
    (d.dy > 0.0 && d.y - m > rows as f32) ||
    (d.dy < 0.0 && d.y + m < 0.0)         ||
    (d.dx > 0.0 && d.x - m > cols as f32) ||
    (d.dx < 0.0 && d.x + m < 0.0)
}

struct VoidZone {
    cx:        f32,
    cy:        f32,
    t:         f32,
    max_r:     f32,
    durations: [f32; 3],  // [expand, hold, collapse]
}

impl VoidZone {
    fn current_radius(&self) -> f32 {
        let [ed, hd, cd] = self.durations;
        if self.t < ed {
            self.max_r * (self.t / ed)
        } else if self.t < ed + hd {
            self.max_r
        } else {
            let prog = (self.t - ed - hd) / cd;
            self.max_r * (1.0 - prog).max(0.0)
        }
    }
    fn is_done(&self) -> bool { self.t >= self.durations.iter().sum::<f32>() }
}

struct CaPatch {
    ox:     i32,   // top-left column (screen space)
    oy:     i32,   // top-left row (screen space)
    w:      usize,
    h:      usize,
    cells:  Vec<bool>,
    next:   Vec<bool>,
    step_t: f32,   // time since last CA step
    life:   f32,
    color:  u8,
}

struct SyncWave {
    row:   usize,
    ch:    char,
    life:  f32,   // 0..1 brightness envelope
    timer: f32,
}

struct Drop {
    x:      f32,
    y:      f32,
    dx:     f32,
    dy:     f32,
    speed:  f32,
    trail:  usize,
    seq:    Vec<char>,
    flip_t: f32,
    hue:    u8,
    pool:   CharPool,
    depth:  u8,   // 0=background · 1=midground · 2=foreground
}

struct StuckChar {
    row:   usize,
    col:   usize,
    ch:    char,
    color: u8,
    life:  f32,
}

#[derive(Clone)]
struct EchoCell {
    ch:    char,
    color: u8,
    life:  f32,
}

struct Ripple {
    cx:     f32,
    cy:     f32,
    radius: f32,
    speed:  f32,
    life:   f32,
    color:  u8,
}

pub struct MatrixViz {
    drops:          Vec<Drop>,
    echo:           Vec<EchoCell>,
    echo_size:      (usize, usize),
    stuck:          Vec<StuckChar>,
    corruption:     Vec<(usize, usize, char, u8, f32)>,
    ripples:        Vec<Ripple>,
    // ── Attractor vortex ──────────────────────────────────────────────────
    attractor:      (f32, f32),
    attractor_vel:  (f32, f32),
    attractor_pull: f32,
    attractor_phase: f32,
    // ── New features ──────────────────────────────────────────────────────
    t:              f32,             // global time for pulsing
    void_zone:      Option<VoidZone>,
    void_cooldown:  f32,
    ca_patches:     Vec<CaPatch>,
    ca_cooldown:    f32,
    cosmic:         Option<(usize, usize, f32, u8)>,  // row, col, life, color
    cosmic_cooldown: f32,
    drain_timer:    f32,
    scatter_timer:  f32,
    sync_wave:      Option<SyncWave>,
    sync_cooldown:  f32,
    // ── Config + audio ────────────────────────────────────────────────────
    bars:           SpectrumBars,
    beat:           BeatDetector,
    beat_flash:     f32,
    beat_hue:       u8,
    source:         String,
    gain:           f32,
}

impl MatrixViz {
    pub fn new(source: &str) -> Self {
        Self {
            drops:           Vec::new(),
            echo:            Vec::new(),
            echo_size:       (0, 0),
            stuck:           Vec::new(),
            corruption:      Vec::new(),
            ripples:         Vec::new(),
            attractor:       (40.0, 12.0),
            attractor_vel:   (0.8, 0.5),
            attractor_pull:  0.5,
            attractor_phase: 0.0,
            t:               0.0,
            void_zone:       None,
            void_cooldown:   25.0,
            ca_patches:      Vec::new(),
            ca_cooldown:     35.0,
            cosmic:          None,
            cosmic_cooldown: 30.0,
            drain_timer:     0.0,
            scatter_timer:   0.0,
            sync_wave:       None,
            sync_cooldown:   15.0,
            bars:            SpectrumBars::new(80),
            beat:            BeatDetector::new(BeatDetectorConfig::standard()),
            beat_flash:      0.0,
            beat_hue:        198,
            source:          source.to_string(),
            gain:            1.0,
        }
    }

    fn sync_drops(&mut self, rows: usize, cols: usize) {
        let mut rng = rand::thread_rng();
        while self.drops.len() < cols {
            self.drops.push(make_drop(rows, cols, &mut rng));
        }
        self.drops.truncate(cols);
    }

    fn sync_echo(&mut self, vis: usize, cols: usize) {
        if self.echo_size != (vis, cols) {
            self.echo = vec![EchoCell { ch: ' ', color: 0, life: 0.0 }; vis * cols];
            self.echo_size = (vis, cols);
        }
    }
}

impl Visualizer for MatrixViz {
    fn name(&self)        -> &str { "matrix" }
    fn description(&self) -> &str { "Dreams of the machine: omnidirectional data streams" }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "matrix",
            "version": CONFIG_VERSION,
            "config": [{
                "name": "gain", "display_name": "Gain",
                "type": "float", "value": 1.0, "min": 0.0, "max": 4.0
            }]
        }).to_string()
    }

    fn set_config(&mut self, json: &str) -> Result<String, String> {
        let merged = merge_config(&self.get_default_config(), json);
        let val: serde_json::Value = serde_json::from_str(&merged)
            .map_err(|e| format!("JSON parse error: {e}"))?;
        if let Some(config) = val["config"].as_array() {
            for entry in config {
                if entry["name"].as_str() == Some("gain") {
                    self.gain = entry["value"].as_f64().unwrap_or(1.0) as f32;
                }
            }
        }
        Ok(merged)
    }

    fn on_resize(&mut self, size: TermSize) {
        self.bars.resize(size.cols as usize);
        self.echo_size = (0, 0);
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: TermSize) {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = rows.saturating_sub(1);

        self.bars.resize(cols);
        with_gained_fft(&audio.fft, self.gain, |fft| self.bars.update(fft, dt));
        self.sync_drops(rows, cols);
        self.sync_echo(vis, cols);

        self.t += dt;

        // ── Advance new-feature timers ────────────────────────────────────────
        // Void zone
        if let Some(ref mut vz) = self.void_zone {
            vz.t += dt;
            if vz.is_done() { self.void_zone = None; }
        }
        self.void_cooldown -= dt;

        // CA patches: step + age
        self.ca_patches.retain_mut(|p| {
            p.life   -= dt;
            p.step_t += dt;
            if p.step_t >= 0.12 {
                p.step_t = 0.0;
                ca_step(p.w, p.h, &p.cells, &mut p.next);
                std::mem::swap(&mut p.cells, &mut p.next);
            }
            p.life > 0.0
        });
        self.ca_cooldown -= dt;

        // Cosmic ray
        if let Some(ref mut c) = self.cosmic { c.2 -= dt; }
        if self.cosmic.as_ref().map_or(false, |c| c.2 <= 0.0) { self.cosmic = None; }
        self.cosmic_cooldown -= dt;

        // Sync wave
        if let Some(ref mut sw) = self.sync_wave {
            sw.timer += dt;
            sw.life = if sw.timer < 0.3 {
                sw.timer / 0.3
            } else if sw.timer < 1.2 {
                1.0
            } else if sw.timer < 1.5 {
                1.0 - (sw.timer - 1.2) / 0.3
            } else {
                0.0
            };
            if sw.timer >= 1.5 { self.sync_wave = None; }
        }
        self.sync_cooldown -= dt;

        // Drain / scatter
        if self.drain_timer > 0.0 {
            self.drain_timer -= dt;
            if self.drain_timer <= 0.0 { self.scatter_timer = 0.5; }
        }
        if self.scatter_timer > 0.0 { self.scatter_timer -= dt; }

        // ── Beat ─────────────────────────────────────────────────────────────
        self.beat.update(&audio.fft, dt);
        let is_beat   = self.beat.is_beat();
        let intensity = self.beat.beat_intensity();

        if is_beat {
            self.beat_flash = intensity.min(2.0);
            let mut rng = rand::thread_rng();
            self.beat_hue = BEAT_HUES[rng.gen_range(0..BEAT_HUES.len())];

            // Every beat: ripple pulse
            let rip_color = if intensity > 1.2 { self.beat_hue }
                            else { TEAL_CYAN[rng.gen_range(0..TEAL_CYAN.len())] };
            if cols > 16 && vis > 6 {
                self.ripples.push(Ripple {
                    cx:     rng.gen_range(8.0..(cols as f32 - 8.0)),
                    cy:     rng.gen_range(3.0..(vis  as f32 - 3.0)),
                    radius: 0.0,
                    speed:  7.0 + intensity * 4.0,
                    life:   1.0,
                    color:  rip_color,
                });
            }

            if intensity > 1.0 { self.stuck.clear(); }

            // Strong beat: corruption burst
            if intensity > 1.2 {
                let n  = (intensity * 5.0) as usize + 3;
                let cr = rng.gen_range(0..vis) as i32;
                let cc = rng.gen_range(0..cols) as i32;
                for _ in 0..n {
                    let r = (cr + rng.gen_range(-5i32..=5)).clamp(0, vis  as i32 - 1) as usize;
                    let c = (cc + rng.gen_range(-10i32..=10)).clamp(0, cols as i32 - 1) as usize;
                    self.corruption.push((r, c, CORRUPT[rng.gen_range(0..CORRUPT.len())],
                                         self.beat_hue, rng.gen_range(0.15f32..0.55)));
                }
            }

            // Strong beat: snap attractor to repulsor + great drain
            if intensity > 1.6 {
                self.attractor_pull = -1.5;
                if self.drain_timer <= 0.0 && self.scatter_timer <= 0.0 {
                    self.drain_timer = 0.6;
                }
                // Early-trigger void zone
                if self.void_zone.is_none() && self.void_cooldown <= 0.0 {
                    let max_r = rng.gen_range(8.0f32..16.0);
                    self.void_zone = Some(VoidZone {
                        cx: rng.gen_range(10.0..(cols as f32 - 10.0).max(11.0)),
                        cy: rng.gen_range(4.0..(vis  as f32 -  4.0).max(5.0)),
                        t: 0.0, max_r,
                        durations: [1.0, rng.gen_range(2.5f32..5.0), 1.3],
                    });
                    self.void_cooldown = rng.gen_range(22.0f32..32.0);
                }
            }
        }
        self.beat_flash = (self.beat_flash - dt * 4.0).max(0.0);

        // ── Attractor drift + polarity oscillation ────────────────────────────
        {
            let (ax, ay)   = self.attractor;
            let (avx, avy) = self.attractor_vel;
            let nx = ax + avx * dt;
            let ny = ay + avy * dt;
            self.attractor_vel.0 = if nx < 8.0 || nx > cols as f32 - 8.0 { -avx } else { avx };
            self.attractor_vel.1 = if ny < 4.0 || ny > vis  as f32 - 4.0 { -avy } else { avy };
            self.attractor.0 = nx.clamp(0.0, cols as f32);
            self.attractor.1 = ny.clamp(0.0, vis  as f32);
        }
        self.attractor_phase += dt * 0.07;
        let natural_pull = self.attractor_phase.sin() * 0.55;
        self.attractor_pull += (natural_pull - self.attractor_pull) * (dt * 1.2).min(1.0);

        // ── Timed spawns ──────────────────────────────────────────────────────
        let mut rng = rand::thread_rng();

        if self.void_zone.is_none() && self.void_cooldown <= 0.0 {
            let max_r = rng.gen_range(6.0f32..14.0);
            self.void_zone = Some(VoidZone {
                cx: rng.gen_range(10.0..(cols as f32 - 10.0).max(11.0)),
                cy: rng.gen_range(4.0..(vis  as f32 -  4.0).max(5.0)),
                t: 0.0, max_r,
                durations: [1.2, rng.gen_range(3.0f32..6.0), 1.5],
            });
            self.void_cooldown = rng.gen_range(22.0f32..32.0);
        }

        if self.ca_patches.len() < 2 && self.ca_cooldown <= 0.0 && cols >= 12 && vis >= 7 {
            let w = 12usize;
            let h = 7usize;
            let cells: Vec<bool> = (0..w * h).map(|_| rng.gen_bool(0.38)).collect();
            let next  = vec![false; w * h];
            self.ca_patches.push(CaPatch {
                ox:     rng.gen_range(0..(cols - w)) as i32,
                oy:     rng.gen_range(0..(vis  - h)) as i32,
                w, h, cells, next, step_t: 0.0,
                life:   rng.gen_range(10.0f32..18.0),
                color:  TEAL_CYAN[rng.gen_range(0..TEAL_CYAN.len())],
            });
            self.ca_cooldown = rng.gen_range(30.0f32..45.0);
        }

        if self.cosmic.is_none() && self.cosmic_cooldown <= 0.0 {
            let colors = [196u8, 231, 226, 201];
            self.cosmic = Some((
                rng.gen_range(0..vis), rng.gen_range(0..cols),
                0.07, colors[rng.gen_range(0..colors.len())],
            ));
            self.cosmic_cooldown = rng.gen_range(20.0f32..50.0);
        }

        if self.sync_wave.is_none() && self.sync_cooldown <= 0.0 && vis > 2 {
            self.sync_wave = Some(SyncWave {
                row:   rng.gen_range(1..vis - 1),
                ch:    SYNC_CHARS[rng.gen_range(0..SYNC_CHARS.len())],
                life:  0.0,
                timer: 0.0,
            });
            self.sync_cooldown = rng.gen_range(10.0f32..20.0);
        }

        // ── Decay timers ──────────────────────────────────────────────────────
        self.corruption.retain_mut(|e| { e.4 -= dt; e.4 > 0.0 });
        self.stuck.retain_mut(|s| { s.life -= dt; s.life > 0.0 });
        self.ripples.retain_mut(|r| {
            r.radius += r.speed * dt;
            r.life    = (r.life - dt * 0.38).max(0.0);
            r.life > 0.0
        });
        for cell in self.echo.iter_mut() {
            cell.life = (cell.life - dt * 0.35).max(0.0);
        }

        // ── Move streams ──────────────────────────────────────────────────────
        let n_bands    = self.bars.smoothed.len();
        let beat_boost = self.beat_flash * 1.5;
        let (ax, ay)   = self.attractor;
        // Compute effective pull BEFORE drop loop to avoid borrow conflict
        let effective_pull = if self.drain_timer > 0.0 {
            self.attractor_pull.abs() * 4.0  // great drain: always attract, strongly
        } else {
            self.attractor_pull
        };
        let scatter_active = self.scatter_timer > 0.0;
        let scatter_str    = self.scatter_timer.min(0.5) / 0.5;  // 1.0→0.0
        let drops_before   = self.drops.len();
        let mut new_drops: Vec<Drop> = Vec::new();

        for d in self.drops.iter_mut() {
            let bx     = (d.x.max(0.0) as usize * n_bands / cols.max(1)).min(n_bands.saturating_sub(1));
            let energy = self.bars.smoothed[bx];
            let dist   = d.speed * (0.35 + energy * 2.8 + beat_boost) * dt * rows as f32 * 0.7;

            d.x += d.dx * dist;
            d.y += d.dy * dist;

            // Attractor force: radial + tangential curl
            let rdx  = ax - d.x;
            let rdy  = (ay - d.y) * 2.0;
            let dist2 = rdx * rdx + rdy * rdy;
            if dist2 > 9.0 {
                let r      = dist2.sqrt();
                let dscale = [1.4f32, 1.0, 0.6][d.depth as usize];
                let pull   = effective_pull * dscale * 6.0 / r;
                d.x += rdx / r * pull * dt;
                d.y += rdy / r * pull * dt * 0.5;
                d.x += (-rdy / r) * pull.abs() * dt * 0.4;
                d.y += ( rdx / r) * pull.abs() * dt * 0.2;
            }

            // Great drain: scatter kick after drain phase
            if scatter_active {
                d.x += rng.gen_range(-scatter_str..scatter_str) * dt * cols as f32 * 0.4;
                d.y += rng.gen_range(-scatter_str..scatter_str) * dt * rows as f32 * 0.2;
            }

            // Scramble characters
            d.flip_t += dt;
            if d.flip_t > 0.08 {
                d.flip_t = 0.0;
                let idx   = rng.gen_range(0..d.seq.len());
                let chars = pool_chars(d.pool);
                d.seq[idx] = chars[rng.gen_range(0..chars.len())];
            }

            // Stuck character
            if rng.gen_bool(0.004) && self.stuck.len() < 20 {
                let tx = d.x.round() as i32;
                let ty = d.y.round() as i32;
                if tx >= 0 && tx < cols as i32 && ty >= 0 && ty < vis as i32 {
                    self.stuck.push(StuckChar {
                        row: ty as usize, col: tx as usize,
                        ch:  d.seq[rng.gen_range(0..d.seq.len())],
                        color: d.hue, life: rng.gen_range(3.0f32..9.0),
                    });
                }
            }

            // Bifurcation near attractor
            if dist2 < 64.0
                && rng.gen_bool(0.0015)
                && drops_before + new_drops.len() < cols + 25
            {
                let mut child = make_drop(rows, cols, &mut rng);
                child.x     = d.x;
                child.y     = d.y;
                child.dx    = -d.dy;    // perpendicular
                child.dy    =  d.dx;
                child.speed = d.speed * rng.gen_range(0.6f32..1.0);
                child.hue   = d.hue;
                child.depth = d.depth;
                child.pool  = d.pool;
                new_drops.push(child);
            }

            if is_offscreen(d, rows, cols) {
                *d = make_drop(rows, cols, &mut rng);
            }
        }

        self.drops.extend(new_drops);
        if self.drops.len() > cols + 25 { self.drops.truncate(cols + 25); }

        // ── Write echo ────────────────────────────────────────────────────────
        for d in &self.drops {
            for i in 0..=d.trail {
                let tx = (d.x - d.dx * i as f32).round() as i32;
                let ty = (d.y - d.dy * i as f32).round() as i32;
                if tx < 0 || ty < 0 || tx >= cols as i32 || ty >= vis as i32 { continue; }
                let idx  = ty as usize * cols + tx as usize;
                let life = if i == 0 { 0.65 } else { 0.5 * (1.0 - i as f32 / d.trail as f32) };
                if life > self.echo[idx].life {
                    self.echo[idx] = EchoCell { ch: d.seq[i % d.seq.len()], color: d.hue, life };
                }
            }
        }
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = rows.saturating_sub(1);

        // Priority: 0=empty · 1=echo/halo · 2=stuck/CA · 3=trail · 4=head · 5=corrupt/attractor/sync · 6=cosmic
        struct Cell { ch: char, color: u8, bright: f32, prio: u8 }
        let mut grid: Vec<Cell> = (0..vis * cols)
            .map(|_| Cell { ch: ' ', color: 0, bright: 0.0, prio: 0 })
            .collect();

        macro_rules! put {
            ($r:expr, $c:expr, $cell:expr) => {{
                let (r, c) = ($r as i32, $c as i32);
                if r >= 0 && r < vis as i32 && c >= 0 && c < cols as i32 {
                    let idx = r as usize * cols + c as usize;
                    if $cell.prio >= grid[idx].prio { grid[idx] = $cell; }
                }
            }};
        }

        // Layer 1: echo ghosts
        if self.echo_size == (vis, cols) {
            for r in 0..vis {
                for c in 0..cols {
                    let e = &self.echo[r * cols + c];
                    if e.life > 0.01 {
                        put!(r, c, Cell { ch: e.ch, color: e.color, bright: e.life * 0.45, prio: 1 });
                    }
                }
            }
        }

        // Layer 2a: CA patches (dim, beneath streams)
        for patch in &self.ca_patches {
            let fade = (patch.life * 0.3).min(0.28);
            for r in 0..patch.h {
                for c in 0..patch.w {
                    if patch.cells[r * patch.w + c] {
                        put!(patch.oy + r as i32, patch.ox + c as i32,
                             Cell { ch: '·', color: patch.color, bright: fade, prio: 2 });
                    }
                }
            }
        }

        // Layer 2b: stuck chars
        for s in &self.stuck {
            put!(s.row, s.col, Cell { ch: s.ch, color: s.color, bright: 0.45, prio: 2 });
        }

        // Layers 3–4: stream trails and heads (depth-modulated)
        for d in &self.drops {
            let bright_scale = [0.38f32, 1.0, 1.0][d.depth as usize];
            for i in 0..=d.trail {
                let tx = (d.x - d.dx * i as f32).round() as i32;
                let ty = (d.y - d.dy * i as f32).round() as i32;
                if tx < 0 || ty < 0 || tx >= cols as i32 || ty >= vis as i32 { continue; }
                let raw_bright = if i == 0 { 1.0 } else { (1.0 - i as f32 / d.trail as f32).max(0.0) };
                let bright = (raw_bright * bright_scale).min(1.0);
                let prio   = if i == 0 { 4u8 } else if d.depth == 0 { 2 } else { 3 };
                let color  = if i == 0 {
                    if self.beat_flash > 0.3 { self.beat_hue } else { 231 }
                } else if raw_bright > 0.55 { d.hue } else { DIM_TRAIL };
                put!(ty, tx, Cell { ch: d.seq[i % d.seq.len()], color, bright, prio });
            }
        }

        // Layer 5a: corruption bursts
        for &(r, c, ch, color, life) in &self.corruption {
            if life > 0.0 {
                put!(r, c, Cell { ch, color, bright: 1.0, prio: 5 });
            }
        }

        // ── Void zone suppression (clears all content within radius) ──────────
        if let Some(ref vz) = self.void_zone {
            let r_void = vz.current_radius();
            if r_void > 0.5 {
                for row in 0..vis {
                    for col in 0..cols {
                        let rdx = col as f32 - vz.cx;
                        let rdy = (row as f32 - vz.cy) * 2.0;
                        if (rdx * rdx + rdy * rdy).sqrt() < r_void {
                            grid[row * cols + col] = Cell { ch: ' ', color: 0, bright: 0.0, prio: 0 };
                        }
                    }
                }
            }
        }

        // Layer 5b: attractor — drawn after void suppression so it always shows
        {
            let ax = self.attractor.0.round() as i32;
            let ay = self.attractor.1.round() as i32;
            let pulse = (self.t * 3.0).sin() * 0.25 + 0.75;
            let (attr_ch, attr_color) = if self.attractor_pull >= 0.0 {
                ('⊙', 51u8)   // attracting: bright teal
            } else {
                ('⊗', 196u8)  // repelling: red
            };
            put!(ay, ax, Cell { ch: attr_ch, color: attr_color, bright: pulse, prio: 5 });
            // Faint halo at 8 compass points (aspect-ratio corrected)
            let halo_bright = 0.18 + pulse * 0.12;
            for i in 0..8 {
                let angle = i as f32 * TAU / 8.0;
                let hx = ax + (angle.cos() * 3.0).round() as i32;
                let hy = ay + (angle.sin() * 1.5).round() as i32;
                put!(hy, hx, Cell { ch: '·', color: attr_color, bright: halo_bright, prio: 1 });
            }
        }

        // ── Ripple brightening ────────────────────────────────────────────────
        const RING_W: f32 = 1.8;
        for ripple in &self.ripples {
            for r in 0..vis {
                for c in 0..cols {
                    let rdx = c as f32 - ripple.cx;
                    let rdy = (r as f32 - ripple.cy) * 2.0;
                    let dist = (rdx * rdx + rdy * rdy).sqrt();
                    let ring_dist = (dist - ripple.radius).abs();
                    if ring_dist >= RING_W { continue; }
                    let boost = (1.0 - ring_dist / RING_W) * ripple.life * 0.55;
                    let idx   = r * cols + c;
                    if grid[idx].prio > 0 {
                        grid[idx].bright = (grid[idx].bright + boost).min(1.0);
                    } else if boost > 0.18 {
                        grid[idx] = Cell { ch: '∘', color: ripple.color, bright: boost * 0.6, prio: 1 };
                    }
                }
            }
        }

        // Layer 5c: standing wave sync — pierces the void
        if let Some(ref sw) = self.sync_wave {
            if sw.row < vis {
                for c in 0..cols {
                    put!(sw.row, c, Cell { ch: sw.ch, color: 51, bright: sw.life * 0.85, prio: 5 });
                }
            }
        }

        // Layer 6: cosmic ray bit flip — overrides everything
        if let Some((cr_row, cr_col, life, cr_color)) = self.cosmic {
            if life > 0.0 && cr_row < vis && cr_col < cols {
                let idx = cr_row * cols + cr_col;
                let ch  = if grid[idx].prio > 0 { grid[idx].ch } else { '×' };
                grid[idx] = Cell { ch, color: cr_color, bright: 1.0, prio: 6 };
            }
        }

        // ── Emit ANSI lines ───────────────────────────────────────────────────
        let mut lines = Vec::with_capacity(rows);
        for r in 0..vis {
            let mut line = String::with_capacity(cols * 12);
            for c in 0..cols {
                let cell = &grid[r * cols + c];
                if cell.prio == 0 {
                    line.push(' ');
                } else if cell.bright >= 0.95 {
                    line.push_str(&format!("\x1b[1m\x1b[38;5;{}m{}\x1b[0m", cell.color, cell.ch));
                } else if cell.bright < 0.25 {
                    line.push_str(&format!("\x1b[2m\x1b[38;5;{}m{}\x1b[0m", cell.color, cell.ch));
                } else {
                    line.push_str(&format!("\x1b[38;5;{}m{}\x1b[0m", cell.color, cell.ch));
                }
            }
            lines.push(line);
        }

        lines.push(status_bar(cols, fps, self.name(), &self.source, ""));
        pad_frame(lines, rows, cols)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(MatrixViz::new(""))]
}

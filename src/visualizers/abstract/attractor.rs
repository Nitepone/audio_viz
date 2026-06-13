/// attractor.rs — Chaos in phase space: a live strange attractor.
///
/// A pool of particles flows along the trajectories of a chaotic dynamical
/// system (Lorenz by default).  Each frame every particle is advanced by a
/// few small Euler sub-steps of the system's ODEs, then rotated in 3D and
/// projected to the terminal with perspective.  Particles are splatted into a
/// decaying brightness buffer, so each leaves a fading motion-blur trail — the
/// whole structure reads as a slowly turning ribbon of luminous smoke.
///
/// Depth drives colour and brightness: points nearer the camera are brighter
/// and warmer, points further away dimmer and cooler, giving real volumetric
/// depth on a flat character grid.
///
/// Audio coupling:
///   • Bass energy bends the system's key parameter (Lorenz ρ), so the shape
///     stretches and breathes with the low end (`morph` controls how hard).
///   • Overall level (RMS) lengthens the trails on loud passages.
///   • Each detected beat releases a fresh burst of particles from the core
///     and kicks the rotation, so the structure pulses and swings to the beat.
///
/// Systems (Paul Bourke, paulbourke.net/fractals/ · Wikipedia "Lorenz system"):
///   lorenz  — the classic butterfly; ρ is the audio-bent parameter
///   aizawa  — volumetric sphere-with-spindle; `a` is bent
///   thomas  — cyclically-symmetric tangle; damping `b` is bent
///
/// Config:
///   gain         — amplifies the audio used to drive morph / trails / beats
///   system       — which attractor to render
///   palette      — depth colour gradient
///   rotate_speed — base turntable rotation rate
///   trail        — 0.5 = long trails, 2.0 = short trails
///   morph        — 0.0 = fixed shape, 1.0 = bass bends it strongly

// ── Index: System@58 · AttractorViz@83 · new@122 · ensure_grid@156 · reseed_all@168 · rng@194 · derivative@206 · sys_params@247 · impl@261 · config@265 · set_config@320 · tick@364 · render@562 · register@615
use crate::beat::{BeatDetector, BeatDetectorConfig};
use crate::visualizer::{
    merge_config,
    pad_frame, specgrad, status_bar,
    AudioFrame, TermSize, Visualizer,
};
use crate::visualizer_utils::{
    band_energy, mag_to_frac, rms, palette_lookup,
    PALETTE_ICE, PALETTE_NEON, PALETTE_FIRE, PALETTE_OCEAN,
};

const CONFIG_VERSION: u64 = 1;

const N_PARTICLES: usize = 1400;

// Camera / projection constants
const CAM_DIST: f32 = 3.2;   // distance of camera along the depth axis
const FOCAL:    f32 = 2.6;   // focal length for perspective divide
const TILT_X:   f32 = 0.45;  // fixed downward tilt (radians)

// ── Attractor systems ───────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum System {
    Lorenz,
    Aizawa,
    Thomas,
}

impl System {
    fn from_str(s: &str) -> System {
        match s {
            "aizawa" => System::Aizawa,
            "thomas" => System::Thomas,
            _        => System::Lorenz,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            System::Lorenz => "lorenz",
            System::Aizawa => "aizawa",
            System::Thomas => "thomas",
        }
    }
}

// ── Struct ────────────────────────────────────────────────────────────────────

pub struct AttractorViz {
    // ── Particle state ────────────────────────────────────────────────────
    particles: Vec<[f32; 3]>,
    speed:     Vec<f32>,    // per-particle flow speed (world units), for colour

    // ── Persistence buffers ───────────────────────────────────────────────
    bright: Vec<Vec<f32>>,  // [vis][cols] — 0.0 (dark) .. 1.0+ (bright)
    cfrac:  Vec<Vec<f32>>,  // [vis][cols] — depth colour fraction 0..1

    // ── Audio / motion state ──────────────────────────────────────────────
    rms_smooth:  f32,
    bass_smooth: f32,
    beat:        BeatDetector,
    beat_count:  u32,
    rotation:    f32,   // current turntable angle (radians)
    rot_vel:     f32,   // beat-driven angular velocity kick (radians/sec)
    flash:       f32,   // brightness boost from recent beat, decays to 0
    twist:       f32,   // smoothed height-dependent shear, driven by treble/beat
    hue:         f32,   // slowly drifting colour offset (spectrum palette)

    // ── PRNG ──────────────────────────────────────────────────────────────
    rng_state: u32,

    // ── Size cache ────────────────────────────────────────────────────────
    cached_rows: usize,
    cached_cols: usize,

    source: String,

    // ── Config fields ─────────────────────────────────────────────────────
    gain:         f32,
    system:       System,
    palette:      u8,    // 0 spectrum, 1 ice, 2 neon, 3 fire, 4 ocean
    rotate_speed: f32,
    trail:        f32,
    morph:        f32,
}

impl AttractorViz {
    pub fn new(source: &str) -> Self {
        let mut v = Self {
            particles:   Vec::new(),
            speed:       Vec::new(),
            bright:      Vec::new(),
            cfrac:       Vec::new(),
            rms_smooth:  0.0,
            bass_smooth: 0.0,
            beat: BeatDetector::new({
                let mut cfg = BeatDetectorConfig::bass_only();
                cfg.cooldown_secs = 0.18;
                cfg
            }),
            beat_count:   0,
            rotation:     0.0,
            rot_vel:      0.0,
            flash:        0.0,
            twist:        0.0,
            hue:          0.0,
            rng_state:    0x1234_5678,
            cached_rows:  0,
            cached_cols:  0,
            source:       source.to_string(),
            gain:         1.0,
            system:       System::Lorenz,
            palette:      0,
            rotate_speed: 0.5,
            trail:        1.0,
            morph:        0.6,
        };
        v.reseed_all();
        v
    }

    fn ensure_grid(&mut self, vis: usize, cols: usize) {
        if self.bright.len() == vis
            && self.bright.first().map_or(0, |r| r.len()) == cols
        {
            return;
        }
        self.bright = vec![vec![0.0f32; cols]; vis];
        self.cfrac  = vec![vec![0.0f32; cols]; vis];
    }

    /// Seed all particles in a small jittered cloud at the system's core, then
    /// warm them up so chaotic divergence spreads them across the attractor.
    fn reseed_all(&mut self) {
        let (center, _scale, h, substeps, base, _span) = Self::sys_params(self.system);
        self.particles = (0..N_PARTICLES)
            .map(|_| {
                [
                    center[0] + (self.rng() - 0.5) * 0.4,
                    center[1] + (self.rng() - 0.5) * 0.4,
                    center[2] + (self.rng() - 0.5) * 0.4,
                ]
            })
            .collect();
        self.speed = vec![0.0; N_PARTICLES];
        // Warm-up: integrate many steps so points distribute along the attractor.
        let warm = substeps * 220;
        for _ in 0..warm {
            for p in &mut self.particles {
                let d = Self::derivative(self.system, *p, base);
                p[0] += d[0] * h;
                p[1] += d[1] * h;
                p[2] += d[2] * h;
            }
        }
    }

    /// Cheap xorshift32 PRNG → f32 in [0, 1).
    #[inline]
    fn rng(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Time derivative of the chosen system at point `p`, with `param` as the
    /// audio-bent free parameter (meaning depends on the system).
    #[inline]
    fn derivative(system: System, p: [f32; 3], param: f32) -> [f32; 3] {
        let (x, y, z) = (p[0], p[1], p[2]);
        match system {
            System::Lorenz => {
                // param = ρ
                const SIGMA: f32 = 10.0;
                const BETA:  f32 = 8.0 / 3.0;
                [
                    SIGMA * (y - x),
                    x * (param - z) - y,
                    x * y - BETA * z,
                ]
            }
            System::Aizawa => {
                // param = a
                const B: f32 = 0.7;
                const C: f32 = 0.6;
                const D: f32 = 3.5;
                const E: f32 = 0.25;
                const F: f32 = 0.1;
                [
                    (z - B) * x - D * y,
                    D * x + (z - B) * y,
                    C + param * z - z * z * z / 3.0 - (x * x + y * y) * (1.0 + E * z)
                        + F * z * x * x * x,
                ]
            }
            System::Thomas => {
                // param = b (damping)
                [
                    y.sin() - param * x,
                    z.sin() - param * y,
                    x.sin() - param * z,
                ]
            }
        }
    }

    /// Per-system tuning: (center, scale, step h, sub-steps, base param, audio span).
    /// `center` is subtracted and `scale` divides to map the attractor into
    /// roughly unit-cube coordinates for projection.
    fn sys_params(system: System) -> ([f32; 3], f32, f32, usize, f32, f32) {
        match system {
            //                center            scale    h      steps base   span
            System::Lorenz => ([0.0, 0.0, 25.0], 24.0,  0.005,  8,    28.0,  18.0),
            // Aizawa collapses onto its z-axis fixed point for a ≳ 1.0, so the
            // bass morph runs a → [0.90, 1.00], which stays well clear of it.
            System::Aizawa => ([0.0, 0.0, 0.6],   1.30,  0.010,  6,    0.90,  0.10),
            System::Thomas => ([0.0, 0.0, 0.0],   3.50,  0.040,  4,    0.208, -0.060),
        }
    }
}

// ── Visualizer impl ───────────────────────────────────────────────────────────

impl Visualizer for AttractorViz {
    fn name(&self)        -> &str { "attractor" }
    fn description(&self) -> &str { "Strange attractor — audio-morphed chaos in 3D phase space" }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "attractor",
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
                    "name": "system",
                    "display_name": "System",
                    "type": "enum",
                    "value": "lorenz",
                    "variants": ["lorenz", "aizawa", "thomas"]
                },
                {
                    "name": "palette",
                    "display_name": "Palette",
                    "type": "enum",
                    "value": "spectrum",
                    "variants": ["spectrum", "ice", "neon", "fire", "ocean"]
                },
                {
                    "name": "rotate_speed",
                    "display_name": "Rotate Speed",
                    "type": "float",
                    "value": 0.5,
                    "min": 0.0,
                    "max": 2.0
                },
                {
                    "name": "trail",
                    "display_name": "Trail",
                    "type": "float",
                    "value": 1.0,
                    "min": 0.5,
                    "max": 2.0
                },
                {
                    "name": "morph",
                    "display_name": "Morph",
                    "type": "float",
                    "value": 0.6,
                    "min": 0.0,
                    "max": 1.0
                }
            ]
        }).to_string()
    }

    fn set_config(&mut self, json: &str) -> Result<String, String> {
        let merged = merge_config(&self.get_default_config(), json);
        let val: serde_json::Value = serde_json::from_str(&merged)
            .map_err(|e| format!("JSON parse error: {e}"))?;
        let prev_system = self.system;
        if let Some(config) = val["config"].as_array() {
            for entry in config {
                match entry["name"].as_str().unwrap_or("") {
                    "gain"  => self.gain  = entry["value"].as_f64().unwrap_or(1.0) as f32,
                    "trail" => self.trail = entry["value"].as_f64().unwrap_or(1.0) as f32,
                    "morph" => self.morph = entry["value"].as_f64().unwrap_or(0.6) as f32,
                    "rotate_speed" => {
                        self.rotate_speed = entry["value"].as_f64().unwrap_or(0.5) as f32;
                    }
                    "system" => {
                        self.system = System::from_str(entry["value"].as_str().unwrap_or("lorenz"));
                    }
                    "palette" => {
                        self.palette = match entry["value"].as_str().unwrap_or("spectrum") {
                            "ice"   => 1,
                            "neon"  => 2,
                            "fire"  => 3,
                            "ocean" => 4,
                            _       => 0,
                        };
                    }
                    _ => {}
                }
            }
        }
        if self.system != prev_system {
            self.reseed_all();
        }
        Ok(merged)
    }

    fn on_resize(&mut self, size: TermSize) {
        let vis  = (size.rows as usize).saturating_sub(1).max(1);
        let cols = size.cols as usize;
        self.ensure_grid(vis, cols);
        self.cached_rows = size.rows as usize;
        self.cached_cols = cols;
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: TermSize) {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = rows.saturating_sub(1).max(1);

        if rows != self.cached_rows || cols != self.cached_cols {
            self.ensure_grid(vis, cols);
            self.cached_rows = rows;
            self.cached_cols = cols;
        }

        // ── Audio analysis ────────────────────────────────────────────────
        // FFT magnitudes sit on a dB scale, so normalise perceptually with
        // mag_to_frac (gain shifts the linear level → ±dB) for a responsive
        // 0..1 range rather than a tiny linear value.
        let level = rms(&audio.mono);
        self.rms_smooth = 0.6 * self.rms_smooth + 0.4 * level;
        let level_norm = mag_to_frac(self.rms_smooth * self.gain, -48.0, -10.0);

        let bass = band_energy(&audio.fft, 30.0, 180.0);
        self.bass_smooth = 0.55 * self.bass_smooth + 0.45 * bass;
        let bass_norm = mag_to_frac(self.bass_smooth * self.gain, -60.0, -16.0);

        // Treble drives the twist (corkscrew shear) and how fast the hue drifts.
        let treble      = band_energy(&audio.fft, 2500.0, 9000.0);
        let treble_norm = mag_to_frac(treble * self.gain, -64.0, -22.0);

        self.beat.update(&audio.fft, dt);
        let beat = self.beat.is_beat();

        // Hue drifts continuously, faster with treble; the spectrum palette
        // cycles so the structure's colour is never static.
        self.hue = (self.hue + (0.04 + treble_norm * 0.35) * dt).fract();

        // ── Morph the system's free parameter with bass ─────────────────────
        let (center, scale, h, substeps, base, span) = Self::sys_params(self.system);
        let param = base + bass_norm * span * self.morph;

        // ── Rotation: base turntable, accelerated by level, plus beat kick ──
        if beat {
            let dir = (self.beat_count as f32 * 2.399_963).sin();
            let kick = 2.2 * self.beat.beat_intensity().clamp(0.5, 2.0);
            self.rot_vel += dir * kick;
            self.flash = (self.flash + 0.9 * self.beat.beat_intensity().clamp(0.5, 2.0)).min(2.0);
            self.beat_count = self.beat_count.wrapping_add(1);

            // Release a fresh burst of particles from the core — a visible pulse.
            let burst = N_PARTICLES / 5;
            for _ in 0..burst {
                let i = (self.rng() * N_PARTICLES as f32) as usize % N_PARTICLES.max(1);
                self.particles[i] = [
                    center[0] + (self.rng() - 0.5) * 0.5,
                    center[1] + (self.rng() - 0.5) * 0.5,
                    center[2] + (self.rng() - 0.5) * 0.5,
                ];
            }
        }
        self.rot_vel *= 0.93f32.powf(dt * 45.0);
        self.flash   *= 0.88f32.powf(dt * 45.0);
        // Louder passages spin the structure faster (up to ~2.5× base).
        self.rotation += (self.rotate_speed * (1.0 + level_norm * 1.5) + self.rot_vel) * dt;

        // Twist: a height-dependent rotation that shears the structure into a
        // corkscrew. Treble sustains it; each beat snaps in extra via `flash`.
        let twist_target = treble_norm * 2.4 + self.flash * 1.2;
        self.twist += (twist_target - self.twist) * (1.0 - 0.85f32.powf(dt * 45.0));

        // Bass drives the flow speed: more integration sub-steps per frame means
        // particles advance further along the attractor each tick (whipping
        // around faster when loud). We scale the *count*, not the step size `h`,
        // so the integrator stays numerically stable at every speed.
        let steps = ((substeps as f32) * (1.0 + bass_norm * 2.0)).round().max(1.0) as usize;

        // ── Advance particles along the attractor ───────────────────────────
        // Use a local PRNG state so we can mutate particles in place.
        let mut rng_state = self.rng_state;
        let mut rng = || {
            let mut x = rng_state;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            rng_state = x;
            (x >> 8) as f32 / (1u32 << 24) as f32
        };
        let system = self.system;
        for (i, p) in self.particles.iter_mut().enumerate() {
            let mut d = [0.0f32; 3];
            for _ in 0..steps {
                d = Self::derivative(system, *p, param);
                p[0] += d[0] * h;
                p[1] += d[1] * h;
                p[2] += d[2] * h;
            }
            // Cache the latest flow speed (world units) for velocity colouring.
            self.speed[i] = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            // Guard against numerical blow-up: respawn diverged particles.
            if !p[0].is_finite() || !p[1].is_finite() || !p[2].is_finite()
                || p[0].abs() > 1e4 || p[1].abs() > 1e4 || p[2].abs() > 1e4
            {
                *p = [
                    center[0] + (rng() - 0.5) * 0.4,
                    center[1] + (rng() - 0.5) * 0.4,
                    center[2] + (rng() - 0.5) * 0.4,
                ];
                self.speed[i] = 0.0;
            }
        }
        self.rng_state = rng_state;

        // ── Decay buffers for motion-blur trails ────────────────────────────
        // Louder audio → longer trails (slower decay).
        let base_decay = 0.74 + level_norm * 0.18;
        let decay = base_decay.powf(self.trail);
        for row in &mut self.bright {
            for v in row {
                *v *= decay;
            }
        }

        // ── Project & splat ─────────────────────────────────────────────────
        let cx = (cols - 1) as f32 / 2.0;
        let cy = (vis  - 1) as f32 / 2.0;
        // Bass "pumps" the zoom — the structure breathes outward on hits.
        let pulse = 1.0 + bass_norm * 0.30;
        let s  = (cols as f32 * 0.46).min(vis as f32 * 0.92) * pulse;
        let scale_x = s;
        let scale_y = s * 0.5;   // correct for ~2:1 terminal cell aspect

        let cos_p = TILT_X.cos();
        let sin_p = TILT_X.sin();

        // Overall level lifts brightness; beat flash adds a transient pop.
        let boost = 1.0 + self.flash + level_norm * 0.8;

        // Per-frame normalised-displacement factor for velocity colouring.
        let speed_k = h * steps as f32 / scale;
        let twist   = self.twist;

        for (i, p) in self.particles.iter().enumerate() {
            // Normalise into ~unit cube, mapping attractor z → screen-up.
            let nx = (p[0] - center[0]) / scale;
            let nd = (p[1] - center[1]) / scale;   // depth (toward/away)
            let nu = (p[2] - center[2]) / scale;   // up

            // Rotate around the vertical (up) axis, with a height-dependent
            // twist so the structure shears into an audio-driven corkscrew.
            let ang   = self.rotation + nu * twist;
            let cos_t = ang.cos();
            let sin_t = ang.sin();
            let rx = nx * cos_t + nd * sin_t;
            let rd = -nx * sin_t + nd * cos_t;

            // Tilt around the horizontal axis.
            let up = nu * cos_p - rd * sin_p;
            let de = nu * sin_p + rd * cos_p;

            // Perspective projection.
            let persp = FOCAL / (CAM_DIST + de);
            if persp <= 0.0 {
                continue;
            }
            let sx = cx + rx * persp * scale_x;
            let sy = cy - up * persp * scale_y;

            let xi = sx.round();
            let yi = sy.round();
            if xi < 0.0 || yi < 0.0 || xi >= cols as f32 || yi >= vis as f32 {
                continue;
            }
            let xi = xi as usize;
            let yi = yi as usize;

            let depth_frac = ((persp - 0.5) / 1.0).clamp(0.0, 1.0);

            // Velocity → colour: fast-whipping regions (e.g. the Lorenz lobes)
            // glow hot, slow regions stay cool. Bass raises the flow speed, so
            // the whole palette shifts hotter on loud passages.
            let vel = (self.speed[i] * speed_k * 3.0).tanh();

            // Colour blends velocity (dominant) and depth, then the drifting
            // hue offset cycles the spectrum palette; fixed palettes just clamp.
            let base_col = 0.12 + 0.62 * vel + 0.26 * depth_frac;
            let col_frac = if self.palette == 0 {
                (base_col + self.hue).fract()
            } else {
                base_col.clamp(0.0, 1.0)
            };

            // Depth gives volumetric shading; fast/near particles burn brightest.
            let intensity = (0.32 + 0.34 * depth_frac + 0.40 * vel) * boost;

            if self.bright[yi][xi] < intensity {
                self.bright[yi][xi] = intensity;
                self.cfrac[yi][xi]  = col_frac;
            }
        }
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = rows.saturating_sub(1).max(1);

        let mut lines = Vec::with_capacity(rows);

        for r in 0..vis {
            let mut line = String::with_capacity(cols * 14);

            let brow = if r < self.bright.len() { &self.bright[r] } else { &[] as &[f32] };
            let crow = if r < self.cfrac.len()  { &self.cfrac[r]  } else { &[] as &[f32] };

            for c in 0..cols {
                let b = if c < brow.len() { brow[c] } else { 0.0 };

                if b <= 0.06 {
                    line.push(' ');
                    continue;
                }

                let frac = if c < crow.len() { crow[c] } else { 0.5 };
                let code = match self.palette {
                    1 => palette_lookup(frac, PALETTE_ICE),
                    2 => palette_lookup(frac, PALETTE_NEON),
                    3 => palette_lookup(frac, PALETTE_FIRE),
                    4 => palette_lookup(frac, PALETTE_OCEAN),
                    _ => specgrad(frac),
                };

                let ch = if b > 0.95 { '@' }
                         else if b > 0.75 { '#' }
                         else if b > 0.55 { '*' }
                         else if b > 0.35 { '+' }
                         else if b > 0.18 { ':' }
                         else { '.' };
                let pfx = if b > 0.70 { "\x1b[1m" } else if b < 0.22 { "\x1b[2m" } else { "" };

                line.push_str(&format!("{pfx}\x1b[38;5;{code}m{ch}\x1b[0m"));
            }

            lines.push(line);
        }

        let bpm   = self.beat.estimated_bpm().round() as u32;
        let extra = format!(" | {} | {} bpm", self.system.as_str(), bpm);
        lines.push(status_bar(cols, fps, self.name(), &self.source, &extra));
        pad_frame(lines, rows, cols)
    }
}

// ── Registration ──────────────────────────────────────────────────────────────

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(AttractorViz::new(""))]
}

/// lissajous.rs — Full-terminal XY oscilloscope with beat-driven rotation.
///
/// ═══════════════════════════════════════════════════════════════════════════
///  OVERVIEW
/// ═══════════════════════════════════════════════════════════════════════════
///
/// The visualizer maps the left audio channel to the horizontal axis and the
/// right channel to the vertical axis.  Each audio sample becomes a point;
/// the persistence grid retains old points with decaying brightness, forming
/// the characteristic Lissajous figure.
///
/// The entire XY signal is rotated in signal-space by an angle that
/// accumulates based on a beat-onset detector.  On each detected beat the
/// angular velocity gets a kick; it then decays back to a slow baseline.
///
/// ═══════════════════════════════════════════════════════════════════════════
///  RENDERING LAYERS  (back → front, i.e. listed in render order)
/// ═══════════════════════════════════════════════════════════════════════════
///
///  1. Orbit reference rings
///     Three faint concentric ellipses at 25%, 52%, and 80% of the half-extents.
///     Pure geometry — cached at init/resize, never recomputed per-frame.
///
///  2. Radial spokes
///     Eight spokes rotating slowly and independently.  Length scales with
///     smoothed RMS so they pulse visibly with the music.
///
///  3. Phase-dot constellation
///     24 dots fixed in signal-space (they co-rotate with the Lissajous
///     figure), giving a "solar system" feel.
///
///  4. Dead-centre nucleus
///     A cluster of @/#/* at the very centre that grows with RMS energy.
///
///  5. Vocal stars
///     Particles spawned by vocal-range energy (300–3400 Hz).
///     Each star travels outward at a fixed screen angle, fading as it goes.
///     Bright bursts on onset transients; a gentle trickle during sustained vocals.
///
///  6. Planets
///     3–6 orbiting bodies (count scales with terminal area).
///     Each planet's orbital speed is driven by energy in a specific frequency
///     band.  Innermost = highest frequency (fastest); outermost = sub-bass
///     (slowest).  Each planet leaves a dot trail.
///
///  7. Beat ripples
///     Expanding concentric ellipses emitted from the centre on each detected
///     beat.  Replace the harsh full-frame flash of earlier implementations.
///
///  8. Spectrum shell
///     A ring of radial tick marks just outside the main figure.  Each tick
///     represents one frequency bar's energy, coloured by the spectrum gradient.
///     Sin/cos values are cached per bar and recomputed only on resize.
///
///  9. Persistence grid  (Lissajous trace itself)
///     Overlaid last so the bright trace always appears on top.
///
/// ═══════════════════════════════════════════════════════════════════════════
///  PERFORMANCE NOTES
/// ═══════════════════════════════════════════════════════════════════════════
///
/// In Rust there is no need for the numpy-vectorisation tricks used in the
/// Python version.  Simple index loops over Vec<f32> are fast enough to hold
/// 45 fps on any modern machine even at a 220-column terminal.
///
/// Geometry that doesn't change between frames (orbit rings, shell sin/cos)
/// is cached in the struct and rebuilt only on resize.

use std::collections::{HashMap, VecDeque};
use std::f32::consts::PI;

use rand::Rng;

use crate::visualizer::{
    pad_frame, specgrad, status_bar,
    AudioFrame, SpectrumBars, TermSize, Visualizer, FFT_SIZE, SAMPLE_RATE,
};

// ── Colour palettes ──────────────────────────────────────────────────────────

/// Trace colour: dark blue (oldest persistence)
const LP_DEEP: &[u8] = &[17, 18, 19, 20, 21];
/// Trace colour: cyan (mid-age persistence)
const LP_MID:  &[u8] = &[27, 33, 39, 45, 51];
/// Trace colour: white-cyan (freshest)
const _LP_BRIGHT: &[u8] = &[159, 195, 231];

/// Hue accent palette that cycles slowly with the rotation angle.
/// Used for spokes, nucleus, fresh trace cells, and beat ripples.
const LP_HUE: &[u8] = &[
    196, 202, 208, 214, 220, 226, 154, 118, 82, 46,
    51,  45,  39,  33,  27,  21,  57,  93, 129, 165, 201,
];

// ── Planet configuration ─────────────────────────────────────────────────────

/// Static per-planet configuration.
/// (band_lo_hz, band_hi_hz, orbit_radius_frac, colour_256)
///
/// Ordered innermost → outermost (highest freq → lowest freq).
/// Innermost has the fastest baseline angular velocity.
const PLANET_BANDS: &[(f32, f32, f32, u8)] = &[
    (4_000.0, 12_000.0, 0.20, 141), // hi-freq / presence — magenta
    (1_500.0,  4_000.0, 0.35,  51), // upper-mid — cyan
    (  500.0,  1_500.0, 0.50, 226), // midrange — yellow
    (  150.0,    500.0, 0.65,  82), // low-mid — green
    (   40.0,    150.0, 0.80, 196), // bass / kick — red
    (   20.0,     40.0, 0.92,  57), // sub-bass — violet  (outermost)
];

// ── Sub-structs ───────────────────────────────────────────────────────────────

/// A vocal-range particle that travels outward from the nucleus.
struct VocalStar {
    /// Screen-space angle (radians).  Fixed at spawn — does not co-rotate.
    angle:    f32,
    /// Normalised radius [0 = centre … 1 = edge].
    radius:   f32,
    /// Radial velocity (normalised units / second).
    vel_r:    f32,
    /// Remaining lifetime in seconds.
    life:     f32,
    /// Total lifetime at spawn — used to compute life_frac for fading.
    max_life: f32,
    /// 256-colour code.
    colour:   u8,
}

/// One orbiting planet.
struct Planet {
    /// Current orbital angle (radians).
    angle:    f32,
    /// Normalised orbit radius [0, 1].
    orbit_r:  f32,
    /// Lower FFT bin index for this planet's frequency band.
    lo_bin:   usize,
    /// Upper FFT bin index for this planet's frequency band.
    hi_bin:   usize,
    /// Smoothed band energy [0, 1].
    energy:   f32,
    /// 256-colour code.
    colour:   u8,
    /// Trail: VecDeque of (angle, alpha) pairs.  Newest at the front.
    trail:    VecDeque<(f32, f32)>,
}

/// One expanding beat ripple.
struct Ripple {
    /// Normalised radius [0 … 1.3+].  Expands outward each tick.
    radius:     f32,
    /// Brightness [0, 1].  Fades as the ring expands.
    brightness: f32,
}

// ── Geometry cache ────────────────────────────────────────────────────────────

/// Pre-computed (row, col, colour) tuples for the three orbit reference rings.
type RingCache = Vec<(usize, usize, u8)>;

/// Pre-computed sin/cos arrays for the spectrum shell bands.
struct ShellCache {
    sin: Vec<f32>,
    cos: Vec<f32>,
    n:   usize, // number of bars this cache was built for
}

// ── Main struct ───────────────────────────────────────────────────────────────

pub struct LissajousViz {
    // ── Shared spectrum bars (smoothed + peak) ────────────────────────────────
    bars: SpectrumBars,

    // ── Raw audio samples (copied each tick for grid plotting) ────────────────
    left:  Vec<f32>,
    right: Vec<f32>,

    // ── Persistence grid ──────────────────────────────────────────────────────
    /// brightness[row][col] ∈ [0, 1].  Decays each tick.
    brightness: Vec<Vec<f32>>,
    /// age[row][col] ∈ [0, 1].  0 = just painted, 1 = old.
    /// Drives the chromatic colour shift: fresh=accent, mid=cyan, old=deep-blue.
    age: Vec<Vec<f32>>,

    // ── Rotation (beat-driven) ────────────────────────────────────────────────
    /// Current signal-space rotation angle (radians).
    rot_angle:    f32,
    /// Current angular velocity (rad/s).  Decays toward rot_baseline.
    rot_vel:      f32,
    /// Maximum allowed |rot_vel| (rad/s).
    rot_vel_max:  f32,
    /// Idle drift velocity (rad/s) that rot_vel decays toward.
    rot_baseline: f32,

    // ── Hue animation ─────────────────────────────────────────────────────────
    /// [0, 1] fraction tied to rot_angle; drives the accent colour cycle.
    hue_t: f32,

    // ── Beat onset detector ───────────────────────────────────────────────────
    /// Slow-moving average of overall RMS, used as the onset baseline.
    beat_avg:    f32,
    /// EMA coefficient for beat_avg (small = slow average).
    beat_alpha:  f32,
    /// Ratio of current RMS to beat_avg required to trigger a beat.
    beat_thresh: f32,
    /// Minimum seconds between consecutive beats (prevents double-triggering).
    beat_min_dt: f32,
    /// Elapsed seconds since the last beat (accumulated from dt).
    time_since_beat: f32,

    // ── Beat ripples ──────────────────────────────────────────────────────────
    ripples: Vec<Ripple>,

    // ── Inner detail: spokes ──────────────────────────────────────────────────
    /// Phase angle (radians) of the 8-spoke rosette.  Advances slowly per tick.
    spoke_phase: f32,
    /// Smoothed overall RMS — drives spoke length and nucleus size.
    rms_smooth:  f32,

    // ── Inner detail: phase-dot constellation ─────────────────────────────────
    /// 24 dots fixed in signal-space: (base_angle, radius_frac).
    phase_dots: Vec<(f32, f32)>,

    // ── Vocal stars (300–3400 Hz particles) ───────────────────────────────────
    vocal_stars:  Vec<VocalStar>,
    vocal_energy: f32, // fast-smoothed vocal RMS
    vocal_avg:    f32, // very-slow average for onset detection
    /// Pre-computed FFT bin range for the vocal frequency band.
    vocal_lo_bin: usize,
    vocal_hi_bin: usize,

    // ── Planets ───────────────────────────────────────────────────────────────
    planets: Vec<Planet>,

    // ── Geometry caches (invalidated on resize) ───────────────────────────────
    ring_cache:  Option<RingCache>,
    shell_cache: Option<ShellCache>,

    // ── Terminal / display metadata ───────────────────────────────────────────
    source: String,
    cached_rows: usize,
    cached_cols: usize,
}

impl LissajousViz {
    pub fn new(source: &str) -> Self {
        let mut rng = rand::thread_rng();

        // Pre-compute vocal FFT bin range (300–3400 Hz)
        let freq_res = SAMPLE_RATE as f32 / FFT_SIZE as f32;
        let vocal_lo = (300.0  / freq_res) as usize;
        let vocal_hi = (3400.0 / freq_res) as usize;

        // Build planets from the static band table.
        // A placeholder count of 6 is used here; the actual count is adjusted
        // on the first resize() call once terminal size is known.
        let n_fft_bins = FFT_SIZE / 2 + 1;
        let planets = PLANET_BANDS.iter().map(|&(flo, fhi, orbit_r, col)| {
            let lo = ((flo / freq_res) as usize).clamp(1, n_fft_bins - 2);
            let hi = ((fhi / freq_res) as usize).clamp(2, n_fft_bins - 1);
            Planet {
                angle:   rng.gen_range(0.0..2.0 * PI),
                orbit_r,
                lo_bin:  lo,
                hi_bin:  hi.max(lo + 1),
                energy:  0.0,
                colour:  col,
                trail:   VecDeque::with_capacity(20),
            }
        }).collect();

        // 24 phase-dot positions: (angle, radius_frac)
        let phase_dots = (0..24).map(|_| {
            (rng.gen_range(0.0..2.0 * PI), rng.gen_range(0.15f32..0.42))
        }).collect();

        Self {
            bars:        SpectrumBars::new(80),
            left:        vec![0.0; FFT_SIZE],
            right:       vec![0.0; FFT_SIZE],
            brightness:  Vec::new(),
            age:         Vec::new(),
            rot_angle:   0.0,
            rot_vel:     0.02,
            rot_vel_max: 3.8,
            rot_baseline: 0.02,
            hue_t:       0.0,
            beat_avg:    0.0,
            beat_alpha:  0.15,
            beat_thresh: 1.55,
            beat_min_dt: 0.18,
            time_since_beat: 999.0,
            ripples:     Vec::new(),
            spoke_phase: 0.0,
            rms_smooth:  0.0,
            phase_dots,
            vocal_stars:  Vec::new(),
            vocal_energy: 0.0,
            vocal_avg:    0.0,
            vocal_lo_bin: vocal_lo,
            vocal_hi_bin: vocal_hi.max(vocal_lo + 1),
            planets,
            ring_cache:   None,
            shell_cache:  None,
            source:       source.to_string(),
            cached_rows:  0,
            cached_cols:  0,
        }
    }

    // ── Grid helpers ──────────────────────────────────────────────────────────

    fn ensure_grid(&mut self, vis: usize, cols: usize) {
        if self.brightness.len() != vis
            || self.brightness.first().map_or(0, |r| r.len()) != cols
        {
            self.brightness = vec![vec![0.0f32; cols]; vis];
            self.age        = vec![vec![1.0f32; cols]; vis];
        }
    }

    // ── Number of planets for a given terminal area ───────────────────────────

    fn n_planets_for(rows: usize, cols: usize) -> usize {
        let area = rows * cols;
        if      area < 2_000  { 3 }
        else if area < 6_000  { 4 }
        else if area < 12_000 { 5 }
        else                  { 6 }
    }

    // ── Accent colour from hue_t ──────────────────────────────────────────────

    fn accent(&self) -> u8 {
        let i = (self.hue_t * LP_HUE.len() as f32) as usize % LP_HUE.len();
        LP_HUE[i]
    }

    fn accent2(&self) -> u8 {
        let i = (self.hue_t * LP_HUE.len() as f32) as usize;
        LP_HUE[(i + LP_HUE.len() / 3) % LP_HUE.len()]
    }

    // ─────────────────────────────────────────────────────────────────────────
    //  TICK SUBSYSTEMS
    // ─────────────────────────────────────────────────────────────────────────

    /// Update the beat detector, kick rotation velocity, spawn ripples.
    fn tick_beat(&mut self, mono: &[f32], dt: f32) {
        // Overall RMS of the full mono signal
        let rms = (mono.iter().map(|v| v * v).sum::<f32>() / mono.len() as f32).sqrt();

        // Slow-moving average used as the onset baseline
        self.beat_avg = self.beat_alpha * rms + (1.0 - self.beat_alpha) * self.beat_avg;

        self.time_since_beat += dt;

        let is_beat = rms > self.beat_thresh * self.beat_avg
            && self.time_since_beat > self.beat_min_dt
            && rms > 0.01;

        if is_beat {
            self.time_since_beat = 0.0;

            // Kick angular velocity; direction alternates with current phase
            // to avoid always spinning the same way.
            let kick_dir = if self.rot_angle.sin() >= 0.0 { 1.0f32 } else { -1.0 };
            let kick_mag = 0.8 + rms * 4.0;
            self.rot_vel = (self.rot_vel + kick_dir * kick_mag)
                .clamp(-self.rot_vel_max, self.rot_vel_max);

            // Spawn an expanding ripple from the centre
            self.ripples.push(Ripple { radius: 0.0, brightness: 1.0 });
        }

        // Decay rot_vel toward the baseline (always in the direction of baseline)
        let sign = self.rot_vel.signum();
        let new_vel = self.rot_vel - sign * 1.8 * dt;
        self.rot_vel = if new_vel.abs() < self.rot_baseline {
            self.rot_baseline
        } else {
            new_vel
        };

        self.rot_angle = (self.rot_angle + self.rot_vel * dt).rem_euclid(2.0 * PI);
        self.hue_t     = self.rot_angle / (2.0 * PI);
        self.spoke_phase = (self.spoke_phase + dt * 0.35).rem_euclid(2.0 * PI);

        // Advance and cull ripples
        for r in &mut self.ripples {
            r.radius     += dt * 1.4; // expands ~0.7 screen-widths per second
            r.brightness -= dt * 2.2; // fades to zero in ~0.45 s
        }
        self.ripples.retain(|r| r.brightness > 0.0 && r.radius < 1.3);
    }

    /// Update smoothed overall RMS (drives spoke length and nucleus size).
    fn tick_rms(&mut self, mono: &[f32]) {
        let rms = (mono.iter().map(|v| v * v).sum::<f32>() / mono.len() as f32).sqrt();
        self.rms_smooth = 0.7 * self.rms_smooth + 0.3 * rms;
    }

    /// Update vocal-band energy and spawn stars on onset.
    ///
    /// Vocal band: 300–3400 Hz — covers essentially all human speech and singing.
    /// Two onset mechanisms:
    ///   - Burst: a sudden jump in energy spawns 1–6 stars at once.
    ///   - Trickle: while vocals are present, one star per frame with probability
    ///     proportional to vocal energy.
    fn tick_vocal_stars(&mut self, fft: &[f32], dt: f32) {
        let mut rng = rand::thread_rng();

        // Compute RMS of the vocal band
        let lo = self.vocal_lo_bin;
        let hi = self.vocal_hi_bin.min(fft.len());
        let v_rms = if hi > lo {
            let slice = &fft[lo..hi];
            (slice.iter().map(|v| v * v).sum::<f32>() / slice.len() as f32).sqrt() * 60.0
        } else {
            0.0
        };

        // Fast-attack / slow-decay smoothing
        let a_v = if v_rms > self.vocal_energy { 0.55 } else { 0.20 };
        self.vocal_energy = a_v * v_rms + (1.0 - a_v) * self.vocal_energy;

        // Very slow background average (τ ≈ 50 frames)
        self.vocal_avg = 0.02 * self.vocal_energy + 0.98 * self.vocal_avg;

        // Onset ratio: how much louder than the background is the current energy?
        let onset_ratio = self.vocal_energy / self.vocal_avg.max(1e-6);
        let is_onset = onset_ratio > 1.35 && self.vocal_energy > 0.04;

        // Burst: spawn multiple stars proportional to onset strength
        if is_onset {
            let n_new = ((1.0 + (onset_ratio - 1.35) * 10.0).min(6.0)) as usize;
            let warm: &[u8] = &[231, 230, 229, 228, 227, 226, 220, 214];
            for _ in 0..n_new {
                self.vocal_stars.push(VocalStar {
                    angle:    rng.gen_range(0.0..2.0 * PI),
                    radius:   0.02,
                    vel_r:    0.18 + self.vocal_energy * 0.55 + rng.gen_range(0.0..0.12),
                    life:     0.6 + rng.gen_range(0.0..0.5),
                    max_life: 0.6 + 0.5, // conservative max
                    colour:   warm[rng.gen_range(0..warm.len())],
                });
            }
        }

        // Trickle: one star at random while vocals are active
        if self.vocal_energy > 0.06 && rng.r#gen::<f32>() < self.vocal_energy * 0.4 {
            let cool: &[u8] = &[195, 159, 231, 230, 229];
            self.vocal_stars.push(VocalStar {
                angle:    rng.gen_range(0.0..2.0 * PI),
                radius:   0.01,
                vel_r:    0.10 + self.vocal_energy * 0.30,
                life:     0.4 + rng.gen_range(0.0..0.3),
                max_life: 0.7,
                colour:   cool[rng.gen_range(0..cool.len())],
            });
        }

        // Integrate star positions and cull dead ones
        for s in &mut self.vocal_stars {
            s.radius += s.vel_r * dt;
            s.life   -= dt;
        }
        self.vocal_stars.retain(|s| s.life > 0.0 && s.radius < 1.05);
    }

    /// Update planet energies and advance orbital angles.
    ///
    /// Each planet's angular velocity = baseline (falls with orbit radius) +
    /// audio-driven kick (rises with band energy).
    ///
    /// Baseline at orbit_r = 0.20:  ~0.55 rad/s (~11.4 s/orbit)
    /// Baseline at orbit_r = 0.92:  ~0.06 rad/s (~105  s/orbit)
    /// Maximum audio kick:          +1.8 rad/s above baseline
    fn tick_planets(&mut self, fft: &[f32], dt: f32, n_visible: usize) {
        // Truncate to the number of planets appropriate for this terminal size
        self.planets.truncate(n_visible);

        for p in &mut self.planets {
            // Band RMS
            let lo = p.lo_bin;
            let hi = p.hi_bin.min(fft.len());
            let raw_e = if hi > lo {
                let slice = &fft[lo..hi];
                (slice.iter().map(|v| v * v).sum::<f32>() / slice.len() as f32).sqrt() * 80.0
            } else { 0.0 };
            let raw_e = raw_e.min(1.0);

            // Fast-attack / slow-decay energy smoothing
            let a_p = if raw_e > p.energy { 0.50 } else { 0.15 };
            p.energy = a_p * raw_e + (1.0 - a_p) * p.energy;

            // Angular velocity
            let baseline = 0.55 * (1.0 - p.orbit_r) + 0.06;
            let omega    = baseline + p.energy * 1.8;
            let old_angle = p.angle;
            p.angle = (p.angle + omega * dt).rem_euclid(2.0 * PI);

            // Record trail entry with fresh alpha
            p.trail.push_front((old_angle, 1.0));
            if p.trail.len() > 18 { p.trail.pop_back(); }

            // Decay all trail alphas
            for (_, alpha) in &mut p.trail {
                *alpha *= 0.82;
            }
            // Prune fully-faded entries from the back
            while p.trail.back().map_or(false, |&(_, a)| a < 0.05) {
                p.trail.pop_back();
            }
        }
    }

    /// Plot audio samples into the persistence grid.
    ///
    /// The L/R signal is rotated by rot_angle before mapping to screen
    /// coordinates, so the entire Lissajous figure spins with the beat.
    ///
    /// 4-neighbour anti-aliasing: each plotted point also stamps slightly
    /// dimmer values on its cardinal neighbours to smooth jagged diagonals.
    fn tick_grid(&mut self, vis: usize, cols: usize, dt: f32) {
        let cx = (cols - 1) as f32 / 2.0;
        let cy = (vis  - 1) as f32 / 2.0;

        let ca = self.rot_angle.cos();
        let sa = self.rot_angle.sin();

        // Scale to 96% of the half-extents.
        // The 0.5 factor on half_y compensates for the ~2:1 character aspect
        // ratio so the figure appears round rather than squashed.
        let half_x = cx * 0.96;
        let half_y = cy * 0.96;

        // Decay brightness (louder → faster decay for more dramatic contrast)
        let decay = (0.84 - self.rms_smooth * 0.12).clamp(0.72, 0.92);
        for row in &mut self.brightness {
            for v in row { *v *= decay; }
        }
        // Age all cells toward 1.0 (old)
        for row in &mut self.age {
            for v in row { *v = (*v + dt * 0.9).min(1.0); }
        }

        // Plot each sample as a (xi, yi) grid cell
        for i in 0..self.left.len().min(FFT_SIZE) {
            let lv = self.left [i];
            let rv = self.right[i];

            // Rotate in signal space
            let xr =  ca * lv + sa * rv;
            let yr = -sa * lv + ca * rv;

            // Map to grid coordinates
            let xi = (xr  * half_x + cx).round().clamp(0.0, (cols - 1) as f32) as usize;
            // Invert Y: positive amplitude → higher on screen (lower row index)
            let yi = (-yr * half_y + cy).round().clamp(0.0, (vis  - 1) as f32) as usize;

            // Stamp main cell
            self.brightness[yi][xi] = 1.0;
            self.age       [yi][xi] = 0.0;

            // 4-neighbour anti-alias
            const NEIGHBOURS: &[(isize, isize, f32)] = &[
                (-1, 0, 0.55), (1, 0, 0.55), (0, -1, 0.45), (0, 1, 0.45),
            ];
            for &(dr, dc, w) in NEIGHBOURS {
                let ny = (yi as isize + dr).clamp(0, vis  as isize - 1) as usize;
                let nx = (xi as isize + dc).clamp(0, cols as isize - 1) as usize;
                if self.brightness[ny][nx] < w {
                    self.brightness[ny][nx] = w;
                    self.age       [ny][nx] = 0.1;
                }
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    //  RENDER HELPERS
    // ─────────────────────────────────────────────────────────────────────────

    /// Return the ANSI escape for a 256-colour foreground.
    #[inline(always)]
    fn fg(code: u8) -> String {
        format!("\x1b[38;5;{code}m")
    }

    /// Polar → screen coordinates, corrected for character aspect ratio.
    ///
    /// `angle` is measured from the positive-X axis.
    /// `r_frac` is the normalised radius [0, 1].
    /// Returns `(row, col)` clamped to the visible area.
    fn polar_to_screen(
        angle: f32, r_frac: f32,
        rx_full: f32, ry_full: f32,
        icx: usize, icy: usize,
        vis: usize, cols: usize,
    ) -> Option<(usize, usize)> {
        let xd =  angle.cos() * r_frac;
        let yd = -angle.sin() * r_frac;
        let rc = (icy as f32 + yd * ry_full).round() as isize;
        let cc = (icx as f32 + xd * rx_full).round() as isize;
        if rc >= 0 && rc < vis as isize && cc >= 0 && cc < cols as isize {
            Some((rc as usize, cc as usize))
        } else {
            None
        }
    }

    /// Build the sparse detail overlay for one frame.
    ///
    /// Returns a HashMap from (row, col) to (char, colour_256, bold).
    /// Layers are written in back-to-front order so later layers overwrite
    /// earlier ones.  The Lissajous trace grid is composited on top of this
    /// overlay in the final render pass.
    fn build_detail(
        &self,
        vis: usize, cols: usize,
        icx: usize, icy: usize,
        rx_full: f32, ry_full: f32,
        accent: u8, accent2: u8,
    ) -> HashMap<(usize, usize), (char, u8, bool)> {
        let mut detail: HashMap<(usize, usize), (char, u8, bool)> =
            HashMap::with_capacity(1024);

        // ── Layer 1: Orbit reference rings ────────────────────────────────────
        // Cache was populated in tick(); just read here.
        if let Some(cache) = &self.ring_cache {
            for &(r, c, ring_col) in cache {
                detail.entry((r, c)).or_insert(('.', ring_col, false));
            }
        }

        // ── Layer 2: Radial spokes ────────────────────────────────────────────
        // 8 spokes that rotate slowly and vary in length with RMS energy.
        // Spoke length: 10%..60% of the half-extent.
        let spoke_len = (0.10 + self.rms_smooth * 0.50).min(1.0);
        const N_SPOKES: usize = 8;
        const N_STEPS: usize  = 18;
        for si in 0..N_SPOKES {
            let a       = self.spoke_phase + si as f32 * (2.0 * PI / N_SPOKES as f32);
            let sin_a   = a.sin();
            let cos_a   = a.cos();
            let abs_sin = sin_a.abs();
            let ch_base = if abs_sin > 0.7 { '|' } else { '-' };

            for step in 0..N_STEPS {
                let frac   = 0.03 + (spoke_len - 0.03) * step as f32 / (N_STEPS - 1) as f32;
                let rc = (icy as f32 - sin_a * ry_full * frac).round() as isize;
                let cc = (icx as f32 + cos_a * rx_full * frac).round() as isize;
                if rc < 0 || rc >= vis as isize || cc < 0 || cc >= cols as isize { continue; }
                let rc = rc as usize;  let cc = cc as usize;

                let bright = 1.0 - frac / spoke_len;
                let ch  = if frac < 0.06 { '+' } else { ch_base };
                let col = if bright > 0.7 { accent } else if bright > 0.4 { accent2 } else { 238 };
                detail.insert((rc, cc), (ch, col, bright > 0.6));
            }
        }

        // ── Layer 3: Phase-dot constellation ─────────────────────────────────
        // 24 dots co-rotating with the Lissajous figure (angle + rot_angle).
        // Their radii expand slightly with RMS so they pulse with the music.
        let rms = self.rms_smooth;
        for &(base_a, r_frac) in &self.phase_dots {
            let a    = base_a + self.rot_angle;
            let rdot = r_frac * (0.6 + rms * 0.9);
            if let Some((rc, cc)) = Self::polar_to_screen(
                a, rdot, rx_full, ry_full, icx, icy, vis, cols,
            ) {
                let col = if r_frac < 0.28 { accent } else { accent2 };
                detail.insert((rc, cc), ('*', col, true));
            }
        }

        // ── Layer 4: Dead-centre nucleus ──────────────────────────────────────
        // A 0–3 cell radius cluster at the very centre that pulses with RMS.
        let nuc_r = (self.rms_smooth * 3.5).round() as isize;
        for dr in -nuc_r..=nuc_r {
            for dc in -nuc_r..=nuc_r {
                let dist = ((dr * dr) as f32 + (dc as f32 * 0.5).powi(2)).sqrt();
                if dist <= nuc_r as f32 + 0.5 {
                    let rc = (icy as isize + dr).clamp(0, vis  as isize - 1) as usize;
                    let cc = (icx as isize + dc).clamp(0, cols as isize - 1) as usize;
                    let ch = if dist < 0.8 { '@' } else if dist < 1.5 { '#' } else { '*' };
                    detail.insert((rc, cc), (ch, accent, true));
                }
            }
        }

        // ── Layer 5: Vocal stars ──────────────────────────────────────────────
        // Outward-travelling particles spawned by vocal-range energy onsets.
        // Fixed screen angle (not co-rotating) so they radiate outward.
        for s in &self.vocal_stars {
            let life_frac = s.life / s.max_life.max(1e-6);
            if let Some((rc, cc)) = Self::polar_to_screen(
                s.angle, s.radius, rx_full, ry_full, icx, icy, vis, cols,
            ) {
                let ch  = if life_frac > 0.65 { '*' } else if life_frac > 0.30 { '+' } else { '.' };
                let bold = life_frac > 0.50;
                detail.insert((rc, cc), (ch, s.colour, bold));
            }
            // Short trail one step behind the star
            let trail_r = (s.radius - s.vel_r * 0.04).max(0.0);
            if life_frac > 0.40 {
                if let Some((rc2, cc2)) = Self::polar_to_screen(
                    s.angle, trail_r, rx_full, ry_full, icx, icy, vis, cols,
                ) {
                    detail.entry((rc2, cc2)).or_insert(('.', s.colour, false));
                }
            }
        }

        // ── Layer 6: Planets ──────────────────────────────────────────────────
        // Orbiting 'o' characters with fading dot trails.
        // Trail dots only overwrite other dots (not stars or spokes).
        for p in &self.planets {
            // Trail (draw first so planet head overwrites trail tip)
            for &(t_angle, t_alpha) in &p.trail {
                if let Some((rc, cc)) = Self::polar_to_screen(
                    t_angle, p.orbit_r, rx_full, ry_full, icx, icy, vis, cols,
                ) {
                    let trail_col = if t_alpha > 0.65 {
                        p.colour
                    } else if t_alpha > 0.35 {
                        240
                    } else {
                        236
                    };
                    // Only overwrite if current cell is a dot or empty
                    let existing = detail.get(&(rc, cc));
                    if existing.is_none() || existing.map_or(false, |e| e.0 == '.') {
                        detail.insert((rc, cc), ('.', trail_col, false));
                    }
                }
            }
            // Planet head
            if let Some((rc, cc)) = Self::polar_to_screen(
                p.angle, p.orbit_r, rx_full, ry_full, icx, icy, vis, cols,
            ) {
                detail.insert((rc, cc), ('o', p.colour, true));
            }
        }

        // ── Layer 7: Beat ripples ─────────────────────────────────────────────
        // Expanding ellipses from the centre on each detected beat.
        // Character and colour degrade as brightness drops:
        //   > 0.70  → 'o' bright accent
        //   0.35–0.70 → '+' accent2
        //   < 0.35  → '.' dim cyan
        for rp in &self.ripples {
            if rp.brightness <= 0.0 || rp.radius <= 0.0 { continue; }

            let (rp_ch, rp_col, rp_bold) = if rp.brightness > 0.70 {
                ('o', accent, true)
            } else if rp.brightness > 0.35 {
                ('+', accent2, false)
            } else {
                let i = (rp.brightness * LP_MID.len() as f32) as usize;
                ('.', LP_MID[i.min(LP_MID.len() - 1)], false)
            };

            let rx_rp = rx_full * rp.radius;
            let ry_rp = ry_full * rp.radius;
            let steps = ((rx_rp + ry_rp) * 3.0).max(48.0) as usize;

            for i in 0..steps {
                let a  = 2.0 * PI * i as f32 / steps as f32;
                let rc = (icy as f32 - a.sin() * ry_rp).round() as isize;
                let cc = (icx as f32 + a.cos() * rx_rp).round() as isize;
                if rc < 0 || rc >= vis as isize || cc < 0 || cc >= cols as isize { continue; }
                let key = (rc as usize, cc as usize);
                let existing = detail.get(&key);
                // Ripples overwrite background dots/lines but not planets/stars
                if existing.is_none() || existing.map_or(false, |e| {
                    matches!(e.0, '.' | '-' | '|')
                }) {
                    detail.insert(key, (rp_ch, rp_col, rp_bold));
                }
            }
        }

        // ── Layer 8: Spectrum shell ───────────────────────────────────────────
        // Radial tick marks just outside the main figure (shell_r = 94%).
        // Sin/cos per band are cached; only energy varies each frame.
        // Shell sin/cos cache was populated in tick(); just read here.
        let n_spec = self.bars.smoothed.len();
        let shell = match &self.shell_cache {
            Some(sc) if sc.n == n_spec => sc,
            _ => return detail, // cache not ready yet (first frame edge case)
        };
        let shell_r_base = 0.94f32;
        const SHELL_STEPS: usize = 5;

        for si in 0..n_spec {
            let e = self.bars.smoothed[si];
            if e < 0.01 { continue; } // skip silent bands

            let frac = si as f32 / (n_spec - 1).max(1) as f32;
            let code = specgrad(frac);
            let bold = e > 0.6;
            let t_len = e * 0.10;

            for step in 0..SHELL_STEPS {
                let df   = step as f32 / (SHELL_STEPS - 1) as f32;
                let frac_r = shell_r_base + df * t_len;
                let rc = (icy as f32 - shell.sin[si] * ry_full * frac_r).round() as isize;
                let cc = (icx as f32 + shell.cos[si] * rx_full * frac_r).round() as isize;
                if rc >= 0 && rc < vis as isize && cc >= 0 && cc < cols as isize {
                    detail.insert((rc as usize, cc as usize), ('|', code, bold));
                }
            }
        }

        detail
    }
}

impl Visualizer for LissajousViz {
    fn name(&self)        -> &str { "lissajous" }
    fn description(&self) -> &str { "Full-terminal XY scope — beat rotation, planets, vocal stars, ripples" }

    fn on_resize(&mut self, size: TermSize) {
        self.bars.resize(size.cols as usize);
        self.ring_cache  = None;
        self.shell_cache = None;
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: TermSize) {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = rows.saturating_sub(1).max(1);

        // Resize dependent state when terminal dimensions change
        if rows != self.cached_rows || cols != self.cached_cols {
            self.bars.resize(cols);
            self.ring_cache  = None;
            self.shell_cache = None;
            self.cached_rows = rows;
            self.cached_cols = cols;
        }

        // Update the shared spectrum bars (used by spokes, shell, planets)
        self.bars.update(&audio.fft, dt);

        // Copy raw samples for grid plotting
        self.left .clone_from(&audio.left);
        self.right.clone_from(&audio.right);

        // Ensure the persistence grid matches the current terminal size
        self.ensure_grid(vis, cols);

        // ── Run all tick subsystems ───────────────────────────────────────────
        self.tick_beat(&audio.mono, dt);
        self.tick_rms (&audio.mono);
        self.tick_vocal_stars(&audio.fft, dt);

        let n_vis = Self::n_planets_for(rows, cols);
        self.tick_planets(&audio.fft, dt, n_vis);

        // ── Warm geometry caches ───────────────────────────────────────────────
        // ring_cache and shell_cache are computed here in tick() (where &mut
        // self is valid) so that render() / build_detail() can use &self only.
        {
            let cx      = (cols - 1) as f32 / 2.0;
            let cy      = (vis  - 1) as f32 / 2.0;
            let rx_full = cx * 0.96;
            let ry_full = cy * 0.96;
            let icx     = cols / 2;
            let icy     = vis  / 2;

            if self.ring_cache.is_none() {
                let mut cache = Vec::new();
                for &(frac, ring_col) in &[(0.25f32, 235u8), (0.52, 236), (0.80, 237)] {
                    let rx    = rx_full * frac;
                    let ry    = ry_full * frac;
                    let steps = ((rx + ry) * 2.5).max(64.0) as usize;
                    for i in 0..steps {
                        let a  = 2.0 * PI * i as f32 / steps as f32;
                        let rc = (icy as f32 - a.sin() * ry).round() as isize;
                        let cc = (icx as f32 + a.cos() * rx).round() as isize;
                        if rc >= 0 && rc < vis as isize && cc >= 0 && cc < cols as isize {
                            cache.push((rc as usize, cc as usize, ring_col));
                        }
                    }
                }
                self.ring_cache = Some(cache);
            }

            let n_spec = self.bars.smoothed.len();
            let rebuild = self.shell_cache.as_ref().map_or(true, |sc| sc.n != n_spec);
            if rebuild {
                let sin_vals: Vec<f32> = (0..n_spec).map(|i| {
                    (i as f32 * 2.0 * PI / n_spec as f32 - PI / 2.0).sin()
                }).collect();
                let cos_vals: Vec<f32> = (0..n_spec).map(|i| {
                    (i as f32 * 2.0 * PI / n_spec as f32 - PI / 2.0).cos()
                }).collect();
                self.shell_cache = Some(ShellCache { sin: sin_vals, cos: cos_vals, n: n_spec });
            }
        }

        self.tick_grid(vis, cols, dt);
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = rows.saturating_sub(1).max(1);

        let icx = cols / 2;
        let icy = vis  / 2;
        let cx  = (cols - 1) as f32 / 2.0;
        let cy  = (vis  - 1) as f32 / 2.0;

        // Full half-extents at 96% fill
        let rx_full = cx * 0.96;
        let ry_full = cy * 0.96;

        let accent  = self.accent();
        let accent2 = self.accent2();

        // ── Build the sparse detail overlay ───────────────────────────────────
        // Geometry caches were warmed in tick() so build_detail() only needs &self.
        let detail = self.build_detail(vis, cols, icx, icy, rx_full, ry_full, accent, accent2);

        // Group detail by row for O(detail/row) lookup instead of O(total)
        let mut detail_by_row: HashMap<usize, Vec<(usize, char, u8, bool)>> =
            HashMap::new();
        for (&(r, c), &(ch, col, bold)) in &detail {
            if r < vis {
                detail_by_row.entry(r).or_default().push((c, ch, col, bold));
            }
        }

        // ── Compose each row ──────────────────────────────────────────────────
        let mut lines = Vec::with_capacity(rows);
        let n_mid  = LP_MID.len();
        let n_deep = LP_DEEP.len();

        for r in 0..vis {
            let mut row_chars: Vec<Option<String>> = vec![None; cols];

            // 1. Paint the Lissajous persistence grid cells
            let brow: &[f32] = if r < self.brightness.len() { &self.brightness[r] } else { &[] };
            let arow: &[f32] = if r < self.age.len()        { &self.age[r]        } else { &[] };

            for c in 0..cols {
                let b = if c < brow.len() { brow[c] } else { 0.0 };
                if b <= 0.06 { continue; } // not active — leave for detail or space

                let a_val = if c < arow.len() { arow[c] } else { 1.0 };

                // Chromatic colour: fresh → accent hue, mid → cyan, old → deep blue
                let code = if a_val < 0.15 {
                    accent
                } else if a_val < 0.45 {
                    let i = (a_val * n_mid as f32) as usize;
                    LP_MID[i.min(n_mid - 1)]
                } else {
                    let i = (a_val * n_deep as f32) as usize;
                    LP_DEEP[i.min(n_deep - 1)]
                };

                // Character density by brightness
                let ch = if b > 0.88 { '@' } else if b > 0.65 { '#' }
                         else if b > 0.40 { '*' } else if b > 0.20 { '+' }
                         else { '.' };

                let bold = if b > 0.70 { "\x1b[1m" } else { "" };
                row_chars[c] = Some(format!("{bold}\x1b[38;5;{code}m{ch}\x1b[0m"));
            }

            // 2. Overwrite inactive cells with detail-overlay entries
            if let Some(entries) = detail_by_row.get(&r) {
                for &(c, ch, col, bold) in entries {
                    if c < cols && row_chars[c].is_none() {
                        let pfx = if bold { "\x1b[1m" } else { "\x1b[2m" };
                        row_chars[c] = Some(format!("{pfx}{}{ch}\x1b[0m", Self::fg(col)));
                    }
                }
            }

            // 3. Fill remaining cells with spaces
            let line: String = row_chars.into_iter()
                .map(|cell| cell.unwrap_or_else(|| " ".to_string()))
                .collect();
            lines.push(line);
        }

        // ── Status bar ────────────────────────────────────────────────────────
        let vel_deg = self.rot_vel * 180.0 / PI;
        let ang_deg = (self.rot_angle * 180.0 / PI) as u32 % 360;
        let beat_ind = if !self.ripples.is_empty() {
            format!("{}\x1b[1m●\x1b[0m", Self::fg(accent))
        } else {
            " ".to_string()
        };
        let extra = format!(" | {beat_ind} {ang_deg:3}° {vel_deg:+.1}°/s");
        lines.push(status_bar(cols, fps, self.name(), &self.source, &extra));

        pad_frame(lines, rows, cols)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(LissajousViz::new(""))]
}

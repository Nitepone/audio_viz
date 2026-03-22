/// matrix.rs — Audio-reactive falling character rain (Matrix effect).
///
/// One column of falling characters per terminal column.  Each column's
/// fall speed is driven by the energy in its corresponding frequency band,
/// so the rain moves faster when the music is louder in that band.
///
/// Head character: bright white (231).
/// Trail fades through green shades: 46 → 40 → 34 → 28 → 22 → 238.
///
/// Characters in the trail are randomly mutated every ~80 ms to give the
/// "glitching" feel of the original effect.

use rand::Rng;
use crate::visualizer::{
    pad_frame, status_bar,
    AudioFrame, SpectrumBars, TermSize, Visualizer,
};

// Characters that can appear in a falling column
const MCHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz\
                         0123456789!@#$%^&*()_+-=[]{}|;:.,<>?/~`\\";

// Trail colour progression from fresh (top) to old (bottom)
const GREEN: &[u8] = &[46, 40, 34, 28, 22, 238];

// Possible hue accents for column heads
const HUES: &[u8] = &[46, 40, 82, 118, 51, 45];

struct Drop {
    /// Floating-point row of the head character (can be fractional)
    y:       f32,
    /// Fall speed in rows/second (before audio modulation)
    speed:   f32,
    /// Length of the visible trail in characters
    trail:   usize,
    /// Rotating character sequence for this column
    seq:     Vec<u8>,
    /// Seconds since last character scramble
    flip_t:  f32,
    /// Accent colour for the bright trail region
    hue:     u8,
}

impl Drop {
    fn new(rows: usize, rng: &mut impl Rng) -> Self {
        Self {
            y:      rng.gen_range(-(rows as f32)..0.0),
            speed:  rng.gen_range(0.4f32..1.3),
            trail:  rng.gen_range(5usize..18),
            seq:    (0..24).map(|_| MCHARS[rng.gen_range(0..MCHARS.len())]).collect(),
            flip_t: 0.0,
            hue:    HUES[rng.gen_range(0..HUES.len())],
        }
    }
}

pub struct MatrixViz {
    drops:  Vec<Drop>,
    bars:   SpectrumBars,
    source: String,
}

impl MatrixViz {
    pub fn new(source: &str) -> Self {
        Self {
            drops:  Vec::new(),
            bars:   SpectrumBars::new(80),
            source: source.to_string(),
        }
    }

    /// Ensure we have exactly `cols` drops, creating or trimming as needed.
    fn sync_drops(&mut self, rows: usize, cols: usize) {
        let mut rng = rand::thread_rng();
        while self.drops.len() < cols {
            self.drops.push(Drop::new(rows, &mut rng));
        }
        self.drops.truncate(cols);
    }
}

impl Visualizer for MatrixViz {
    fn name(&self)        -> &str { "matrix" }
    fn description(&self) -> &str { "Audio-reactive falling character rain" }

    fn on_resize(&mut self, size: TermSize) {
        self.bars.resize(size.cols as usize);
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: TermSize) {
        let rows = size.rows as usize;
        let cols = size.cols as usize;

        self.bars.resize(cols);
        self.bars.update(&audio.fft, dt);
        self.sync_drops(rows, cols);

        let n = self.bars.smoothed.len();
        let mut rng = rand::thread_rng();

        for (ci, d) in self.drops.iter_mut().enumerate() {
            // Map column to a frequency band
            let band   = (ci * n / cols.max(1)).min(n.saturating_sub(1));
            let energy = self.bars.smoothed[band];

            // Advance head position
            d.y    += d.speed * (0.35 + energy * 2.8) * dt * rows as f32 * 0.7;
            d.flip_t += dt;

            // Randomly mutate one character in the trail
            if d.flip_t > 0.08 {
                d.flip_t = 0.0;
                let idx = rng.gen_range(0..d.seq.len());
                d.seq[idx] = MCHARS[rng.gen_range(0..MCHARS.len())];
            }

            // Respawn when the entire trail has scrolled off the bottom
            if d.y - d.trail as f32 > rows as f32 {
                *d = Drop::new(rows, &mut rng);
            }
        }
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let vis  = rows.saturating_sub(1); // reserve one row for status bar

        // Sparse grid: (row, col) → (char_byte, brightness 0..1, hue)
        // brightness 1.0 = head, < 1.0 = trail
        let mut grid: std::collections::HashMap<(usize, usize), (u8, f32, u8)> =
            std::collections::HashMap::new();

        for (ci, d) in self.drops.iter().enumerate() {
            if ci >= cols { break; }
            let trl = d.trail;
            for pos in 0..=trl {
                let r = d.y as isize - pos as isize;
                if r >= 0 && (r as usize) < vis {
                    let bright = if pos == 0 {
                        1.0f32
                    } else {
                        (1.0 - pos as f32 / trl as f32).max(0.0)
                    };
                    let ch = d.seq[pos % d.seq.len()];
                    grid.insert((r as usize, ci), (ch, bright, d.hue));
                }
            }
        }

        let mut lines = Vec::with_capacity(rows);
        for r in 0..vis {
            let mut line = String::with_capacity(cols * 12);
            for c in 0..cols {
                if let Some(&(ch, bright, hue)) = grid.get(&(r, c)) {
                    let ch_char = ch as char;
                    if bright >= 0.95 {
                        // Head: bright white flash
                        line.push_str(&format!("\x1b[1m\x1b[38;5;231m{ch_char}\x1b[0m"));
                    } else {
                        // Trail: fade through green shades
                        let shade = if bright > 0.5 {
                            hue
                        } else {
                            let si = ((1.0 - bright) * (GREEN.len() - 1) as f32) as usize;
                            GREEN[si.min(GREEN.len() - 1)]
                        };
                        line.push_str(&format!("\x1b[38;5;{shade}m{ch_char}\x1b[0m"));
                    }
                } else {
                    line.push(' ');
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

/// vu.rs — Stereo VU meter visualizer.
///
/// This file is intentionally kept as simple as possible to serve as a
/// reference implementation for developers adding new visualizers.
///
/// ═══════════════════════════════════════════════════════════════════
///  HOW TO ADD A NEW VISUALIZER
/// ═══════════════════════════════════════════════════════════════════
///
///  1. Create src/visualizers/yourname.rs (this file is your template).
///
///  2. Implement the `Visualizer` trait:
///       - `name()`        short lowercase string, used on the CLI
///       - `description()` one line shown in --list
///       - `tick()`        called every frame with fresh audio + dt seconds
///       - `render()`      return exactly `size.rows` strings, each exactly
///                         `size.cols` display-columns wide
///       - `on_resize()`   optional; invalidate any size-dependent caches
///
///  3. Export `pub fn register() -> Vec<Box<dyn Visualizer>>` returning
///     one entry per visualizer defined in this file.
///
///  4. Run `cargo build` — build.rs scans src/visualizers/*.rs automatically
///     and adds your visualizer to the registry.  No other files need editing.
///
/// ═══════════════════════════════════════════════════════════════════
///  USEFUL HELPERS FROM visualizer.rs
/// ═══════════════════════════════════════════════════════════════════
///
///  AudioFrame fields:
///    audio.left   Vec<f32>  left channel samples  [-1, 1], len = FFT_SIZE
///    audio.right  Vec<f32>  right channel samples [-1, 1], len = FFT_SIZE
///    audio.mono   Vec<f32>  (left + right) * 0.5, len = FFT_SIZE
///    audio.fft    Vec<f32>  rfft magnitude spectrum, len = FFT_SIZE/2 + 1
///
///  Shared palette:
///    specgrad(frac: f32) -> u8   256-colour code, red→green→blue over [0,1]
///
///  Frame helpers:
///    pad_frame(lines, rows, cols)   pad/truncate to exact dimensions
///    status_bar(cols, fps, name, source, extra)   standard bottom row
///    hline(cols, color)             dim horizontal rule
///    title_line(cols, text, color)  centred bold title
///
///  SpectrumBars   maintains smoothed bar heights + peak markers for you;
///                 call bars.update(&audio.fft, dt) each tick.

use crate::visualizer::{
    pad_frame, status_bar,
    AudioFrame, TermSize, Visualizer,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// How quickly the meter rises toward a louder signal (0 = instant, 1 = frozen).
const RISE: f32 = 0.30;
/// How quickly the meter falls back toward silence.
const FALL: f32 = 0.85;

/// How long the peak marker stays at its maximum before starting to drop (seconds).
const PEAK_HOLD: f32 = 1.5;
/// How fast the peak marker falls after the hold expires, per second.
const PEAK_FALL: f32 = 0.40;

// ── Colour ramp ───────────────────────────────────────────────────────────────
//
// Maps a normalised level [0, 1] to a 256-colour code.
// Green for low levels, yellow for mid, red for high — the classic VU look.

fn level_colour(level: f32) -> u8 {
    if level > 0.85 {
        196 // bright red   — clipping / very loud
    } else if level > 0.65 {
        214 // orange       — loud
    } else if level > 0.40 {
        226 // yellow       — mid
    } else {
        46  // green        — normal
    }
}

// ── Struct ────────────────────────────────────────────────────────────────────

pub struct VuViz {
    // Smoothed RMS level for each channel, in [0, 1].
    level_l: f32,
    level_r: f32,

    // Peak-hold values and their timers.
    peak_l:  f32,
    peak_r:  f32,
    timer_l: f32, // seconds since peak_l was last refreshed
    timer_r: f32,

    // Source device name shown in the status bar.
    source: String,
}

impl VuViz {
    pub fn new(source: &str) -> Self {
        Self {
            level_l: 0.0,
            level_r: 0.0,
            peak_l:  0.0,
            peak_r:  0.0,
            timer_l: 0.0,
            timer_r: 0.0,
            source:  source.to_string(),
        }
    }

    /// Compute RMS of a sample slice and return a value in [0, 1].
    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let mean_sq = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
        mean_sq.sqrt()
    }

    /// Update one channel's smoothed level and peak marker.
    ///
    /// `level`  — mutable reference to the smoothed level accumulator
    /// `peak`   — mutable reference to the peak value
    /// `timer`  — mutable reference to the peak hold timer (seconds)
    /// `raw`    — instantaneous RMS for this frame
    /// `dt`     — elapsed time since the last tick (seconds)
    fn update_channel(level: &mut f32, peak: &mut f32, timer: &mut f32, raw: f32, dt: f32) {
        // Asymmetric exponential moving average: fast attack, slow decay.
        let alpha = if raw > *level { RISE } else { FALL };
        *level = alpha * *level + (1.0 - alpha) * raw;

        // Peak hold: refresh if the smoothed level exceeds the stored peak.
        if *level > *peak {
            *peak  = *level;
            *timer = 0.0;
        } else {
            *timer += dt;
            if *timer > PEAK_HOLD {
                *peak = (*peak - PEAK_FALL * dt).max(0.0);
            }
        }
    }

    /// Render a single horizontal VU bar row.
    ///
    /// `label`     — short label drawn before the bar (e.g. " L ")
    /// `level`     — normalised fill level [0, 1]
    /// `peak`      — normalised peak marker position [0, 1]
    /// `bar_width` — number of characters available for the bar itself
    fn render_bar(label: &str, level: f32, peak: f32, bar_width: usize) -> String {
        if bar_width == 0 {
            return label.to_string();
        }

        let filled     = (level * bar_width as f32).round() as usize;
        let peak_pos   = (peak  * bar_width as f32).round() as usize;

        let mut bar = String::with_capacity(bar_width * 20);

        for i in 0..bar_width {
            if i < filled {
                // Filled segment — colour depends on position along the bar.
                let pos_frac = i as f32 / bar_width as f32;
                let code     = level_colour(pos_frac);
                bar.push_str(&format!("\x1b[38;5;{code}m█\x1b[0m"));
            } else if i == peak_pos && peak > 0.01 {
                // Peak marker — use the colour for that position.
                let pos_frac = i as f32 / bar_width as f32;
                let code     = level_colour(pos_frac);
                bar.push_str(&format!("\x1b[1m\x1b[38;5;{code}m▌\x1b[0m"));
            } else {
                // Empty segment — dim background tick every 10%.
                let is_tick = (i + 1) % (bar_width / 10).max(1) == 0;
                if is_tick {
                    bar.push_str("\x1b[38;5;236m·\x1b[0m");
                } else {
                    bar.push(' ');
                }
            }
        }

        format!("{label}{bar}")
    }
}

// ── Visualizer impl ───────────────────────────────────────────────────────────

impl Visualizer for VuViz {
    fn name(&self)        -> &str { "vu" }
    fn description(&self) -> &str { "Stereo VU meter (reference implementation)" }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, _size: TermSize) {
        let raw_l = Self::rms(&audio.left);
        let raw_r = Self::rms(&audio.right);

        Self::update_channel(&mut self.level_l, &mut self.peak_l, &mut self.timer_l, raw_l, dt);
        Self::update_channel(&mut self.level_r, &mut self.peak_r, &mut self.timer_r, raw_r, dt);
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;

        // Layout:
        //   row 0        — blank padding
        //   row 1        — title
        //   row 2        — blank
        //   row 3        — L bar
        //   row 4        — blank
        //   row 5        — R bar
        //   row 6        — blank
        //   row 7..n-2   — more blank padding
        //   row n-1      — status bar

        // Label is " L  " or " R  " — 4 chars wide.
        let label_w  = 4;
        let bar_w    = cols.saturating_sub(label_w);

        let mut lines: Vec<String> = Vec::with_capacity(rows);

        // Title
        lines.push(String::new());
        let title = " VU METER ";
        let pad   = cols.saturating_sub(title.len()) / 2;
        lines.push(format!("\x1b[1m\x1b[38;5;255m{}{}\x1b[0m", " ".repeat(pad), title));
        lines.push(String::new());

        // Left channel
        lines.push(Self::render_bar(" L  ", self.level_l, self.peak_l, bar_w));
        lines.push(String::new());

        // Right channel
        lines.push(Self::render_bar(" R  ", self.level_r, self.peak_r, bar_w));
        lines.push(String::new());

        // Status bar (always the last row)
        lines.push(status_bar(cols, fps, self.name(), &self.source, ""));

        pad_frame(lines, rows, cols)
    }
}

// ── Registration ──────────────────────────────────────────────────────────────

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(VuViz::new(""))]
}

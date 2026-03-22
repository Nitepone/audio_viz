/// spectrum.rs — Classic log-spaced vertical frequency bar visualizer.
///
/// One vertical bar per terminal column.  Each bar's height is the smoothed
/// RMS energy of a log-spaced frequency band.  Peak markers (* characters)
/// float at the highest recent value and slowly fall.
///
/// Frequency labels (30 Hz … 16 kHz) are printed along the bottom.

use crate::visualizer::{
    pad_frame, specgrad, status_bar, hline, title_line,
    AudioFrame, SpectrumBars, TermSize, Visualizer,
};

pub struct SpectrumViz {
    bars:   SpectrumBars,
    source: String, // display name of the audio source for the status bar
}

impl SpectrumViz {
    pub fn new(source: &str) -> Self {
        // Start with an arbitrary bar count; resize() will correct it on the
        // first frame before render() is ever called.
        Self {
            bars:   SpectrumBars::new(80),
            source: source.to_string(),
        }
    }
}

impl Visualizer for SpectrumViz {
    fn name(&self)        -> &str { "spectrum" }
    fn description(&self) -> &str { "Classic log-spaced frequency bars" }

    fn on_resize(&mut self, size: TermSize) {
        self.bars.resize(size.cols as usize);
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: TermSize) {
        self.bars.resize(size.cols as usize);
        self.bars.update(&audio.fft, dt);
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        // Reserve 4 rows for: title, separator, freq labels, status bar.
        // Ensure at least 4 visible rows of spectrum.
        let vis  = (rows.saturating_sub(4)).max(4);

        let mut lines = Vec::with_capacity(rows);
        lines.push(title_line(cols, " SPECTRUM ANALYZER ", 255));
        lines.push(hline(cols, 238));

        // Draw bars from top to bottom.
        // row 0 = top of visible area (threshold ≈ 1.0),
        // row vis-1 = bottom (threshold ≈ 0).
        for row in (0..vis).rev() {
            let threshold = row as f32 / vis as f32;
            let mut line  = String::with_capacity(cols * 12);

            for bi in 0..cols {
                let bh   = self.bars.smoothed[bi.min(self.bars.smoothed.len() - 1)];
                let ph   = self.bars.peaks   [bi.min(self.bars.peaks.len()    - 1)];
                let frac = bi as f32 / (cols - 1).max(1) as f32;
                let code = specgrad(frac);
                let pkr  = (ph * vis as f32) as usize; // row index of peak marker

                if bh >= threshold {
                    // Bar body — bright near top, dim near bottom
                    let pfx = if threshold > 0.75 {
                        "\x1b[1m"
                    } else if threshold < 0.25 {
                        "\x1b[2m"
                    } else {
                        ""
                    };
                    line.push_str(&format!("{pfx}\x1b[38;5;{code}m|\x1b[0m"));
                } else if pkr > 0 && row == pkr - 1 && ph > 0.03 {
                    // Peak marker
                    line.push_str(&format!("\x1b[1m\x1b[38;5;{code}m*\x1b[0m"));
                } else {
                    line.push(' ');
                }
            }
            lines.push(line);
        }

        lines.push(hline(cols, 238));

        // Frequency labels along the bottom
        let mut label_row: Vec<u8> = vec![b' '; cols];
        let log_lo = 30f32.log10();
        let log_hi = 18_000f32.log10();
        for (freq, lbl) in &[
            (30u32, "30"), (60, "60"), (125, "125"), (250, "250"),
            (500, "500"), (1000, "1k"), (2000, "2k"), (4000, "4k"),
            (8000, "8k"), (16000, "16k"),
        ] {
            let f    = (*freq as f32).log10();
            let frac = (f - log_lo) / (log_hi - log_lo);
            let col  = ((frac * (cols - 1) as f32) as usize).min(cols - 1);
            for (i, ch) in lbl.bytes().enumerate() {
                if col + i < cols {
                    label_row[col + i] = ch;
                }
            }
        }
        let label_str = String::from_utf8(label_row).unwrap_or_default();
        lines.push(format!("\x1b[38;5;245m{}\x1b[0m", label_str));

        lines.push(status_bar(cols, fps, self.name(), &self.source, ""));

        pad_frame(lines, rows, cols)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    // The source name is injected by main.rs via the constructor.
    // build.rs calls register() with no arguments, so we use a placeholder
    // that main.rs replaces by calling SpectrumViz::new(source) directly.
    // See main.rs: visualizers are constructed via their own new() functions,
    // not via the generic register() path for the default source.
    vec![Box::new(SpectrumViz::new(""))]
}

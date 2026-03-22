/// scope.rs — Dual-channel time-domain oscilloscope.
///
/// The left and right audio channels are drawn as separate waveform panels,
/// stacked vertically.  Steep slopes are bridged with '|' connectors.
/// A dim zero-line runs through the centre of each panel.
///
/// Left channel: cyan (256-colour 51 / 39)
/// Right channel: orange (256-colour 214 / 208)

use crate::visualizer::{
    pad_frame, status_bar, hline, title_line,
    AudioFrame, TermSize, Visualizer, FFT_SIZE,
};

pub struct ScopeViz {
    left:   Vec<f32>,
    right:  Vec<f32>,
    source: String,
}

impl ScopeViz {
    pub fn new(source: &str) -> Self {
        Self {
            left:   vec![0.0; FFT_SIZE],
            right:  vec![0.0; FFT_SIZE],
            source: source.to_string(),
        }
    }

    /// Render one waveform channel into `height` rows × `cols` columns.
    ///
    /// Returns a Vec of ANSI-coloured strings, one per row.
    /// `color_hi` is used for high-amplitude regions, `color_lo` for quiet ones.
    fn draw_wave(samples: &[f32], height: usize, cols: usize, color_hi: u8, color_lo: u8)
        -> Vec<String>
    {
        // Character and colour grids (row-major)
        let mut chars:  Vec<Vec<char>>  = vec![vec![' '; cols]; height];
        let mut colors: Vec<Vec<u8>>    = vec![vec![0;   cols]; height];
        let mut bolds:  Vec<Vec<bool>>  = vec![vec![false; cols]; height];

        let zero = height / 2;

        // Dim zero-line
        for c in 0..cols {
            chars [zero][c] = '-';
            colors[zero][c] = 234;
        }

        if samples.len() < 2 {
            return chars.iter()
                .map(|row| row.iter().collect())
                .collect();
        }

        // Resample the waveform to exactly `cols` display columns
        let mut rpos = vec![0usize; cols];
        let mut amps = vec![0f32;  cols];
        for xi in 0..cols {
            let src_idx = (xi as f32 / (cols - 1).max(1) as f32
                * (samples.len() - 1) as f32) as usize;
            let amp = samples[src_idx.min(samples.len() - 1)];
            amps[xi] = amp;
            // Map amplitude [-1,1] to row index; positive amplitude → higher on screen
            let row = ((1.0 - amp) * 0.5 * (height - 1) as f32)
                .round()
                .clamp(0.0, (height - 1) as f32) as usize;
            rpos[xi] = row;
        }

        // Draw with vertical line bridging between adjacent columns
        let mut prev = rpos[0];
        for xi in 0..cols {
            let cur  = rpos[xi];
            let amp  = amps[xi].abs();
            let code = if amp > 0.45 { color_hi } else { color_lo };
            let bold = amp > 0.3;

            let lo_r = prev.min(cur);
            let hi_r = prev.max(cur);
            for r in lo_r..=hi_r {
                chars [r][xi] = if r != cur { '|' } else if bold { '*' } else { '.' };
                colors[r][xi] = code;
                bolds [r][xi] = bold && (r == cur);
            }
            prev = cur;
        }

        // Render each row to a String
        chars
            .iter()
            .enumerate()
            .map(|(r, row)| {
                let mut s = String::with_capacity(cols * 12);
                for c in 0..cols {
                    let ch   = row[c];
                    let code = colors[r][c];
                    if code > 0 {
                        let bold_pfx = if bolds[r][c] { "\x1b[1m" } else { "" };
                        s.push_str(&format!("{bold_pfx}\x1b[38;5;{code}m{ch}\x1b[0m"));
                    } else {
                        s.push(ch);
                    }
                }
                s
            })
            .collect()
    }

    /// Separator line with a centred coloured label.
    fn sep(cols: usize, label: &str, lcolor: u8) -> String {
        let lbl  = format!(" {label} ");
        let ld   = 3;
        let rd   = cols.saturating_sub(ld + lbl.len());
        format!(
            "\x1b[2m\x1b[38;5;238m{dashes}\x1b[0m\x1b[1m\x1b[38;5;{lcolor}m{lbl}\x1b[0m\x1b[2m\x1b[38;5;238m{rdashes}\x1b[0m",
            dashes  = "-".repeat(ld),
            rdashes = "-".repeat(rd),
        )
    }
}

impl Visualizer for ScopeViz {
    fn name(&self)        -> &str { "scope" }
    fn description(&self) -> &str { "Dual-channel time-domain oscilloscope" }

    fn tick(&mut self, audio: &AudioFrame, _dt: f32, _size: TermSize) {
        self.left.clone_from(&audio.left);
        self.right.clone_from(&audio.right);
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        // Reserve: title + 2 separators + hline + status = 5 rows
        let vis  = (rows.saturating_sub(5)).max(4);
        let half = vis / 2;

        let mut lines = Vec::with_capacity(rows);
        lines.push(title_line(cols, " OSCILLOSCOPE ", 51));
        lines.push(Self::sep(cols, "LEFT  ch.1", 51));
        lines.extend(Self::draw_wave(&self.left,  half,       cols, 51,  39));
        lines.push(Self::sep(cols, "RIGHT ch.2", 214));
        lines.extend(Self::draw_wave(&self.right, vis - half, cols, 214, 208));
        lines.push(hline(cols, 238));
        lines.push(status_bar(cols, fps, self.name(), &self.source, ""));

        pad_frame(lines, rows, cols)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(ScopeViz::new(""))]
}

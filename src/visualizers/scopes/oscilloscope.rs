/// oscilloscope.rs — High-fidelity XY oscilloscope with triggering and grid.
///
/// This visualizer maps the left channel to the X-axis and the right channel to the Y-axis.
/// To prevent the "dancing" effect of raw XY plots, it implements a basic "trigger"
/// mechanism that finds a stable zero-crossing to start the trace.
///
/// ═══════════════════════════════════════════════════════════════════════════
///  FEATURES
/// ═══════════════════════════════════════════════════════════════════════════
///  - XY Plotting: L/R audio channels mapped to terminal coordinates.
///  - Triggering: Stabilizes the waveform by starting the trace at a zero-crossing.
///  - Grid: A dim, centered grid for scale reference.
///  - CRT Aesthetic: High-contrast phosphor colors (Green/Amber).

// ── Index: OscilloscopeViz@15 · new@22 · tick@140 · render@195 · config@245 · set_config@275 · register@325
use crate::visualizer::{
    merge_config,
    pad_frame, status_bar, hline, title_line,
    AudioFrame, TermSize, Visualizer, FFT_SIZE, SAMPLE_RATE,
};

const CONFIG_VERSION: u64 = 1;

pub struct OscilloscopeViz {
    left:      Vec<f32>,
    right:     Vec<f32>,
    source:    String,
    // ── Config fields ──────────────────────────────────────────────────────
    gain:          f32,
    /// The threshold amplitude for the trigger (0.0 to 1.0).
    trigger_level: f32,
    /// Vertical scale multiplier.
    v_scale:       f32,
    /// Horizontal scale multiplier (controls how many samples to display).
    h_scale:       f32,
    /// Whether to show a faint measurement grid and unit labels.
    show_grid:     bool,
    theme:         Theme,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Theme {
    PhosphorGreen,
    Amber,
}

impl Theme {
    fn from_str(s: &str) -> Self {
        match s {
            "amber" => Theme::Amber,
            _ => Theme::PhosphorGreen,
        }
    }
    fn color(&self) -> u8 {
        match self {
            Theme::PhosphorGreen => 46,
            Theme::Amber => 214,
        }
    }
}

impl OscilloscopeViz {
    pub fn new(source: &str) -> Self {
        Self {
            left:           vec![0.0; FFT_SIZE],
            right:          vec![0.0; FFT_SIZE],
            source:         source.to_string(),
            gain:           1.0,
            trigger_level:  0.05,
            v_scale:        1.0,
            h_scale:        1.0,
            show_grid:      false,
            theme:          Theme::PhosphorGreen,
        }
    }

    /// Find the index of the first zero-crossing with a positive slope
    /// that exceeds the trigger level.
    fn find_trigger_index(&self, right: &[f32]) -> usize {
        for i in 1..right.len() {
            if right[i-1] < self.trigger_level && right[i] >= self.trigger_level {
                return i;
            }
        }
        0
    }
}

impl Visualizer for OscilloscopeViz {
    fn name(&self)        -> &str { "oscilloscope" }
    fn description(&self) -> &str { "High-fidelity XY oscilloscope with triggering" }

    fn get_default_config(&self) -> String {
        serde_json::json!({
            "visualizer_name": "oscilloscope",
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
                    "name": "v_scale",
                    "display_name": "Vertical Scale",
                    "type": "float",
                    "value": 1.0,
                    "min": 0.1,
                    "max": 5.0
                },
                {
                    "name": "h_scale",
                    "display_name": "Horizontal Scale",
                    "type": "float",
                    "value": 1.0,
                    "min": 0.1,
                    "max": 2.0
                },
                {
                    "name": "trigger_level",
                    "display_name": "Trigger Level",
                    "type": "float",
                    "value": 0.05,
                    "min": 0.0,
                    "max": 0.5
                },
                {
                    "name": "theme",
                    "display_name": "Theme",
                    "type": "enum",
                    "value": "green",
                    "variants": ["green", "amber"]
                },
                {
                    "name": "show_grid",
                    "display_name": "Show Grid",
                    "type": "bool",
                    "value": false
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
                    "gain"          => self.gain          = entry["value"].as_f64().unwrap_or(1.0) as f32,
                    "v_scale"       => self.v_scale       = entry["value"].as_f64().unwrap_or(1.0) as f32,
                    "h_scale"       => self.h_scale       = entry["value"].as_f64().unwrap_or(1.0) as f32,
                    "trigger_level" => self.trigger_level = entry["value"].as_f64().unwrap_or(0.05) as f32,
                    "theme"         => self.theme         = Theme::from_str(entry["value"].as_str().unwrap_or("green")),
                    "show_grid"     => self.show_grid     = entry["value"].as_bool().unwrap_or(false),
                    _ => {}
                }
            }
        }
        Ok(merged)
    }

    fn tick(&mut self, audio: &AudioFrame, _dt: f32, _size: TermSize) {
        let gain = self.gain;
        for i in 0..audio.left.len().min(FFT_SIZE) {
            self.left[i] = audio.left[i] * gain;
        }
        for i in 0..audio.right.len().min(FFT_SIZE) {
            self.right[i] = audio.right[i] * gain;
        }
    }

    fn render(&self, size: TermSize, fps: f32) -> Vec<String> {
        let rows = size.rows as usize;
        let cols = size.cols as usize;
        let color = self.theme.color();

        let mut grid: Vec<Vec<char>> = vec![vec![' '; cols]; rows];
        let mut grid_colors: Vec<Vec<u8>> = vec![vec![0; cols]; rows];

        let mid_x = cols / 2;
        let mid_y = rows / 2;

        for x in 0..cols {
            grid[mid_y][x] = '·';
            grid_colors[mid_y][x] = 236;
        }
        for y in 0..rows {
            grid[y][mid_x] = '·';
            grid_colors[y][mid_x] = 236;
        }

        if self.show_grid {
            let grid_color = 235;
            for x in 1..4 {
                let pos = cols * x / 4;
                if pos < cols {
                    for y in 0..rows {
                        if grid[y][pos] == ' ' {
                            grid[y][pos] = '·';
                            grid_colors[y][pos] = grid_color;
                        }
                    }
                }
            }
            for y in 1..4 {
                let pos = rows * y / 4;
                if pos < rows {
                    for x in 0..cols {
                        if grid[pos][x] == ' ' {
                            grid[pos][x] = '·';
                            grid_colors[pos][x] = grid_color;
                        }
                    }
                }
            }

            let label_color = 234;
            let y_labels = [0.25, 0.5, 0.75, 1.0];
            for &val in &y_labels {
                let y_raw = (rows as f32 / 2.0) - (val * self.v_scale * (rows as f32 / 2.0));
                let y = y_raw.round() as isize;
                if y >= 0 && y < rows as isize {
                    let label = format!("{:.2}", val);
                    let start_x = 1;
                    for (i, ch) in label.chars().enumerate() {
                        if start_x + i < cols && grid[y as usize][start_x + i] == ' ' {
                            grid[y as usize][start_x + i] = ch;
                            grid_colors[y as usize][start_x + i] = label_color;
                        }
                    }
                }
            }
            let x_labels = [0.25, 0.5, 0.75];
            let n_samples = (512.0 * self.h_scale) as usize;
            let n_samples = n_samples.clamp(2, FFT_SIZE);
            for &val in &x_labels {
                let x_f = val * (cols - 1) as f32;
                let x = x_f.round() as usize;
                let label = format!("{:.0}s", val * (n_samples as f32 / SAMPLE_RATE as f32));
                let start_y = rows - 2;
                if start_y < rows {
                    for (i, ch) in label.chars().enumerate() {
                        if x + i < cols && grid[start_y][x + i] == ' ' {
                            grid[start_y][x + i] = ch;
                            grid_colors[start_y][x + i] = label_color;
                        }
                    }
                }
            }
        }

        let trigger_idx = self.find_trigger_index(&self.right);
        let n_samples = (512.0 * self.h_scale) as usize;
        let n_samples = n_samples.clamp(2, FFT_SIZE);

        let start_idx = trigger_idx.min(FFT_SIZE - n_samples);
        let end_idx   = (start_idx + n_samples).min(FFT_SIZE);

        for i in start_idx..end_idx {
            let rel_idx = i - start_idx;
            let x_f = (rel_idx as f32 / (n_samples - 1) as f32) * (cols - 1) as f32;
            let x = x_f.round() as usize;

            let y_f = self.right[i] * self.v_scale * (rows as f32 / 2.0);
            let y_raw = (rows as f32 / 2.0) - y_f;
            let y = y_raw.round() as isize;

            if x < cols && y >= 0 && y < rows as isize {
                let y = y as usize;
                grid[y][x] = '*';
                grid_colors[y][x] = color;
            }
        }

        let mut lines = Vec::with_capacity(rows);
        lines.push(title_line(cols, " OSCILLOSCOPE ", color));
        lines.push(hline(cols, 238));

        for r in 0..rows {
            let mut line = String::with_capacity(cols * 12);
            for c in 0..cols {
                let ch = grid[r][c];
                let col = grid_colors[r][c];
                if col > 0 {
                    line.push_str(&format!("\x1b[38;5;{color}m{ch}\x1b[0m"));
                } else {
                    line.push(ch);
                }
            }
            lines.push(line);
        }

        lines.push(hline(cols, 238));
        lines.push(status_bar(cols, fps, self.name(), &self.source, ""));

        pad_frame(lines, rows, cols)
    }
}

pub fn register() -> Vec<Box<dyn Visualizer>> {
    vec![Box::new(OscilloscopeViz::new(""))]
}

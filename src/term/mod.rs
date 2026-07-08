/// term/ — Terminal render layer: runs TUI visualizers in the windowed app.
///
/// `TermViz` adapts the legacy terminal `Visualizer` trait
/// (crate::tui::visualizer — ANSI strings over a rows×cols character grid)
/// onto the windowed app's software-framebuffer `Visualizer` trait
/// (crate::visualizer).  Each frame:
///
///   1. tick(): derive a virtual terminal size from the render pane's pixel
///      size and the current font's cell metrics, then tick the wrapped TUI
///      visualizer with a TUI-shaped AudioFrame.
///   2. render_software(): ask the TUI visualizer for its ANSI frame, parse
///      it into a cell grid (ansi.rs), and rasterise the grid with the
///      configured font (font.rs) into the RGBA framebuffer, centred.
///
/// The adapter appends three settings to the wrapped visualizer's config
/// schema — font family, font size, and whether to show the TUI status bar
/// — so every installed TUI visualizer gets them in the settings panel for
/// free.  TUI visualizer sources are installed under src/visualizers/tui/;
/// see that directory and CLAUDE.md for the install recipe.

pub mod ansi;
pub mod font;

use crate::term::font::{TermRenderer, DEFAULT_FONT, FONT_VARIANTS};
use crate::tui::visualizer::{
    AudioFrame as TuiAudioFrame, TermSize, Visualizer as TuiVisualizer,
};
use crate::visualizer::{AudioFrame, Framebuffer, PixelSize, RenderMode, Visualizer};

const DEFAULT_FONT_PX: i64 = 16;

pub struct TermViz {
    inner: Box<dyn TuiVisualizer>,
    renderer: TermRenderer,
    fb: Framebuffer,
    /// Terminal size passed to the wrapped visualizer's tick()/render().
    term_size: TermSize,
    /// Rows actually painted (excludes the status-bar row when hidden).
    shown_rows: usize,
    show_status_bar: bool,
    /// Reused TUI-shaped audio frame to avoid per-frame allocations.
    tui_audio: TuiAudioFrame,
    fps_ema: f32,
}

impl TermViz {
    pub fn new(inner: Box<dyn TuiVisualizer>) -> Self {
        Self {
            inner,
            renderer: TermRenderer::new(DEFAULT_FONT, DEFAULT_FONT_PX as u32),
            fb: Framebuffer::new(1, 1),
            term_size: TermSize { rows: 2, cols: 2 },
            shown_rows: 1,
            show_status_bar: false,
            tui_audio: TuiAudioFrame {
                left: Vec::new(),
                right: Vec::new(),
                mono: Vec::new(),
                fft: Vec::new(),
                sample_rate: crate::visualizer::SAMPLE_RATE,
            },
            fps_ema: 60.0,
        }
    }

    /// Wrap a TUI `register()` result for the windowed registry — the whole
    /// body of an installed visualizer's `register()`:
    ///
    ///   pub fn register() -> Vec<Box<dyn crate::visualizer::Visualizer>> {
    ///       crate::term::TermViz::adapt(vec![Box::new(MyViz::new(""))])
    ///   }
    pub fn adapt(inner: Vec<Box<dyn TuiVisualizer>>) -> Vec<Box<dyn Visualizer>> {
        inner.into_iter().map(|v| Box::new(TermViz::new(v)) as Box<dyn Visualizer>).collect()
    }
}

impl Visualizer for TermViz {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn mode(&self) -> RenderMode {
        RenderMode::Software
    }

    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: PixelSize) {
        // Virtual terminal size from pixel pane / font cell metrics.
        let cols = (size.width / self.renderer.cell_w).clamp(2, 800) as u16;
        let rows = (size.height / self.renderer.cell_h).clamp(2, 400) as usize;
        // When the status bar is hidden, simulate one extra row and simply
        // never paint it — TUI visualizers reserve their last row for it.
        let term_rows = if self.show_status_bar { rows } else { rows + 1 };
        let term_size = TermSize { rows: term_rows as u16, cols };
        if term_size != self.term_size {
            self.term_size = term_size;
            self.inner.on_resize(term_size);
        }
        self.shown_rows = rows;

        self.tui_audio.left.clear();
        self.tui_audio.left.extend_from_slice(&audio.left);
        self.tui_audio.right.clear();
        self.tui_audio.right.extend_from_slice(&audio.right);
        self.tui_audio.mono.clear();
        self.tui_audio.mono.extend_from_slice(&audio.mono);
        self.tui_audio.fft.clear();
        self.tui_audio.fft.extend_from_slice(&audio.fft);
        self.tui_audio.sample_rate = audio.sample_rate;

        self.fps_ema = 0.9 * self.fps_ema + 0.1 / dt.max(1e-6);
        self.inner.tick(&self.tui_audio, dt, term_size);
    }

    fn render_software(&mut self, size: PixelSize) -> Option<&Framebuffer> {
        self.fb.ensure_size(size);
        self.fb.data.fill(0);

        let lines = self.inner.render(self.term_size, self.fps_ema);
        let grid = ansi::parse_frame(&lines, self.shown_rows, self.term_size.cols as usize);
        self.renderer.paint(&grid, &mut self.fb);
        Some(&self.fb)
    }

    fn get_default_config(&self) -> String {
        // The wrapped visualizer's schema plus the terminal-layer settings.
        let mut val: serde_json::Value = serde_json::from_str(&self.inner.get_default_config())
            .unwrap_or_else(|_| serde_json::json!({ "config": [] }));
        if let Some(entries) = val["config"].as_array_mut() {
            entries.push(serde_json::json!({
                "name": "term_font",
                "display_name": "Font",
                "type": "enum",
                "value": DEFAULT_FONT,
                "variants": FONT_VARIANTS,
            }));
            entries.push(serde_json::json!({
                "name": "term_font_size",
                "display_name": "Font Size (px)",
                "type": "int",
                "value": DEFAULT_FONT_PX,
                "min": 6,
                "max": 72,
            }));
            entries.push(serde_json::json!({
                "name": "term_status_bar",
                "display_name": "Show Status Bar",
                "type": "bool",
                "value": false,
            }));
        }
        val.to_string()
    }

    fn set_config(&mut self, json: &str) -> Result<String, String> {
        let merged = crate::config::merge_config(&self.get_default_config(), json);
        let val: serde_json::Value =
            serde_json::from_str(&merged).map_err(|e| format!("JSON parse error: {e}"))?;

        let mut family = DEFAULT_FONT.to_string();
        let mut px = DEFAULT_FONT_PX;
        if let Some(entries) = val["config"].as_array() {
            for entry in entries {
                match entry["name"].as_str().unwrap_or("") {
                    "term_font" => {
                        if let Some(s) = entry["value"].as_str() {
                            family = s.to_string();
                        }
                    }
                    "term_font_size" => px = entry["value"].as_i64().unwrap_or(DEFAULT_FONT_PX),
                    "term_status_bar" => {
                        self.show_status_bar = entry["value"].as_bool().unwrap_or(false);
                    }
                    _ => {}
                }
            }
        }
        self.renderer.configure(&family, px.clamp(6, 96) as u32);

        // The wrapped visualizer merges against its own schema, silently
        // dropping the term_* entries; the combined `merged` is what we
        // persist and report back.
        self.inner.set_config(&merged)?;
        Ok(merged)
    }
}

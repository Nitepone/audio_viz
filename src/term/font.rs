/// font.rs — Monospace glyph rasterisation for the terminal render layer.
///
/// Renders a parsed `CellGrid` (ansi.rs) into the software `Framebuffer`.
/// Text glyphs are rasterised with fontdue from fonts embedded in the
/// binary (assets/fonts/) and cached per (char, bold) at the current pixel
/// size.  Block/shade/half-block and Braille characters — the workhorses of
/// TUI visualizers — are drawn geometrically instead of through the font so
/// cells tile seamlessly with no inter-cell gaps at any size.

use std::collections::HashMap;

use crate::term::ansi::CellGrid;
use crate::visualizer::Framebuffer;

// ── Embedded font families ───────────────────────────────────────────────────

static DEJAVU: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono.ttf");
static DEJAVU_BOLD: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono-Bold.ttf");
static FIRA: &[u8] = include_bytes!("../../assets/fonts/FiraCode-Regular.ttf");
static FIRA_BOLD: &[u8] = include_bytes!("../../assets/fonts/FiraCode-Bold.ttf");

/// Config-facing font family names (enum variants in the settings panel).
pub const FONT_VARIANTS: [&str; 2] = ["DejaVu Sans Mono", "Fira Code"];
pub const DEFAULT_FONT: &str = FONT_VARIANTS[0];

struct FontPair {
    regular: fontdue::Font,
    bold: fontdue::Font,
}

fn load(bytes: &[u8]) -> fontdue::Font {
    fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
        .expect("embedded font must parse")
}

fn font_pair(family: &str) -> &'static FontPair {
    use std::sync::OnceLock;
    static DEJAVU_PAIR: OnceLock<FontPair> = OnceLock::new();
    static FIRA_PAIR: OnceLock<FontPair> = OnceLock::new();
    match family {
        "Fira Code" => {
            FIRA_PAIR.get_or_init(|| FontPair { regular: load(FIRA), bold: load(FIRA_BOLD) })
        }
        _ => DEJAVU_PAIR
            .get_or_init(|| FontPair { regular: load(DEJAVU), bold: load(DEJAVU_BOLD) }),
    }
}

// ── Renderer ─────────────────────────────────────────────────────────────────

/// One cached rasterised glyph: fontdue coverage bitmap + placement metrics.
struct Glyph {
    metrics: fontdue::Metrics,
    coverage: Vec<u8>,
}

pub struct TermRenderer {
    family: String,
    px: f32,
    /// Integer cell metrics derived from the font at `px`.
    pub cell_w: u32,
    pub cell_h: u32,
    baseline: i32,
    cache: HashMap<(char, bool), Glyph>,
}

impl TermRenderer {
    pub fn new(family: &str, px: u32) -> Self {
        let mut r = Self {
            family: String::new(),
            px: 0.0,
            cell_w: 1,
            cell_h: 1,
            baseline: 0,
            cache: HashMap::new(),
        };
        r.configure(family, px);
        r
    }

    /// Set font family + pixel size, recomputing cell metrics and dropping
    /// the glyph cache when anything changed.
    pub fn configure(&mut self, family: &str, px: u32) {
        let px = px.clamp(6, 96) as f32;
        if self.family == family && self.px == px {
            return;
        }
        self.family = family.to_string();
        self.px = px;
        self.cache.clear();

        let font = &font_pair(&self.family).regular;
        let line = font
            .horizontal_line_metrics(px)
            .expect("monospace font must have horizontal metrics");
        self.cell_h = line.new_line_size.round().max(1.0) as u32;
        self.cell_w = font.metrics('M', px).advance_width.round().max(1.0) as u32;
        self.baseline = line.ascent.round() as i32;
    }

    /// Paint the grid into `fb`, centred, over the existing (cleared) pixels.
    pub fn paint(&mut self, grid: &CellGrid, fb: &mut Framebuffer) {
        let grid_w = grid.cols as u32 * self.cell_w;
        let grid_h = grid.rows as u32 * self.cell_h;
        let x0 = (fb.width.saturating_sub(grid_w) / 2) as i32;
        let y0 = (fb.height.saturating_sub(grid_h) / 2) as i32;

        for r in 0..grid.rows {
            for c in 0..grid.cols {
                let cell = grid.cells[r * grid.cols + c];
                let cx = x0 + (c as u32 * self.cell_w) as i32;
                let cy = y0 + (r as u32 * self.cell_h) as i32;

                if let Some(bg) = cell.bg {
                    fill_rect(fb, cx, cy, self.cell_w, self.cell_h, bg, 1.0);
                }
                if cell.ch == ' ' {
                    continue;
                }
                if !self.draw_geometric(fb, cx, cy, cell.ch, cell.fg) {
                    self.draw_glyph(fb, cx, cy, cell.ch, cell.bold, cell.fg);
                }
            }
        }
    }

    /// Block elements, shades and Braille are drawn as exact rectangles /
    /// dot grids so adjacent cells tile without seams. Returns false when
    /// the char is not geometric and should go through the font.
    fn draw_geometric(&self, fb: &mut Framebuffer, x: i32, y: i32, ch: char, fg: [u8; 3]) -> bool {
        let (w, h) = (self.cell_w, self.cell_h);
        match ch {
            // Shades: full-cell rectangles at partial opacity.
            '░' => fill_rect(fb, x, y, w, h, fg, 0.30),
            '▒' => fill_rect(fb, x, y, w, h, fg, 0.55),
            '▓' => fill_rect(fb, x, y, w, h, fg, 0.80),
            '█' => fill_rect(fb, x, y, w, h, fg, 1.0),
            // Lower blocks ▁▂▃▄▅▆▇ (U+2581–2587): bottom n/8 of the cell.
            '\u{2581}'..='\u{2587}' => {
                let n = ch as u32 - 0x2580; // 1..=7 eighths
                let bh = (h * n + 4) / 8;
                fill_rect(fb, x, y + (h - bh) as i32, w, bh, fg, 1.0);
            }
            '▀' => fill_rect(fb, x, y, w, h / 2, fg, 1.0),
            // Left blocks ▉▊▋▌▍▎▏ (U+2589–258F): left (8-n)/8 of the cell.
            '\u{2589}'..='\u{258F}' => {
                let n = 8 - (ch as u32 - 0x2588); // 7..=1 eighths
                fill_rect(fb, x, y, (w * n + 4) / 8, h, fg, 1.0);
            }
            '▐' => fill_rect(fb, x + (w / 2) as i32, y, w - w / 2, h, fg, 1.0),
            // Braille U+2800–28FF: 2×4 dot matrix.
            '\u{2800}'..='\u{28FF}' => {
                let bits = ch as u32 - 0x2800;
                // Bit → (col, row): 0..2 = left rows 0–2, 3..5 = right rows
                // 0–2, 6 = left row 3, 7 = right row 3.
                const POS: [(u32, u32); 8] =
                    [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)];
                let dw = (w / 2).max(1);
                let dh = (h / 4).max(1);
                for (bit, &(dc, dr)) in POS.iter().enumerate() {
                    if bits & (1 << bit) != 0 {
                        // Inset each dot slightly so distinct dots read as dots.
                        let ix = x + (dc * dw) as i32 + (dw / 4) as i32;
                        let iy = y + (dr * dh) as i32 + (dh / 4) as i32;
                        fill_rect(fb, ix, iy, (dw - dw / 4).max(1), (dh - dh / 4).max(1), fg, 1.0);
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn draw_glyph(&mut self, fb: &mut Framebuffer, x: i32, y: i32, ch: char, bold: bool, fg: [u8; 3]) {
        let glyph = self.cache.entry((ch, bold)).or_insert_with(|| {
            let pair = font_pair(&self.family);
            let mut font = if bold { &pair.bold } else { &pair.regular };
            // Fall back to DejaVu for glyphs the chosen family lacks.
            if font.lookup_glyph_index(ch) == 0 {
                let dj = font_pair(DEFAULT_FONT);
                let dj = if bold { &dj.bold } else { &dj.regular };
                if dj.lookup_glyph_index(ch) != 0 {
                    font = dj;
                }
            }
            let (metrics, coverage) = font.rasterize(ch, self.px);
            Glyph { metrics, coverage }
        });

        let gx = x + glyph.metrics.xmin;
        let gy = y + self.baseline - glyph.metrics.height as i32 - glyph.metrics.ymin;
        for (row, chunk) in glyph.coverage.chunks_exact(glyph.metrics.width).enumerate() {
            let py = gy + row as i32;
            if py < 0 || py >= fb.height as i32 {
                continue;
            }
            for (col, &cov) in chunk.iter().enumerate() {
                if cov == 0 {
                    continue;
                }
                let px = gx + col as i32;
                if px < 0 || px >= fb.width as i32 {
                    continue;
                }
                blend(fb, px as u32, py as u32, fg, cov as f32 / 255.0);
            }
        }
    }
}

// ── Pixel helpers ────────────────────────────────────────────────────────────

#[inline]
fn blend(fb: &mut Framebuffer, x: u32, y: u32, rgb: [u8; 3], alpha: f32) {
    let i = ((y * fb.width + x) * 4) as usize;
    for ch in 0..3 {
        let dst = fb.data[i + ch] as f32;
        fb.data[i + ch] = (dst + (rgb[ch] as f32 - dst) * alpha) as u8;
    }
    fb.data[i + 3] = 255;
}

fn fill_rect(fb: &mut Framebuffer, x: i32, y: i32, w: u32, h: u32, rgb: [u8; 3], alpha: f32) {
    let x_lo = x.max(0) as u32;
    let y_lo = y.max(0) as u32;
    let x_hi = ((x + w as i32).max(0) as u32).min(fb.width);
    let y_hi = ((y + h as i32).max(0) as u32).min(fb.height);
    for py in y_lo..y_hi {
        for px in x_lo..x_hi {
            if alpha >= 1.0 {
                let i = ((py * fb.width + px) * 4) as usize;
                fb.data[i] = rgb[0];
                fb.data[i + 1] = rgb[1];
                fb.data[i + 2] = rgb[2];
                fb.data[i + 3] = 255;
            } else {
                blend(fb, px, py, rgb, alpha);
            }
        }
    }
}

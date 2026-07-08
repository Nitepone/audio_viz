/// ansi.rs — ANSI/SGR escape-sequence parsing into a character-cell grid.
///
/// TUI visualizers render frames as `Vec<String>` — one string per terminal
/// row, containing plain characters interleaved with 256-colour SGR escape
/// sequences (`\x1b[38;5;196m`, `\x1b[1m`, `\x1b[0m`, …).  This module
/// parses those strings into a flat `Vec<Cell>` grid that the font
/// rasteriser (font.rs) can paint into a pixel framebuffer.
///
/// Supported SGR codes (everything the TUI visualizers emit, plus the
/// common basic-colour codes for safety): 0 reset · 1 bold · 2 dim ·
/// 22 normal intensity · 30–37/90–97 basic fg · 38;5;n 256-colour fg ·
/// 39 default fg · 40–47/100–107 basic bg · 48;5;n 256-colour bg ·
/// 49 default bg.  Unknown codes are ignored.

/// Default foreground: a light terminal grey.
pub const DEFAULT_FG: [u8; 3] = [229, 229, 229];

/// One terminal character cell.
#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg: [u8; 3],
    /// `None` = transparent (the framebuffer's black background).
    pub bg: Option<[u8; 3]>,
    pub bold: bool,
}

impl Cell {
    pub const BLANK: Cell = Cell { ch: ' ', fg: DEFAULT_FG, bg: None, bold: false };
}

/// A parsed frame: `rows * cols` cells, row-major.
pub struct CellGrid {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Cell>,
}

/// Convert an xterm-256 colour index to RGB.
pub fn ansi256_to_rgb(idx: u8) -> [u8; 3] {
    // 0–15: standard + bright colours (xterm defaults).
    const BASE: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    match idx {
        0..=15 => BASE[idx as usize],
        16..=231 => {
            // 6×6×6 colour cube; component levels 0,95,135,175,215,255.
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let i = idx as usize - 16;
            [LEVELS[i / 36], LEVELS[(i / 6) % 6], LEVELS[i % 6]]
        }
        232..=255 => {
            // 24-step greyscale ramp: 8, 18, …, 238.
            let g = 8 + 10 * (idx as u16 - 232);
            [g as u8; 3]
        }
    }
}

/// Live SGR attribute state while scanning a line.
struct SgrState {
    fg: [u8; 3],
    bg: Option<[u8; 3]>,
    bold: bool,
    dim: bool,
}

impl SgrState {
    fn reset(&mut self) {
        *self = SgrState { fg: DEFAULT_FG, bg: None, bold: false, dim: false };
    }

    /// Apply one complete SGR parameter list (the numbers between `\x1b[`
    /// and `m`).
    fn apply(&mut self, params: &[u16]) {
        let mut i = 0;
        if params.is_empty() {
            self.reset(); // bare "\x1b[m"
        }
        while i < params.len() {
            match params[i] {
                0 => self.reset(),
                1 => self.bold = true,
                2 => self.dim = true,
                22 => {
                    self.bold = false;
                    self.dim = false;
                }
                30..=37 => self.fg = ansi256_to_rgb(params[i] as u8 - 30),
                90..=97 => self.fg = ansi256_to_rgb(params[i] as u8 - 90 + 8),
                38 if params.get(i + 1) == Some(&5) => {
                    if let Some(&n) = params.get(i + 2) {
                        self.fg = ansi256_to_rgb(n.min(255) as u8);
                    }
                    i += 2;
                }
                39 => self.fg = DEFAULT_FG,
                40..=47 => self.bg = Some(ansi256_to_rgb(params[i] as u8 - 40)),
                100..=107 => self.bg = Some(ansi256_to_rgb(params[i] as u8 - 100 + 8)),
                48 if params.get(i + 1) == Some(&5) => {
                    if let Some(&n) = params.get(i + 2) {
                        self.bg = Some(ansi256_to_rgb(n.min(255) as u8));
                    }
                    i += 2;
                }
                49 => self.bg = None,
                _ => {}
            }
            i += 1;
        }
    }

    /// Dim halves perceived intensity — approximate by scaling the colour.
    fn effective_fg(&self) -> [u8; 3] {
        if self.dim {
            [
                (self.fg[0] as u16 * 55 / 100) as u8,
                (self.fg[1] as u16 * 55 / 100) as u8,
                (self.fg[2] as u16 * 55 / 100) as u8,
            ]
        } else {
            self.fg
        }
    }
}

/// Parse one frame of ANSI-escaped lines into a `rows × cols` cell grid.
/// Lines beyond `rows` and characters beyond `cols` are discarded; missing
/// ones are padded with blanks (matching terminal clipping behaviour).
pub fn parse_frame(lines: &[String], rows: usize, cols: usize) -> CellGrid {
    let mut cells = vec![Cell::BLANK; rows * cols];

    for (r, line) in lines.iter().take(rows).enumerate() {
        let mut sgr = SgrState { fg: DEFAULT_FG, bg: None, bold: false, dim: false };
        let mut col = 0usize;
        let mut chars = line.chars();

        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Expect CSI "…m"; ignore any other escape kind.
                if chars.next() != Some('[') {
                    continue;
                }
                let mut params: Vec<u16> = Vec::with_capacity(4);
                let mut cur: u16 = 0;
                let mut has_digit = false;
                for ch in chars.by_ref() {
                    match ch {
                        '0'..='9' => {
                            cur = cur.saturating_mul(10) + (ch as u16 - '0' as u16);
                            has_digit = true;
                        }
                        ';' => {
                            params.push(cur);
                            cur = 0;
                            has_digit = false;
                        }
                        'm' => {
                            if has_digit || !params.is_empty() {
                                params.push(cur);
                            }
                            sgr.apply(&params);
                            break;
                        }
                        // Not an SGR sequence (cursor moves etc.) — drop it.
                        _ => break,
                    }
                }
            } else {
                if col < cols {
                    cells[r * cols + col] = Cell {
                        ch: c,
                        fg: sgr.effective_fg(),
                        bg: sgr.bg,
                        bold: sgr.bold,
                    };
                }
                col += 1;
            }
        }
    }

    CellGrid { rows, cols, cells }
}

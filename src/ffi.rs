// src/ffi.rs — C-compatible FFI for the tvOS static library.
//
// Gated behind the `tvos` feature; never compiled into the terminal binary
// or the WASM build.
//
// Memory contract
// ───────────────
// All `*const c_char` return values are owned by the Handle (or by a process-
// wide static for aviz_list_visualizers).  The pointer is valid until the next
// call to the same function on the same handle, or until aviz_destroy.
// Swift callers must copy the string (String(cString:)) before calling again.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::OnceLock;

use crate::visualizer::{AudioFrame, TermSize, Visualizer, FFT_SIZE};
use crate::visualizers;

// ── Handle ────────────────────────────────────────────────────────────────────

pub struct Handle {
    viz:        Box<dyn Visualizer>,
    size:       TermSize,
    /// Pre-computed at creation; stable for the handle's lifetime.
    name_buf:   CString,
    render_buf: CString,
    config_buf: CString,
}

// Visualizer is Send; the handle is only ever accessed from one thread at a time
// (the Swift audio/render loop), so this is safe.
unsafe impl Send for Handle {}

// ── xterm-256 → RGB ──────────────────────────────────────────────────────────
//
// Mirrors the identical function in web/src/lib.rs.

fn xterm256_to_rgb(idx: u8) -> (u8, u8, u8) {
    match idx {
        0  => (0,   0,   0),
        1  => (128, 0,   0),
        2  => (0,   128, 0),
        3  => (128, 128, 0),
        4  => (0,   0,   128),
        5  => (128, 0,   128),
        6  => (0,   128, 128),
        7  => (192, 192, 192),
        8  => (128, 128, 128),
        9  => (255, 0,   0),
        10 => (0,   255, 0),
        11 => (255, 255, 0),
        12 => (0,   0,   255),
        13 => (255, 0,   255),
        14 => (0,   255, 255),
        15 => (255, 255, 255),
        16..=231 => {
            let n  = idx - 16;
            let b  = n % 6;
            let g  = (n / 6) % 6;
            let r  = n / 36;
            let lv = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (lv(r), lv(g), lv(b))
        }
        232..=255 => {
            let v = 8 + (idx - 232) * 10;
            (v, v, v)
        }
    }
}

// ── ANSI → Cell array ─────────────────────────────────────────────────────────
//
// Mirrors parse_frame in web/src/lib.rs, but serialises to JSON manually
// to avoid pulling in serde derive.

struct Cell {
    ch:   char,
    col:  u32,
    row:  u32,
    r:    u8,
    g:    u8,
    b:    u8,
    bold: bool,
    dim:  bool,
}

fn parse_frame(lines: &[String]) -> Vec<Cell> {
    let mut cells = Vec::with_capacity(lines.len() * 40);

    for (row_idx, line) in lines.iter().enumerate() {
        let mut col  = 0u32;
        let mut fg   = (192u8, 192u8, 192u8);
        let mut bold = false;
        let mut dim  = false;

        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '[' {
                i += 2;
                let mut params = String::new();
                while i < chars.len() && chars[i] != 'm' {
                    params.push(chars[i]);
                    i += 1;
                }
                i += 1; // consume 'm'

                let parts: Vec<&str> = params.split(';').collect();
                let mut pi = 0;
                while pi < parts.len() {
                    match parts[pi].parse::<u32>().unwrap_or(0) {
                        0  => { bold = false; dim = false; fg = (192, 192, 192); }
                        1  => { bold = true; }
                        2  => { dim = true; }
                        38 if pi + 2 < parts.len() && parts[pi + 1] == "5" => {
                            let idx = parts[pi + 2].parse::<u8>().unwrap_or(7);
                            fg = xterm256_to_rgb(idx);
                            pi += 2;
                        }
                        _ => {}
                    }
                    pi += 1;
                }
            } else {
                let ch = chars[i];
                i += 1;
                if ch != ' ' {
                    let (mut r, mut g, mut b) = fg;
                    if bold {
                        r = r.saturating_add(40);
                        g = g.saturating_add(40);
                        b = b.saturating_add(40);
                    }
                    if dim {
                        r = (r as u16 * 55 / 100) as u8;
                        g = (g as u16 * 55 / 100) as u8;
                        b = (b as u16 * 55 / 100) as u8;
                    }
                    cells.push(Cell { ch, col, row: row_idx as u32, r, g, b, bold, dim });
                }
                col += 1;
            }
        }
    }

    cells
}

fn cells_to_json(cells: &[Cell]) -> String {
    let mut out = String::with_capacity(cells.len() * 60);
    out.push('[');
    for (i, c) in cells.iter().enumerate() {
        if i > 0 { out.push(','); }
        // JSON-escape the character (block chars are safe; guard the common cases)
        let escaped = match c.ch {
            '"'  => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            ch   => ch.to_string(),
        };
        out.push_str(&format!(
            r#"{{"ch":"{}","col":{},"row":{},"r":{},"g":{},"b":{},"bold":{},"dim":{}}}"#,
            escaped, c.col, c.row, c.r, c.g, c.b, c.bold, c.dim
        ));
    }
    out.push(']');
    out
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_viz(name: &str) -> Box<dyn Visualizer> {
    let mut all = visualizers::all_visualizers();
    if let Some(pos) = all.iter().position(|v| v.name() == name) {
        return all.swap_remove(pos);
    }
    if !all.is_empty() { return all.swap_remove(0); }
    unreachable!("no visualizers registered")
}

fn to_cstring(s: String) -> CString {
    // Replace any interior NUL bytes (shouldn't occur in practice)
    CString::new(s.replace('\0', "")).unwrap_or_else(|_| CString::new("").unwrap())
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ── FFI ───────────────────────────────────────────────────────────────────────

/// Create a visualizer by name (e.g. `"spectrum"`, `"matrix"`).
/// `cols` / `rows` set the initial character-grid dimensions.
/// Returns an opaque handle; free with `aviz_destroy`.
#[unsafe(no_mangle)]
pub extern "C" fn aviz_create(name: *const c_char, cols: u16, rows: u16) -> *mut Handle {
    let name_str = unsafe { CStr::from_ptr(name) }.to_str().unwrap_or("");
    let size = TermSize { cols, rows };
    let mut viz = make_viz(name_str);
    viz.on_resize(size);
    let name_buf = to_cstring(viz.name().to_string());
    Box::into_raw(Box::new(Handle {
        viz,
        size,
        name_buf,
        render_buf: to_cstring("[]".to_string()),
        config_buf: to_cstring("{}".to_string()),
    }))
}

/// Free a handle returned by `aviz_create`.
#[unsafe(no_mangle)]
pub extern "C" fn aviz_destroy(handle: *mut Handle) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle)); }
    }
}

/// Update the character-grid dimensions.
#[unsafe(no_mangle)]
pub extern "C" fn aviz_resize(handle: *mut Handle, cols: u16, rows: u16) {
    let h = unsafe { &mut *handle };
    h.size = TermSize { cols, rows };
    h.viz.on_resize(h.size);
}

/// Advance the visualizer by one frame.
///
/// - `fft` / `fft_len`: magnitude spectrum (FFT_SIZE/2+1 floats, linear scale)
/// - `left` / `right` / `pcm_len`: raw PCM samples per channel (FFT_SIZE floats each)
/// - `dt`: elapsed seconds since the last tick
/// - `sample_rate`: audio sample rate negotiated with the hardware (e.g. 44100)
#[unsafe(no_mangle)]
pub extern "C" fn aviz_tick(
    handle:      *mut Handle,
    fft:         *const f32, fft_len: usize,
    left:        *const f32,
    right:       *const f32, pcm_len: usize,
    dt:          f32,
    sample_rate: u32,
) {
    let h = unsafe { &mut *handle };

    let fft_slice   = unsafe { std::slice::from_raw_parts(fft,   fft_len) };
    let left_slice  = unsafe { std::slice::from_raw_parts(left,  pcm_len) };
    let right_slice = unsafe { std::slice::from_raw_parts(right, pcm_len) };

    let mut l: Vec<f32> = left_slice.to_vec();
    let mut r: Vec<f32> = right_slice.to_vec();
    let mut m: Vec<f32> = l.iter().zip(r.iter()).map(|(a, b)| (a + b) * 0.5).collect();
    l.resize(FFT_SIZE, 0.0);
    r.resize(FFT_SIZE, 0.0);
    m.resize(FFT_SIZE, 0.0);

    let frame = AudioFrame {
        left:        l,
        right:       r,
        mono:        m,
        fft:         fft_slice.to_vec(),
        sample_rate,
    };
    h.viz.tick(&frame, dt, h.size);
}

/// Render the current frame and return a JSON array of cell objects:
/// `[{"ch":"█","col":3,"row":7,"r":255,"g":64,"b":0,"bold":true,"dim":false}, …]`
///
/// The returned pointer is valid until the next call to `aviz_render` on this handle.
/// Copy the string before calling again.
#[unsafe(no_mangle)]
pub extern "C" fn aviz_render(handle: *mut Handle, fps: f32) -> *const c_char {
    let h = unsafe { &mut *handle };
    let lines = h.viz.render(h.size, fps);
    let cells = parse_frame(&lines);
    h.render_buf = to_cstring(cells_to_json(&cells));
    h.render_buf.as_ptr()
}

/// Return the active visualizer's name (e.g. `"spectrum"`).
/// The pointer is stable for the lifetime of the handle.
#[unsafe(no_mangle)]
pub extern "C" fn aviz_name(handle: *const Handle) -> *const c_char {
    let h = unsafe { &*handle };
    h.name_buf.as_ptr()
}

/// Return the default config JSON for the active visualizer.
/// Pointer valid until the next call to `aviz_get_config` or `aviz_set_config`.
#[unsafe(no_mangle)]
pub extern "C" fn aviz_get_config(handle: *mut Handle) -> *const c_char {
    let h = unsafe { &mut *handle };
    h.config_buf = to_cstring(h.viz.get_default_config());
    h.config_buf.as_ptr()
}

/// Apply a (possibly partial) config JSON and return the merged result.
/// Pointer valid until the next call to `aviz_get_config` or `aviz_set_config`.
#[unsafe(no_mangle)]
pub extern "C" fn aviz_set_config(handle: *mut Handle, json: *const c_char) -> *const c_char {
    let h = unsafe { &mut *handle };
    let json_str = unsafe { CStr::from_ptr(json) }.to_str().unwrap_or("{}");
    let result = h.viz.set_config(json_str).unwrap_or_default();
    h.config_buf = to_cstring(result);
    h.config_buf.as_ptr()
}

/// Return a JSON array of all visualizer descriptors:
/// `[{"name":"spectrum","description":"…"}, …]`
///
/// Computed once on first call; the pointer is valid for the lifetime of the process.
#[unsafe(no_mangle)]
pub extern "C" fn aviz_list_visualizers() -> *const c_char {
    static CACHE: OnceLock<CString> = OnceLock::new();
    let s = CACHE.get_or_init(|| {
        let vizs = visualizers::all_visualizers();
        let mut out = String::from("[");
        for (i, v) in vizs.iter().enumerate() {
            if i > 0 { out.push(','); }
            out.push_str(&format!(
                r#"{{"name":"{}","description":"{}"}}"#,
                json_escape(v.name()),
                json_escape(v.description()),
            ));
        }
        out.push(']');
        to_cstring(out)
    });
    s.as_ptr()
}

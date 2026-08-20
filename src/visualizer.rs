/// visualizer.rs — The Visualizer trait and all shared data types.
///
/// This file is intentionally kept free of application logic.  Its only job
/// is to define the stable interface between the core engine (app.rs / gpu/)
/// and individual visualizers (src/visualizers/<category>/*.rs).
///
/// A visualizer renders in one of two modes:
///
///   RenderMode::Software — the visualizer draws into a CPU-side RGBA8
///     `Framebuffer` each frame.  The engine uploads it as a texture and
///     blits it to the window.
///
///   RenderMode::Shader — the visualizer supplies a WGSL *fragment shader*
///     (usually via include_str! of a sibling .wgsl file) plus per-frame
///     uniform parameters.  The engine prepends a common prelude (uniforms,
///     audio-data texture, previous-frame feedback texture, fullscreen
///     vertex shader) and runs it on the GPU.  See src/gpu/prelude.wgsl for
///     the exact binding contract.

// ── Shared constants ──────────────────────────────────────────────────────────

/// Audio sample rate used throughout the application.
pub const SAMPLE_RATE: u32 = 44_100;

/// FFT window size.  Must be a power of two for rustfft efficiency.
pub const FFT_SIZE: usize = 4_096;

/// Number of audio channels captured (stereo).
pub const CHANNELS: usize = 2;

/// Width of the GPU audio texture: one texel per sample / FFT bin.
/// Rows: 0 = left PCM, 1 = right PCM, 2 = mono PCM, 3 = FFT magnitudes.
pub const AUDIO_TEX_WIDTH: usize = FFT_SIZE;

// ── Pixel size ────────────────────────────────────────────────────────────────

/// Render-target size in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

// ── Audio frame ───────────────────────────────────────────────────────────────

/// One frame of analysed audio, passed to `tick()` every frame.
///
/// `left` / `right` / `mono` are sliding windows of the most recent
/// FFT_SIZE PCM samples.  `fft` is the magnitude spectrum of `mono`
/// (FFT_SIZE / 2 + 1 bins).
pub struct AudioFrame {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub mono: Vec<f32>,
    pub fft: Vec<f32>,
    #[allow(dead_code)] // part of the stable visualizer-facing API
    pub sample_rate: u32,
}

// ── CPU framebuffer ───────────────────────────────────────────────────────────

/// A CPU-side RGBA8 pixel buffer used by software visualizers.
/// Owned by the visualizer so state (e.g. scroll history) persists
/// across frames without re-rendering everything.
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, `width * height * 4` bytes.
    pub data: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, data: vec![0; (width * height * 4) as usize] }
    }

    /// Resize the buffer if needed, clearing to black. Returns true if resized.
    pub fn ensure_size(&mut self, size: PixelSize) -> bool {
        if self.width != size.width || self.height != size.height {
            self.width = size.width;
            self.height = size.height;
            self.data = vec![0; (size.width * size.height * 4) as usize];
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn put(&mut self, x: u32, y: u32, rgb: [u8; 3]) {
        if x < self.width && y < self.height {
            let i = ((y * self.width + x) * 4) as usize;
            self.data[i] = rgb[0];
            self.data[i + 1] = rgb[1];
            self.data[i + 2] = rgb[2];
            self.data[i + 3] = 255;
        }
    }

    /// Clear the whole buffer to opaque black in one memset.
    #[inline]
    pub fn clear(&mut self) {
        self.data.fill(0);
    }

    /// Fill an axis-aligned rectangle with a solid colour.  Writes each row as
    /// one contiguous span (no per-pixel bounds checks), so this is much
    /// cheaper than calling `put()` in a loop for large fills.
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3]) {
        let x0 = x.min(self.width);
        let y0 = y.min(self.height);
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let px = [rgb[0], rgb[1], rgb[2], 255];
        let stride = (self.width * 4) as usize;
        for row in y0..y1 {
            let base = row as usize * stride;
            let start = base + x0 as usize * 4;
            let end = base + x1 as usize * 4;
            for cell in self.data[start..end].chunks_exact_mut(4) {
                cell.copy_from_slice(&px);
            }
        }
    }

    /// Move all pixel rows down by `n` rows (newest content goes on top).
    /// Rows scrolled off the bottom are discarded; the top `n` rows are
    /// left with their previous content and should be overwritten.
    pub fn scroll_down(&mut self, n: u32) {
        let n = n.min(self.height);
        if n == 0 || self.height == 0 {
            return;
        }
        let row_bytes = (self.width * 4) as usize;
        let move_rows = (self.height - n) as usize;
        let src_end = move_rows * row_bytes;
        self.data.copy_within(0..src_end, (n as usize) * row_bytes);
    }
}

// ── Render mode ───────────────────────────────────────────────────────────────

/// How a visualizer produces its frames.
pub enum RenderMode {
    /// CPU rendering into a `Framebuffer` via `render_software()`.
    Software,
    /// GPU rendering: `fragment_wgsl` is compiled once (with the engine
    /// prelude prepended) and driven by `shader_params()` each frame.
    Shader { fragment_wgsl: &'static str },
}

// ── The core trait ────────────────────────────────────────────────────────────

pub trait Visualizer: Send {
    fn name(&self) -> &str;
    fn description(&self) -> &str;

    /// Rendering mode.  Must be constant for the lifetime of the instance —
    /// the engine reads it once when the visualizer is activated.
    fn mode(&self) -> RenderMode;

    /// Advance internal state by `dt` seconds using the latest audio.
    /// `size` is the current render-target size in physical pixels.
    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: PixelSize);

    /// Software mode only: return the framebuffer to display this frame.
    /// The visualizer owns the buffer and is responsible for sizing it to
    /// `size` (see `Framebuffer::ensure_size`).
    fn render_software(&mut self, _size: PixelSize) -> Option<&Framebuffer> {
        None
    }

    /// Shader mode only: 16 free-form f32 parameters delivered to the
    /// shader as `u.params` (four vec4s).  Meaning is defined per shader.
    fn shader_params(&self) -> [f32; 16] {
        [0.0; 16]
    }

    fn on_resize(&mut self, _size: PixelSize) {}

    // ── Runtime configuration interface ──────────────────────────────────────
    // Identical JSON schema to the original terminal app:
    //
    // {
    //   "visualizer_name": "spectrogram",
    //   "version": 1,
    //   "config": [
    //     { "name": "gain", "display_name": "Gain", "type": "float",
    //       "value": 1.0, "min": 0.0, "max": 4.0 },
    //     { "name": "style", "display_name": "Style", "type": "enum",
    //       "value": "solid", "variants": ["solid", "dotted"] }
    //   ]
    // }
    //
    // Types: "float", "int", "enum", "bool".

    /// Return the default (reference) configuration as a JSON string.
    /// Implementations must never read instance state — every call returns
    /// the same schema regardless of the current configuration values.
    fn get_default_config(&self) -> String;

    /// Apply a (possibly partial) JSON configuration string, merged against
    /// `get_default_config()`.  On success returns the complete, cleaned
    /// JSON suitable for persisting to disk.
    fn set_config(&mut self, json: &str) -> Result<String, String>;
}

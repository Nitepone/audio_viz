# audio-viz

A windowed 2D/3D audio visualizer written in Rust. Visualizers render either
on the GPU (WGSL fragment shaders via **wgpu**) or in software (CPU RGBA
framebuffers), presented in a **winit** window. All wgpu backends are enabled:
Metal on macOS, Vulkan/DX12 on Windows, Vulkan/GL on Linux.

The legacy terminal (ASCII/ANSI) app lives in `audio-viz-tui/` — it is its own
Cargo workspace and builds independently from inside that directory. Do not
edit it as part of new-app work.

## UI layer (src/ui.rs)

**egui** (`egui-wgpu` + `egui-winit`) draws pop-out side panels over the wgpu
frame: left = visualizer picker (grouped by category), right = settings.
Panels never resize the window — they claim width and the app passes the
remaining central rect to `GpuContext::set_viz_rect()`, shrinking the render
pane. The settings panel is **generated from the visualizer's config JSON
schema** (float/int → slider, enum → dropdown, bool → checkbox) — never write
per-visualizer UI code. Changes go through `viz.set_config()` and persist via
`config::write_config()` immediately.

Per-frame contract (`app.rs::redraw`): `ui.frame()` (build + collect
`UiAction`s) → apply actions → `gpu.set_viz_rect(ui.central_px(..))` → tick +
render visualizer → `ui.paint()` into the same encoder → `gpu.end_frame()`.
Window events go to `ui.on_window_event()` first; app shortcuts only run when
egui didn't consume the event.

Keys: `q`/`Esc` quit · `f` fullscreen · `v` visualizer panel · `s` settings panel.

## Architecture

```
src/
├── main.rs            CLI (clap): [VISUALIZER] --device --fps --list --list-devices
├── app.rs             winit ApplicationHandler; per-frame: drain audio → FFT →
│                      beat detect → viz.tick() → render (software or shader path)
├── audio.rs           cpal capture thread + ring buffer + FftEngine
├── beat.rs            BeatDetector (sub-band spectral flux) — per-viz instances
├── config.rs          config_path / merge_config / load_and_apply_config
├── dsp.rs             rms, freq_to_bin, band_energy, mag_to_frac, smoothing
├── palette.rs         RGB gradient palettes (heat/ice/spectrum/mono/phosphor)
├── ui.rs              egui overlay: left visualizer-picker panel, right
│                      settings panel (generated from config JSON schemas)
├── fx.rs              post-effect registry (EFFECTS table) + FxViz wrapper
│                      that appends fx toggles/sliders to every viz's schema
├── term/              terminal render layer (TUI → windowed bridge)
│   ├── mod.rs         TermViz adapter: TUI trait → software Visualizer;
│   │                  injects Font / Font Size / Show Status Bar settings
│   ├── ansi.rs        SGR escape parser → character-cell grid
│   └── font.rs        fontdue glyph cache over embedded assets/fonts/;
│                      block/shade/braille chars drawn geometrically
├── tui/               vendored TUI compat runtime (from audio-viz-tui/src/)
│   ├── visualizer.rs        TUI Visualizer trait, TermSize, status_bar, …
│   └── visualizer_utils.rs  TUI DSP helpers + ANSI palettes
├── gpu/
│   ├── mod.rs         GpuContext: surface, uniform buffer, audio texture,
│   │                  ping-pong feedback textures, blit + post-fx pipelines
│   ├── prelude.wgsl   prepended to every shader visualizer — binding contract
│   ├── blit.wgsl      offscreen/software texture → swapchain
│   ├── fx_prelude.wgsl  prepended to every post effect — binding contract
│   └── fx/            post-effect fragment shaders: crt.wgsl
└── visualizers/
    ├── mod.rs         include!(OUT_DIR/registry.rs)
    ├── frequency/     spectrogram.rs            (software)
    ├── scopes/        classic_lissajous, polar, scope  .{rs,wgsl}   (shader)
    └── tui/           installed TUI visualizers: tempest.rs  (software)
```

## Build & Test

```bash
cargo check          # fast validation — use after every edit
cargo build --release
./target/release/audio-viz --list
./target/release/audio-viz spectrogram
```

Runtime WGSL errors panic at startup with a naga validation message — always
launch a shader visualizer once after editing its `.wgsl`.

## Adding a Visualizer

Auto-discovered at compile time — no manual registration:

1. Create `src/visualizers/<category>/<name>.rs`
2. Implement the `Visualizer` trait and export
   `pub fn register() -> Vec<Box<dyn Visualizer>>`
3. `build.rs` regenerates the registry on the next build

### The Visualizer trait (src/visualizer.rs)

```rust
pub trait Visualizer: Send {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn mode(&self) -> RenderMode;                 // Software | Shader { fragment_wgsl }
    fn tick(&mut self, audio: &AudioFrame, dt: f32, size: PixelSize);
    fn render_software(&mut self, size: PixelSize) -> Option<&Framebuffer> { None }
    fn shader_params(&self) -> [f32; 16] { [0.0; 16] }
    fn on_resize(&mut self, _size: PixelSize) {}
    fn get_default_config(&self) -> String;       // JSON schema string
    fn set_config(&mut self, json: &str) -> Result<String, String>;
}
```

- **Software** visualizers own a persistent `Framebuffer` (RGBA8) and return it
  from `render_software()`; size it with `fb.ensure_size(size)`. `Framebuffer`
  has `put()` and `scroll_down()` helpers.
- **Shader** visualizers put their fragment WGSL in a sibling `.wgsl` file,
  pulled in with `include_str!`, and pack per-frame uniform params in
  `shader_params()` (delivered as `u.params`, four vec4s).
- Config JSON schema is identical to the terminal app (`float`, `int`, `enum`,
  `bool` entries; merge via `config::merge_config`).

### Installing a TUI visualizer (src/visualizers/tui/)

Visualizers from the legacy terminal app run unmodified through the terminal
render layer (`src/term/`): the `TermViz` adapter sizes a virtual terminal
from the pane's pixel size and the configured font, ticks the TUI visualizer,
parses its ANSI output, and rasterises the cell grid into a software
framebuffer. Font family, font size, and a status-bar toggle are appended to
the wrapped visualizer's settings automatically — never add them by hand.

To install one (see `tempest.rs` for the reference example):

1. Copy `audio-viz-tui/src/visualizers/<cat>/<name>.rs` to
   `src/visualizers/tui/<name>.rs`. The filename must equal the visualizer's
   `name()`, and must not collide with any existing visualizer.
2. Rewrite the imports — everything else stays verbatim:
   - `crate::visualizer::…` → `crate::tui::visualizer::…`
   - `crate::visualizer_utils::…` → `crate::tui::visualizer_utils::…`
   - `crate::beat::…` stays as-is (identical library)
3. Replace the `register()` body:
   ```rust
   pub fn register() -> Vec<Box<dyn crate::visualizer::Visualizer>> {
       crate::term::TermViz::adapt(vec![Box::new(MyViz::new(""))])
   }
   ```

`src/tui/` holds vendored copies of the TUI crate's `visualizer.rs` and
`visualizer_utils.rs` (import paths adjusted, otherwise in sync with
`audio-viz-tui/src/`). Fonts are embedded from `assets/fonts/` (DejaVu Sans
Mono + Fira Code, regular/bold); block, shade, and braille characters are
drawn geometrically in `src/term/font.rs` so cells tile without seams.

## Post Effects (src/fx.rs + src/gpu/fx/)

Screen-space effects (e.g. CRT) applied after the visualizer renders — they
stack in a chain that ends at the swapchain blit, with the final pass running
at display resolution inside the viz rect. They work on top of every
visualizer: shader, software, and terminal-rendered. The `FxViz` wrapper
(applied to every visualizer by app.rs) appends each effect's on/off toggle
and parameter sliders to the visualizer's settings schema, so UI and per-viz
persistence are automatic (config names: `fx_<effect>` / `fx_<effect>_<param>`).

To add an effect: create `src/gpu/fx/<name>.wgsl` (fragment only — see
`fx_prelude.wgsl` for bindings: `src` texture, `fx.resolution`, `fx.time`,
`fx.params`) and add a row to `EFFECTS` in `src/fx.rs` (max 8 params).
Nothing else: pipelines, settings UI, and persistence are table-driven.
Launch once after editing a `.wgsl` — naga errors panic at startup.

### Shader binding contract (src/gpu/prelude.wgsl)

The engine prepends the prelude; the visualizer defines only
`@fragment fn fs_main(in: VsOut) -> @location(0) vec4<f32>`.

Available: `u` (resolution, time, dt, rms, beat, sample_rate, params),
`audio_sample(row, i)` over a 4096×4 R32Float texture
(rows `ROW_LEFT`/`ROW_RIGHT`/`ROW_MONO`/`ROW_FFT`), and
`prev_pixel(uv)` — the visualizer's previous frame (Rgba16Float), enabling
phosphor persistence and feedback effects. Shader visualizers render at an
internally capped resolution (~1.6 MP) and are upscaled in the blit pass.

## Key Constants (src/visualizer.rs)

- `SAMPLE_RATE`: 44,100 Hz · `FFT_SIZE`: 4,096 · `CHANNELS`: 2
- `AUDIO_TEX_WIDTH`: 4,096 (texels per audio-texture row)

## Beat Detection (src/beat.rs)

Same library as the terminal app. The engine feeds a shared detector's
`beat_intensity()` into shader uniforms as `u.beat`; software visualizers can
own their own `BeatDetector` instance (presets: `simple()`, `standard()`,
`bass_only()`).

## Config Persistence

- macOS: `~/Library/Application Support/audio_viz/<viz>.json`
- Linux: `$XDG_CONFIG_HOME/audio_viz/` (fallback `~/.config/audio_viz/`)
- Loaded and re-cleaned at startup via `config::load_and_apply_config`.

## Roadmap

`future-plans/` contains one spec per upcoming feature (settings UI, runtime
shader loading, app icon/bundle, porting the remaining terminal visualizers,
WASM/WebGPU port, true 3D pipeline). Read the relevant spec before starting
one of those tasks.

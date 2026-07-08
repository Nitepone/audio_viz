# audio-viz

A windowed 2D/3D audio visualizer written in Rust. Visualizers render either
on the GPU (WGSL fragment shaders via wgpu — Metal on macOS, Vulkan/DX12/GL
elsewhere) or in software (CPU pixel buffers), inside a native window.

This is the successor to the terminal ASCII visualizer, which lives unchanged
in [`audio-viz-tui/`](audio-viz-tui/) and still builds from that directory.

## Running

```bash
cargo build --release
./target/release/audio-viz [VISUALIZER] [OPTIONS]

./target/release/audio-viz --list           # list visualizers
./target/release/audio-viz --list-devices   # list audio input devices
./target/release/audio-viz scope --device blackhole
```

Keys: `q` / `Esc` quit · `f` fullscreen · `v` visualizer panel · `s` settings panel.

Pop-out side panels (buttons in the top corners, or the `v`/`s` keys) let you
switch visualizers and edit settings with the mouse; they shrink the render
pane rather than resizing the window. Settings apply live and persist to the
same per-visualizer JSON files as the terminal app.

To visualize system audio on macOS, install
[BlackHole](https://existential.audio/blackhole/) and route output through it;
audio-viz picks up loopback devices automatically.

## Visualizers

| name              | mode     | description                                    |
|-------------------|----------|------------------------------------------------|
| `spectrogram`       | software | scrolling spectrogram, frequency vs time      |
| `classic_lissajous` | shader   | XY phosphor oscilloscope with CRT persistence |
| `scope`             | shader   | dual-channel time-domain oscilloscope         |

Per-visualizer settings are JSON files under the platform config directory
(`~/Library/Application Support/audio_viz/` on macOS) — the same schema as the
terminal app. The in-app settings panel is generated from that schema.

## Architecture

```
src/
├── main.rs            CLI entry point
├── app.rs             winit event loop + per-frame pipeline
├── ui.rs              egui side panels (picker + settings)
├── audio.rs           cpal capture thread, ring buffer, FFT
├── beat.rs            shared beat-detection library
├── config.rs          config persistence + JSON merge
├── dsp.rs, palette.rs shared DSP helpers and RGB palettes
├── gpu/               wgpu engine: uniforms, audio texture, feedback, blit
│   ├── prelude.wgsl   the binding contract for shader visualizers
│   └── blit.wgsl
└── visualizers/       auto-discovered at compile time (build.rs)
    ├── frequency/spectrogram.rs
    └── scopes/{classic_lissajous,scope}.{rs,wgsl}
```

A visualizer is either **software** (owns a persistent RGBA framebuffer the
engine uploads and blits) or **shader** (a WGSL fragment shader compiled
against `src/gpu/prelude.wgsl`, receiving audio as a texture, 16 free-form
uniform params, and its own previous frame for feedback/persistence effects).

See `CLAUDE.md` for the full development guide and `future-plans/` for the
roadmap (settings UI, runtime shader loading, app bundle/icon, WASM port, …).

# WASM / browser port

## Goal

Run audio-viz in the browser, replacing the old ANSI-parsing web frontend
(`audio-viz-tui/web/`). Rendering via WebGPU (wgpu compiles to it directly),
audio via the Web Audio API.

## Current state

- Not started. The native crate uses cpal (no wasm support) and
  `pollster::block_on` (not available on wasm) — both need cfg-gated
  alternatives.
- wgpu + winit both support wasm32 targets; shader visualizers should work
  unchanged. The old web frontend has reusable audio capture plumbing
  (`audio-viz-tui/web/audio.js`, `processor.worklet.js`: mic +
  getDisplayMedia system-audio capture).

## Suggested approach

1. Split the crate: keep visualizers + gpu + dsp in a lib free of cpal/clap;
   native bin and wasm entry (`wasm-bindgen`) both consume it.
2. wasm audio: AudioWorklet posts PCM into a ring buffer shared with Rust
   (either via JS→wasm calls per chunk, or SharedArrayBuffer).
3. Async init: `GpuContext::new_async` already exists; on wasm call it with
   `wasm_bindgen_futures::spawn_local` instead of pollster.
4. Canvas sizing / device-pixel-ratio handling in place of window resize.
5. Fall back to WebGL2 via wgpu's `webgl` feature for browsers without WebGPU
   (note: R32Float texture filtering rules differ; we only textureLoad, so OK).
6. CI deploy to GitHub Pages like the old project did.

## Acceptance

- All shader visualizers plus the spectrogram run in Chrome (WebGPU) from a
  static page with mic input; the egui panel UI (src/ui.rs) works — egui,
  egui-wgpu and egui-winit all support wasm32.

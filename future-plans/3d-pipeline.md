# True 3D visualizer pipeline

## Goal

Support real 3D visualizers — audio-driven meshes, particle fields, terrain
flyovers — not just fullscreen fragment shaders. Example targets: a 3D
spectrum terrain scrolling into the distance; an FFT-driven particle galaxy.

## Current state

- `RenderMode::Shader` runs a fullscreen triangle with a fragment shader;
  there is no depth buffer, no vertex data, no camera.
- `gpu/mod.rs` is small and single-purpose; adding a third path is
  straightforward.

## Suggested approach

1. Add `RenderMode::Scene` (or a `SceneVisualizer` sub-trait) where the
   visualizer supplies vertex/index buffers (or instance data) and its own
   vertex+fragment WGSL, and receives a camera uniform (view-projection
   matrix) from the engine.
2. Engine additions: depth texture (Depth32Float) sized with the surface,
   a shared camera struct (orbit camera with mouse drag + scroll zoom in
   app.rs), and a per-scene-viz pipeline cache.
3. Keep the audio texture + `u` uniforms binding identical to the 2D contract
   so DSP access is uniform across modes.
4. First implementation: `frequency/terrain3d` — a heightfield grid where new
   FFT rows push in from the far edge (the 3D sibling of the spectrogram).
   The mesh can be a static grid displaced in the vertex shader from the
   audio texture — no per-frame buffer uploads needed beyond what exists.

## Acceptance

- terrain3d runs at 60 fps with mouse-orbit camera, correct depth, resize-safe.
- The 2D shader and software paths are unaffected.

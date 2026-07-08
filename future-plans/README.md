# future-plans/

One file per planned feature, written as a self-contained brief for an agent
(or human) picking the task up cold. Each spec describes the goal, the current
state of the relevant code, a suggested approach, and acceptance criteria.

Suggested order (dependencies noted inside each spec):

1. [app-icon-and-bundle.md](app-icon-and-bundle.md) — icon + macOS .app bundle
2. [runtime-shader-loading.md](runtime-shader-loading.md) — drop-in .wgsl visualizers
3. [port-remaining-visualizers.md](port-remaining-visualizers.md) — the other 18 terminal visualizers
4. [3d-pipeline.md](3d-pipeline.md) — true 3D (vertex/mesh) visualizers
5. [software-only-fallback.md](software-only-fallback.md) — run without any GPU
6. [wasm-port.md](wasm-port.md) — browser build via WebGPU
7. [ci-and-releases.md](ci-and-releases.md) — cross-platform CI

Already implemented (removed from this folder): mouse-driven settings panel
and in-app visualizer picker — both live in `src/ui.rs` (egui side panels).

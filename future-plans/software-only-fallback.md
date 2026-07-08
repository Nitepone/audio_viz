# Software-only presentation fallback

## Goal

Run on machines with no usable GPU (VMs, old drivers, headless-ish setups):
if wgpu can't provide an adapter, present software visualizers via a plain
CPU swapchain instead of failing.

## Current state

- `GpuContext::new` errors out when `request_adapter` returns nothing, and
  even software visualizers are presented through a wgpu blit.
- Software visualizers themselves are already GPU-free (they produce plain
  RGBA buffers).

## Suggested approach

1. Try wgpu first, including `force_fallback_adapter` (llvmpipe/SwiftShader)
   before giving up.
2. If no adapter: fall back to the `softbuffer` crate (winit-compatible CPU
   presentation) and restrict `--list`/selection to `RenderMode::Software`
   visualizers with a clear message for shader ones.
3. Abstract presentation behind a small enum (`Presenter::Wgpu(GpuContext) |
   Presenter::Soft(SoftbufferState)`) in app.rs rather than threading trait
   objects through everything.

## Acceptance

- With GPU: behaviour unchanged.
- With `WGPU_BACKEND=noop` (or on a GPU-less VM): spectrogram still runs;
  selecting a shader visualizer prints an actionable error instead of crashing.

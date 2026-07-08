# Runtime shader loading (drop-in .wgsl visualizers)

## Goal

Let users add shader visualizers without recompiling: any `.wgsl` file dropped
into a user directory (e.g. `~/Library/Application Support/audio_viz/shaders/`
on macOS) appears as a selectable visualizer. Stretch: hot-reload on file save
for live shader development.

## Current state

The architecture was designed for this:

- A shader visualizer is *only* a WGSL fragment shader against the binding
  contract in `src/gpu/prelude.wgsl` (uniforms `u`, `audio_sample()`,
  `prev_pixel()`, `fs_main`). Built-ins embed theirs with `include_str!`;
  nothing about the pipeline requires the source to be static —
  `GpuContext::set_shader()` already takes `&str` at runtime.
- The 16 `u.params` floats are packed by each Rust wrapper from its config.

## Suggested approach

1. Define a metadata convention inside the WGSL file — a leading comment block
   with JSON mapping config fields to param slots:
   `//! { "name": "plasma", "params": [{"name":"gain","slot":0,"type":"float","min":0,"max":4,"value":1}] }`
2. Add a generic `UserShaderViz` implementing `Visualizer`: parses the header,
   generates the config schema from it, packs `shader_params()` accordingly.
3. Scan the shaders directory at startup; append discovered shaders to
   `all_visualizers()` output (namespace as `user/<name>` on collision).
4. Compile user WGSL with `device.push_error_scope` /
   `create_shader_module` error handling so a broken shader shows an error
   (log + on-screen message) instead of panicking. This is the one engine
   change needed: `build_shader_state` currently assumes valid WGSL.
5. Hot reload: watch file mtimes (`notify` crate or a 0.5 s poll) and re-run
   `set_shader()` on change.

## Acceptance

- Copying a valid .wgsl into the folder makes it appear in `--list` and run.
- A file with WGSL errors degrades gracefully (message, not crash), and
  fixing + saving it recovers without restart (if hot reload is included).

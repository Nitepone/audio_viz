# Port the remaining terminal visualizers

## Goal

Recreate the other terminal visualizers as native pixel/GPU visualizers. The
originals (ASCII renderers) are in `audio-viz-tui/src/visualizers/` and remain
the reference for behaviour, config schemas, and DSP.

## Inventory (audio-viz-tui/src/visualizers/)

- **frequency/**: spectrum (bar analyzer), radial, vu
- **scopes/**: lissajous (the "modern" variant), polar
- **effects/**: aurora, fire, matrix, missiles, night_sky, plasma, ripple,
  tempest, tunnel
- **abstract/**: attractor, crystal, orbit, pulsar

## Guidance

- Choose per visualizer: continuous-field looks (plasma, aurora, tunnel,
  ripple) fit fragment shaders naturally; stateful particle/cell systems
  (matrix, fire, missiles, night_sky) are easiest as software renders first —
  the CPU cost is fine at window resolution.
- Port each original's config schema verbatim where it still makes sense
  (drop terminal-isms like character sets; translate ANSI palettes to RGB via
  `src/palette.rs` — add gradients as needed).
- DSP building blocks already ported: `src/dsp.rs`, `src/beat.rs`,
  `SpectrumBars`-style smoothing lives only in the old
  `audio-viz-tui/src/visualizer.rs` (`build_binmap`, `spec_to_bars`,
  `SpectrumBars`) — port that into `src/dsp.rs` when doing `spectrum`.
- One visualizer per PR/commit; follow the trait contract in `CLAUDE.md`.

## Acceptance (per visualizer)

- Appears in `--list` under the right category; config round-trips; visually
  comparable to (or better than) the terminal original at 60 fps.

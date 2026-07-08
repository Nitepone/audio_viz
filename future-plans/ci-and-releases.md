# CI and release builds

## Goal

GitHub Actions pipeline: check/build on Linux, macOS (x86_64 + ARM), and
Windows; produce release artifacts on tags (including the macOS .app bundle
once app-icon-and-bundle.md lands).

## Current state

- No workflow at the repo root. The old pipeline
  (`audio-viz-tui/.github/workflows/build.yml`) is a good template for the
  matrix + artifact upload, but targeted the terminal binary and WASM.

## Suggested approach

1. New `.github/workflows/build.yml`: `cargo check` + `cargo build --release`
   matrix (ubuntu, macos-13, macos-latest, windows). Linux needs
   `libasound2-dev` (ALSA headers for cpal) and — for winit — no display, so
   build only; don't run the app.
2. Keep the tui project out of CI (or a separate optional job) — it builds
   from `audio-viz-tui/`.
3. Tag pushes: upload per-platform binaries as release assets.
4. Add `cargo fmt --check` + `cargo clippy -- -D warnings` once the codebase
   is formatted/clippy-clean.

## Acceptance

- PRs get green/red status from all four targets; tagging `v*` publishes
  downloadable binaries.

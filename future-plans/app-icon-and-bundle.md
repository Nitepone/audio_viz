# App icon and macOS bundle

## Goal

Ship audio-viz as a double-clickable app with an icon: a macOS `.app` bundle
(and window icons on Windows/Linux, where winit supports setting them
directly).

## Current state

- Plain CLI binary; no icon assets exist. On macOS the Dock shows the default
  executable icon and the window has no custom icon (macOS takes the icon from
  the app bundle, not from winit).

## Suggested approach

1. Design/generate an icon (e.g. a stylised oscilloscope trace), export a
   1024×1024 PNG into `assets/icon.png`, derive `icon.icns` (macOS) and
   `icon.ico` (Windows) via `iconutil` / `cargo-packager` tooling.
2. macOS: use `cargo-bundle` or `cargo-packager` to produce
   `audio-viz.app` with `Info.plist` (CFBundleName, NSMicrophoneUsageDescription —
   required, cpal requests mic access) and the icns.
3. Windows/Linux: call `window.set_window_icon()` in `src/app.rs::resumed`
   (decode the PNG with the `image` crate, behind `#[cfg(not(target_os = "macos"))]`).
4. Wire bundle creation into CI (see ci-and-releases.md).

## Acceptance

- `open audio-viz.app` launches with the icon in the Dock and mic permission
  prompt showing the proper app name.
- CLI usage (`./audio-viz scope`) keeps working unchanged.

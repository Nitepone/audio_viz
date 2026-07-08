/// tui/ — Compatibility runtime for legacy terminal (TUI) visualizers.
///
/// These modules are vendored copies of the corresponding files in
/// `audio-viz-tui/src/` so that TUI visualizer sources can be "installed"
/// into `src/visualizers/tui/` and compile unmodified apart from their
/// import paths:
///
///   crate::visualizer        →  crate::tui::visualizer
///   crate::visualizer_utils  →  crate::tui::visualizer_utils
///   crate::beat              →  crate::beat            (already identical)
///
/// The rendering side of the bridge lives in `src/term/`, which adapts the
/// TUI `Visualizer` trait (ANSI strings over a character grid) onto the
/// windowed app's software-framebuffer `Visualizer` trait.
// Both modules are a library surface for installed visualizers — only the
// parts the currently installed ones use are exercised.
#[allow(dead_code)]
pub mod visualizer;
#[allow(dead_code)]
pub mod visualizer_utils;

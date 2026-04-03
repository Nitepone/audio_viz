import SwiftUI

// ── VisualizerView ────────────────────────────────────────────────────────────
//
// Fullscreen canvas that renders one audio_viz frame per display tick.
//
// Render loop
// ───────────
// SwiftUI's TimelineView(.animation) fires at the display's refresh rate
// (typically 60 fps on Apple TV).  On each tick we call bridge.render() which
// returns the JSON cell array from the Rust core, decode it via CellRenderer,
// and draw each character with the correct RGB colour.
//
// Thread safety
// ─────────────
// bridge.render() is guarded by RustBridge's internal NSLock, so it is safe
// to call from the main thread while tick() fires on the audio thread.
//
// Idle state
// ──────────
// When `bridge` is nil (engine not started) a black screen is shown.
// The overlay UI (playback controls, track info) is layered on top by the
// parent NowPlayingView in Phase 6; this view is purely the canvas.

struct VisualizerView: View {

    /// The active Rust visualizer bridge.  May be nil before audio starts.
    let bridge: RustBridge?

    var body: some View {
        if let bridge {
            TimelineView(.animation(minimumInterval: 1.0 / 45.0)) { _ in
                VisualizerCanvas(bridge: bridge)
            }
        } else {
            Color.black
        }
    }
}

// ── VisualizerCanvas ──────────────────────────────────────────────────────────
//
// Separated from VisualizerView so TimelineView only re-renders the canvas
// and not any surrounding layout that changes for other reasons.

private struct VisualizerCanvas: View {

    let bridge: RustBridge

    var body: some View {
        Canvas { ctx, size in
            // Fill background
            ctx.fill(
                Path(CGRect(origin: .zero, size: size)),
                with: .color(.black)
            )

            // Render frame from Rust
            let json  = bridge.render(fps: 45.0)
            let cells = CellRenderer.decode(json: json)
            CellRenderer.draw(cells: cells, in: &ctx, size: size)
        }
        .ignoresSafeArea()
        .background(Color.black)
    }
}

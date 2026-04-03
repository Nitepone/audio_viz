import SwiftUI

// ── ContentView ───────────────────────────────────────────────────────────────
//
// Temporary scaffold for Phases 2–5 — proves all subsystems wire together.
//
//  • VisualizerView renders live behind a simple control overlay
//  • Start/Stop toggles the audio engine
//  • "Play Test Tone" injects a 440 Hz sine to verify the tap fires
//  • The visualizer list confirms Rust FFI is live
//
// Phase 6 replaces this with the full NowPlaying / Library navigation stack.

struct ContentView: View {
    @StateObject private var appState = AppState()

    var body: some View {
        ZStack {
            // ── Visualizer canvas ─────────────────────────────────────────────
            VisualizerView(bridge: appState.bridge)
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            // ── Overlay ───────────────────────────────────────────────────────
            VStack {
                Spacer()

                VStack(spacing: 20) {

                    // Status
                    if appState.isRunning {
                        Text(appState.currentVizName)
                            .font(.headline)
                            .foregroundStyle(.white)
                    }

                    // Controls
                    HStack(spacing: 40) {
                        Button(appState.isRunning ? "Stop" : "Start Audio") {
                            if appState.isRunning { appState.stop() }
                            else { appState.start() }
                        }

                        if appState.isRunning {
                            Button("Test Tone") { appState.playTestTone() }
                        }
                    }
                    .buttonStyle(.card)

                    // Visualizer picker (quick test — full carousel in Phase 6)
                    if appState.isRunning && !appState.availableVisualizers.isEmpty {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 20) {
                                ForEach(appState.availableVisualizers) { viz in
                                    Button(viz.name) {
                                        appState.switchVisualizer(to: viz.name)
                                    }
                                    .buttonStyle(.card)
                                    .opacity(viz.name == appState.currentVizName ? 1.0 : 0.5)
                                }
                            }
                            .padding(.horizontal, 40)
                        }
                    }
                }
                .padding(40)
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 16))
                .padding(60)
            }
        }
        .background(Color.black)
        .ignoresSafeArea()
    }
}

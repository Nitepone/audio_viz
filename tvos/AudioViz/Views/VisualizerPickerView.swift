import SwiftUI

// ── VisualizerPickerView ──────────────────────────────────────────────────────
//
// Horizontal carousel of all compiled-in visualizers.
// Selecting a card switches the active visualizer immediately and dismisses.

struct VisualizerPickerView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 48) {
            Text("Choose Visualizer")
                .font(.largeTitle.bold())

            if appState.availableVisualizers.isEmpty {
                ContentUnavailableView(
                    "No visualizers found",
                    systemImage: "waveform.slash",
                    description: Text("Make sure the Rust library was compiled with visualizers.")
                )
            } else {
                ScrollView(.horizontal, showsIndicators: false) {
                    LazyHStack(spacing: 40) {
                        ForEach(appState.availableVisualizers) { viz in
                            VisualizerCard(
                                viz: viz,
                                isActive: viz.name == appState.currentVizName
                            )
                            .onTapGesture {
                                appState.switchVisualizer(to: viz.name)
                                dismiss()
                            }
                        }
                    }
                    .padding(.horizontal, 80)
                    .padding(.vertical, 20)
                }
            }
        }
        .padding(.top, 60)
        .padding(.bottom, 60)
    }
}

// MARK: - Card

private struct VisualizerCard: View {
    let viz:      VisualizerInfo
    let isActive: Bool

    @FocusState private var isFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            // Preview tile — filled with accent colour when active
            ZStack {
                RoundedRectangle(cornerRadius: 14)
                    .fill(isActive
                          ? Color.accentColor
                          : Color.secondary.opacity(isFocused ? 0.35 : 0.18))
                    .frame(width: 240, height: 150)

                Image(systemName: iconName(for: viz.name))
                    .font(.system(size: 52))
                    .foregroundStyle(isActive ? .white : .secondary)

                if isActive {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.white)
                        .font(.title2)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                        .padding(10)
                }
            }

            // Labels
            Text(viz.name)
                .font(.headline)
                .foregroundStyle(isActive ? .primary : .secondary)

            Text(viz.description)
                .font(.caption)
                .foregroundStyle(.tertiary)
                .lineLimit(2)
                .frame(width: 240, alignment: .leading)
        }
        .focusable()
        .focused($isFocused)
        .scaleEffect(isFocused ? 1.05 : 1.0)
        .animation(.easeInOut(duration: 0.15), value: isFocused)
    }

    /// Map a visualizer name to a representative SF Symbol.
    private func iconName(for name: String) -> String {
        switch name.lowercased() {
        case let n where n.contains("spectrum"): return "waveform.path.ecg"
        case let n where n.contains("scope"):    return "waveform"
        case let n where n.contains("matrix"):   return "squareshape.split.2x2"
        case let n where n.contains("fire"):     return "flame.fill"
        case let n where n.contains("radial"):   return "circle.hexagongrid.fill"
        case let n where n.contains("lissajous"):return "infinity.circle"
        case let n where n.contains("vu"):       return "speaker.wave.3.fill"
        case let n where n.contains("orbit"):    return "atom"
        case let n where n.contains("pulsar"):   return "dot.radiowaves.left.and.right"
        case let n where n.contains("crystal"):  return "sparkles"
        default:                                  return "waveform.path"
        }
    }
}

import SwiftUI

// ── SettingsView ──────────────────────────────────────────────────────────────
//
// Dynamic settings form driven by the active visualizer's config JSON.
//
// • Float/Int → +/− buttons with value display and range indicator
// • Enum     → Horizontal inline buttons (not Picker — avoids blurry sheet)
// • Bool     → Toggle
//
// Changes are applied immediately via bridge.setConfig().
// "Reset to Defaults" reloads the original config from the bridge.

struct SettingsView: View {
    @EnvironmentObject var appState: AppState

    @State private var configRoot: ConfigRoot?
    @State private var items: [ConfigItem] = []
    @State private var hasChanges = false

    var body: some View {
        NavigationStack {
            Group {
                if items.isEmpty {
                    ContentUnavailableView(
                        "No settings",
                        systemImage: "slider.horizontal.below.square.filled.and.square",
                        description: Text("This visualizer has no configurable options.")
                    )
                } else {
                    Form {
                        ForEach($items) { $item in
                            ConfigRow(item: $item) {
                                applyConfig()
                            }
                            .listRowInsets(EdgeInsets(top: 12, leading: 40, bottom: 12, trailing: 40))
                        }
                        Section {
                            Button("Reset to Defaults", role: .destructive) {
                                loadConfig()
                            }
                            .listRowInsets(EdgeInsets(top: 12, leading: 40, bottom: 12, trailing: 40))
                        }
                    }
                }
            }
            .navigationTitle("\(appState.currentVizName) settings")
        }
        .onAppear { loadConfig() }
    }

    // MARK: - Config I/O

    private func loadConfig() {
        guard let bridge = appState.bridge else { return }
        let json = bridge.getConfig()
        guard
            let data = json.data(using: .utf8),
            let root = try? JSONDecoder().decode(ConfigRoot.self, from: data)
        else { return }
        configRoot = root
        items = root.config
        hasChanges = false
    }

    private func applyConfig() {
        guard let bridge = appState.bridge, var root = configRoot else { return }
        root.config = items
        guard
            let data = try? JSONEncoder().encode(root),
            let json = String(data: data, encoding: .utf8)
        else { return }
        bridge.setConfig(json)
        hasChanges = true
    }
}

// MARK: - Row views

private struct ConfigRow: View {
    @Binding var item: ConfigItem
    var onChange: () -> Void

    var body: some View {
        switch item.type {
        case "float":  floatRow
        case "int":    intRow
        case "enum":   enumRow
        case "bool":   boolRow
        default:
            LabeledContent(item.display_name) {
                Text(item.value.stringValue ?? "—").foregroundStyle(.secondary)
            }
        }
    }

    // MARK: Float
    // tvOS has neither Slider nor Stepper.  Use focusable +/− buttons instead;
    // the Siri Remote activates them with a click.

    private var floatRow: some View {
        let lo      = item.min ?? 0.0
        let hi      = max(item.max ?? 1.0, lo + 0.001)
        let step    = (hi - lo) / 20.0
        let current = item.value.doubleValue ?? lo
        let fraction = (current - lo) / (hi - lo)

        return HStack(spacing: 20) {
            Text(item.display_name)
            Spacer()

            Button {
                item.value = .double(max(lo, current - step))
                onChange()
            } label: {
                Image(systemName: "minus.circle.fill")
                    .font(.title2)
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.plain)
            .hoverEffect(.highlight)

            VStack(spacing: 4) {
                Text(String(format: "%.2f", current))
                    .monospacedDigit()
                    .frame(width: 80, alignment: .center)
                ProgressView(value: fraction)
                    .frame(width: 80)
                    .tint(.accentColor)
            }

            Button {
                item.value = .double(min(hi, current + step))
                onChange()
            } label: {
                Image(systemName: "plus.circle.fill")
                    .font(.title2)
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.plain)
            .hoverEffect(.highlight)
        }
    }

    // MARK: Int

    private var intRow: some View {
        let lo      = Int(item.min ?? 0)
        let hi      = Int(item.max ?? 100)
        let current = Int(item.value.doubleValue ?? Double(lo))
        let range   = Double(max(hi - lo, 1))
        let fraction = Double(current - lo) / range

        return HStack(spacing: 20) {
            Text(item.display_name)
            Spacer()

            Button {
                item.value = .double(Double(max(lo, current - 1)))
                onChange()
            } label: {
                Image(systemName: "minus.circle.fill")
                    .font(.title2)
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.plain)
            .hoverEffect(.highlight)

            VStack(spacing: 4) {
                Text("\(current)")
                    .monospacedDigit()
                    .frame(width: 80, alignment: .center)
                ProgressView(value: fraction)
                    .frame(width: 80)
                    .tint(.accentColor)
            }

            Button {
                item.value = .double(Double(min(hi, current + 1)))
                onChange()
            } label: {
                Image(systemName: "plus.circle.fill")
                    .font(.title2)
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.plain)
            .hoverEffect(.highlight)
        }
    }

    // MARK: Enum
    // Picker inside a tvOS Form pops a blurry navigation sheet; use inline
    // buttons instead so the selection is always visible and readable.

    private var enumRow: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(item.display_name)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 16) {
                    ForEach(item.variants ?? [], id: \.self) { variant in
                        let selected = item.value.stringValue == variant
                        Button {
                            item.value = .string(variant)
                            onChange()
                        } label: {
                            Text(variant)
                                .lineLimit(1)
                                .padding(.horizontal, 20)
                                .padding(.vertical, 10)
                                .background(
                                    selected ? Color.accentColor
                                             : Color.secondary.opacity(0.25),
                                    in: RoundedRectangle(cornerRadius: 8)
                                )
                                .foregroundStyle(selected ? Color.white : Color.primary)
                        }
                        .buttonStyle(.plain)
                        .hoverEffect(.highlight)
                    }
                }
                .padding(.horizontal, 20)
            }
        }
        .padding(.vertical, 4)
    }

    // MARK: Bool

    private var boolRow: some View {
        Toggle(
            item.display_name,
            isOn: Binding(
                get: { item.value.boolValue ?? false },
                set: { item.value = .bool($0) ; onChange() }
            )
        )
    }
}

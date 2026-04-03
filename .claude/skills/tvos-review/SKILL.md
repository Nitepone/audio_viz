---
name: tvos-review
description: Pre-build quality check for a tvOS Swift file. Verifies tvOS API compliance, focus handling, environment object passing, and config consistency.
argument-hint: <path/to/File.swift>
---

Review the tvOS Swift file at `$ARGUMENTS`. Work through each applicable item in order. Report pass/fail for each, fix any failures, then give a final summary.

## Checklist

### 1. No banned tvOS APIs
Search the file for these patterns — none should exist:
- `Slider(` or `Stepper(` — tvOS has neither; use `+/−` Button pairs
- `UITextFieldStyle.roundedBorder` — use `.plain` or `.automatic`
- `AVAudioSession.recordPermission` or microphone references — tvOS has no mic
- `UIApplication.shared.open(` — limited URL scheme support on tvOS

### 2. Focus handling
Every interactive element (Button, Toggle, etc.) should be navigable with the Siri Remote:
- Buttons should use `.buttonStyle(.plain)` + `.hoverEffect()` or `.hoverEffect(.highlight)`
- Groups of horizontally-arranged buttons should be wrapped in a `.focusSection()` container so D-pad up/down moves between groups
- If custom focus logic exists, verify `@FocusState` bindings are applied to all relevant views

### 3. Environment objects
- All views presented as `.sheet()` must pass `.environmentObject(appState)` (it does not propagate automatically through sheets)
- Verify the view declares `@EnvironmentObject var appState: AppState` if it accesses app state

### 4. Config round-trip (SettingsView only)
If reviewing SettingsView.swift:
- Verify `loadConfig()` calls `bridge.getConfig()` and decodes to `ConfigRoot`
- Verify `applyConfig()` encodes back to JSON and calls `bridge.setConfig()`
- Check that all config types (float, int, enum, bool) have corresponding row views
- Verify the Reset button calls `loadConfig()` to restore defaults

### 5. Sheet presentation
If the file presents sheets:
- Verify `@State` binding variables exist for each sheet
- Verify `.sheet(isPresented:)` passes the correct binding
- Verify dismiss logic works (sheet content should be self-contained or use `@Environment(\.dismiss)`)

### 6. Thread safety
If the file calls RustBridge methods:
- `bridge.tick()` should only be called from the audio callback (not main thread)
- `bridge.render()` should only be called from the main thread (Canvas body)
- Never store or cache the bridge reference across async boundaries without checking for nil

---

## Output format

Report each item as `✓ passed` or `✗ failed — <what was wrong and what was fixed>`. Skip items that don't apply to the file being reviewed. End with either:

> All checks passed. Ready to build.

or a summary of remaining issues if any require user input.

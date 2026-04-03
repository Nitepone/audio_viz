---
name: tvos-add-view
description: Scaffold a new SwiftUI view for the tvOS app with correct boilerplate, focus handling, and sheet wiring.
argument-hint: <ViewName>
---

Create a new SwiftUI view named `$ARGUMENTS[0]` for the tvOS audio visualizer app.

## Steps

### 1. Create the view file

Write a new file at `tvos/AudioViz/Views/$ARGUMENTS[0].swift` with this template:

```swift
import SwiftUI

struct $ARGUMENTS[0]: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            VStack {
                // TODO: Add content here
                Text("$ARGUMENTS[0]")
            }
            .navigationTitle("$ARGUMENTS[0]")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}
```

### 2. Wire up sheet presentation in NowPlayingView

Add to `NowPlayingView.swift`:

1. A `@State` binding:
   ```swift
   @State private var show$ARGUMENTS[0] = false
   ```

2. A button in the `actionRow` (or wherever appropriate):
   ```swift
   actionButton("<Label>", icon: "<sf.symbol>") { show$ARGUMENTS[0] = true }
   ```

3. A `.sheet` modifier on the ZStack:
   ```swift
   .sheet(isPresented: $show$ARGUMENTS[0]) {
       $ARGUMENTS[0]().environmentObject(appState)
   }
   ```

### 3. Focus handling reminder

If the new view has interactive elements:
- Wrap horizontal button groups in `.focusSection()`
- Use `.buttonStyle(.plain)` + `.hoverEffect(.highlight)` on buttons
- Add `@FocusState` if custom focus navigation is needed

### 4. Verify

After creating the file:
- Confirm the file is in the `tvos/AudioViz/Views/` directory (XcodeGen picks it up automatically from `project.yml` source glob)
- Run `/tvos-review tvos/AudioViz/Views/$ARGUMENTS[0].swift` to validate

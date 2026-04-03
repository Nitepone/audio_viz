# audio_viz — tvOS

A tvOS music visualizer app. Plays Apple Music via MusicKit and visualizes the audio in real time using the same Rust visualizer core as the terminal and web builds.

## Architecture

```
tvos/
├── build-rust.sh              Cross-compile Rust core → libaudio_viz.a
├── project.yml                XcodeGen spec (source of truth for .xcodeproj)
├── bridge/
│   ├── audio_viz.h            C header — aviz_* FFI declarations
│   ├── libaudio_viz.a         Compiled Rust static library (device or stub)
│   └── stub.c                 Simulator stub — aviz_* returning safe empty values
└── AudioViz/
    ├── App.swift              @main entry — injects AppState, starts engine
    ├── AppState.swift         @MainActor ObservableObject — owns engine + bridge + music
    ├── RustBridge.swift       Swift wrapper around aviz_* C FFI + NSLock
    ├── AudioViz-Bridging-Header.h  Imports audio_viz.h into Swift
    ├── AudioViz.entitlements  com.apple.developer.musickit
    ├── Audio/
    │   ├── AudioEngine.swift  AVAudioEngine + output tap + vDSP FFT → onAudioFrame
    │   └── MusicPlayer.swift  MusicKit auth, library fetch, ApplicationMusicPlayer
    ├── Rendering/
    │   ├── VisualizerView.swift  TimelineView + Canvas at 45 fps
    │   └── CellRenderer.swift   JSON cell decode + CoreGraphics draw (80×45 grid)
    ├── Views/
    │   ├── NowPlayingView.swift  Root view — fullscreen visualizer + control strip
    │   ├── LibraryView.swift     Songs / Albums / Playlists / Search tabs
    │   ├── VisualizerPickerView.swift  Horizontal carousel to switch visualizers
    │   └── SettingsView.swift    Dynamic form from visualizer config JSON
    └── Models/
        └── ConfigModels.swift   ConfigRoot / ConfigItem / JSONValue (Codable)
```

## Build Commands

**Regenerate Xcode project** (run after editing `project.yml`):
```bash
cd tvos
xcodegen generate
```

**Build for Simulator** (no Rust .a needed — stub is used):
```bash
cd tvos
xcodebuild \
  -project AudioViz.xcodeproj -scheme AudioViz \
  -destination 'generic/platform=tvOS Simulator' \
  -sdk appletvsimulator \
  CODE_SIGN_IDENTITY="" CODE_SIGNING_REQUIRED=NO \
  build
```

**Build Rust static library for device** (requires nightly + aarch64-apple-tvos target):
```bash
cd tvos
./build-rust.sh           # release (default)
./build-rust.sh --debug   # debug build
```

> **Important:** Never run `wasm-pack` from this directory. The Rust build for tvOS uses `cargo +nightly -Z build-std --target aarch64-apple-tvos`. The simulator uses a pre-built stub in `bridge/stub.c`; only the device build uses real Rust.

## Key Concepts

### Audio pipeline

```
AVAudioEngine (AudioEngine.swift)
  └── AVAudioPlayerNode  ←── MusicPlayer schedules tracks here
        └── mainMixerNode
              └── installTap (4096-frame buffer)
                    ├── Sliding window → sampleBufferL / sampleBufferR
                    ├── vDSP Hann-windowed FFT → 2049 magnitude bins
                    └── onAudioFrame(fft, left, right, dt, sampleRate)
                          └── RustBridge.tick()  [audio thread]
```

`VisualizerView` calls `RustBridge.render()` on the main thread at 45 fps. Both `tick()` and `render()` acquire `RustBridge.lock` (an NSLock), so they are safe to call concurrently.

### Audio capture limitation

`ApplicationMusicPlayer` (used by MusicKit) routes audio through the system session, **not** through our `AVAudioEngine`. The visualizer therefore animates on the audio that flows through our engine (test tones, or future local-file support). When Apple Music is playing, the visualizer shows an idle animation. Full synchronised visualisation requires audio to flow through `AudioEngine.playerNode`.

### Rust FFI

`RustBridge.swift` wraps the C functions declared in `bridge/audio_viz.h`:

| Function | Purpose |
|---|---|
| `aviz_create(name, cols, rows)` | Create a visualizer by name; returns opaque handle |
| `aviz_destroy(handle)` | Free the handle |
| `aviz_resize(handle, cols, rows)` | Update grid dimensions |
| `aviz_tick(handle, fft, left, right, dt, sr)` | Advance one audio frame |
| `aviz_render(handle, fps)` | Render → JSON cell array string |
| `aviz_name(handle)` | Active visualizer name |
| `aviz_get_config(handle)` | Config JSON (see schema below) |
| `aviz_set_config(handle, json)` | Apply (partial) config, returns merged JSON |
| `aviz_list_visualizers()` | `[{name, description}]` JSON array (static) |

Returned `const char *` pointers are owned by the handle and valid only until the next call to the same function. `RustBridge` copies them to `String` immediately.

### Config JSON schema

```json
{
  "visualizer_name": "spectrum",
  "version": 1,
  "config": [
    {"name":"gain",  "display_name":"Gain",  "type":"float", "value":1.0, "min":0.0, "max":4.0},
    {"name":"theme", "display_name":"Theme", "type":"enum",  "value":"hifi", "variants":["classic","hifi","led"]},
    {"name":"mirror","display_name":"Mirror","type":"bool",  "value":true}
  ]
}
```

`SettingsView` decodes this into `[ConfigItem]`, renders dynamic controls (float → `+/−` buttons, enum → `Picker`, bool → `Toggle`), and sends the full modified JSON back via `aviz_set_config`.

### Grid layout

The renderer uses a fixed 80-column × 45-row grid (matching `CellRenderer.cols/rows`). On the standard tvOS screen (1920×1080 points) each cell is 24×24 pt, font ≈ 20 pt monospaced. Pass `UInt16(CellRenderer.cols)` and `UInt16(CellRenderer.rows)` when constructing a `RustBridge`.

### tvOS-specific constraints

- **No `Slider` or `Stepper`** — use `Button` pairs with `+/−` icons for numeric input.
- **No `UITextFieldStyle.roundedBorder`** — use `.plain` or `.automatic`.
- **No microphone** — `AVAudioSession` recording is blocked on tvOS.
- **`ApplicationMusicPlayer` is not tappable** — see audio capture limitation above.
- **Siri Remote**: use `.onPlayPauseCommand`, `.onMoveCommand`, `.hoverEffect()`, and `@FocusState` for remote interaction. Menu button is handled by the system.

## State ownership

```
App.swift
  @StateObject AppState          ← single instance for the app lifetime
    AudioEngine                  ← AVAudioEngine, always running
    RustBridge?                  ← nil until start(), replaced by switchVisualizer()
    MusicPlayer                  ← MusicKit wrapper, @MainActor
```

All `@Published` mutations happen on `@MainActor`. The audio callback captures `RustBridge` directly (not `self`) to avoid actor-hop overhead on every frame.

## Adding a visualizer

Visualizers are defined in Rust (`../src/visualizers/`). The tvOS app discovers them at runtime via `aviz_list_visualizers()` — no Swift changes needed when a new Rust visualizer is added. After adding a Rust visualizer, rebuild the static library:

```bash
./build-rust.sh
```

## Modifying the Xcode project

Edit `project.yml` (not the `.xcodeproj` directly), then run `xcodegen generate`. The `.xcodeproj` is regenerated and should be treated as a build artefact.

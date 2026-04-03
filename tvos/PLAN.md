# audio_viz — tvOS App Plan

## Concept

A combined **music player + audio visualizer** for Apple TV. The app plays tracks from the user's Apple Music library and visualizes the audio in real time using the same Rust visualizer core as the terminal and web builds.

This approach is required because tvOS provides no API to capture audio from other apps (the Music app, etc.). By playing music through its own AVAudioEngine, the app can tap its own audio output for visualization.

---

## Architecture

```
tvos/
├── PLAN.md                         (this file)
├── AudioViz.xcodeproj/
└── AudioViz/
    ├── App.swift                   SwiftUI app entry point
    ├── ContentView.swift           Root nav / state router
    │
    ├── Views/
    │   ├── NowPlayingView.swift    Fullscreen visualizer + transport controls
    │   ├── LibraryView.swift       Browse Apple Music library
    │   ├── VisualizerPickerView.swift  Swipe through visualizers
    │   └── SettingsView.swift      Per-visualizer config sliders/pickers
    │
    ├── Audio/
    │   ├── AudioEngine.swift       AVAudioEngine: playback + output tap + FFT
    │   └── MusicPlayer.swift       MusicKit integration (library, queue, playback)
    │
    ├── Rust/
    │   ├── audio_viz.h             C header for Rust FFI
    │   ├── libaudio_viz.a          Pre-built Rust static library (aarch64-apple-tvos)
    │   └── RustBridge.swift        Swift wrapper around FFI calls
    │
    └── Rendering/
        ├── CellRenderer.swift      Parses Rust JSON cell output → draw calls
        └── VisualizerView.swift    CADisplayLink-driven CoreGraphics canvas
```

---

## Key Technologies

| Concern | Technology |
|---|---|
| Music library access | MusicKit (tvOS 15+) |
| Audio playback | AVAudioEngine + AVAudioPlayerNode |
| Audio analysis | AVAudioEngine output tap + Accelerate.vDSP FFT |
| Visualizer logic | Rust core compiled to aarch64-apple-tvos static lib |
| Swift ↔ Rust | C FFI via bridging header |
| Rendering | CoreGraphics in CADisplayLink (60fps) |
| UI framework | SwiftUI (tvOS focus engine, Siri Remote gestures) |

---

## User Interface

### Screen 1 — Now Playing (primary / fullscreen)
- Fullscreen visualizer canvas (black background, cell-based rendering)
- Overlay fades in on Siri Remote interaction, auto-hides after 3s
- Overlay shows: track title, artist, album art thumbnail
- Transport controls: previous / play-pause / next (Siri Remote play/pause + swipe)
- Bottom-right: "Visuals" button → navigates to Visualizer Picker
- Bottom-left: settings gear → navigates to Settings

### Screen 2 — Library Browser
- Shown at launch if nothing is playing
- Tabs: Playlists / Albums / Artists / Songs
- Standard tvOS card grid layout, large album art
- Select a track/album/playlist → begins playing → transitions to Now Playing

### Screen 3 — Visualizer Picker
- Horizontal carousel of visualizer preview cards
- Each card shows the visualizer name and a brief description
- Select to switch the active visualizer immediately
- Siri Remote swipe left/right to browse, click to confirm

### Screen 4 — Settings Panel
- Slides in from the right over the Now Playing view (sheet presentation)
- Lists config fields for the current visualizer (from `get_default_config()`)
- Sliders for float fields, segmented pickers for enum fields, toggles for bool fields
- Changes apply immediately via `set_config()` FFI call
- Reset button restores defaults

---

## Rust Integration

### Build

The Rust library compiles to a static lib for tvOS using Rust's `aarch64-apple-tvos` target (Tier 3, requires nightly + `-Z build-std`):

```bash
cargo +nightly build -Z build-std \
  --target aarch64-apple-tvos \
  --release \
  --no-default-features \
  --features wasm   # reuses the no-terminal feature set
```

The resulting `libaudio_viz.a` is linked into the Xcode project.

### C FFI (audio_viz.h)

```c
// Lifecycle
void*  aviz_create(const char* name);   // create visualizer by name
void   aviz_destroy(void* handle);

// Per-frame
void   aviz_tick(void* handle,
                 const float* fft, size_t fft_len,
                 const float* left, const float* right, size_t pcm_len,
                 float dt);

// Returns JSON: [{"ch":"█","col":3,"row":7,"r":255,"g":64,"b":0,"bold":true,"dim":false}, ...]
const char* aviz_render(void* handle, uint16_t cols, uint16_t rows, float fps);

// Config
const char* aviz_get_default_config(void* handle);   // returns JSON string
const char* aviz_set_config(void* handle, const char* json);

// Registry
const char* aviz_list_visualizers();  // returns JSON array of {name, description}
```

### Render Output

The WASM build already serializes `render()` output as a JSON cell array. The tvOS build will reuse this exact format:

```json
[
  {"ch": "█", "col": 3, "row": 7, "r": 255, "g": 64, "b": 0, "bold": true, "dim": false},
  ...
]
```

`CellRenderer.swift` decodes this and draws each cell with CoreGraphics using a monospace font sized to fill the screen.

---

## Audio Pipeline

```
AVAudioEngine
  └── AVAudioPlayerNode (plays MusicKit tracks)
        └── mainMixerNode
              └── outputNode
                    └── installTap(bufferSize: 4096)
                          ├── PCM samples → left[], right[]
                          ├── vDSP FFT → fft[] magnitudes
                          └── aviz_tick(fft, left, right, dt)
```

Frame rate: 45fps (matches FPS_TARGET). The tap fires at audio buffer boundaries; a CADisplayLink drives rendering at display rate independently.

---

## MusicKit Integration

MusicKit (tvOS 15+) provides:
- `MusicLibrary` — access user's library (requires entitlement + user permission)
- `MusicCatalogSearch` — search Apple Music catalog
- `ApplicationMusicPlayer` — system music player (but uses its own audio session, not tappable)

**Important:** `ApplicationMusicPlayer` plays through the system, not through our AVAudioEngine — so we cannot tap it. Instead we must use `AVAudioPlayerNode` to play audio files directly. This requires:

1. Use MusicKit to browse and resolve tracks
2. Use `AVAsset`/`AVURLAsset` to load the audio content
3. Schedule on our own `AVAudioPlayerNode` within our engine
4. This requires the user to have an Apple Music subscription

Alternatively, for tracks in the local library (downloaded), this is straightforward. For streamed tracks it requires DRM-cleared playback — which MusicKit handles if the app is authorized. This is the same approach used by third-party music apps.

---

## Siri Remote Input Mapping

| Gesture | Action |
|---|---|
| Swipe left / right | Previous / Next track |
| Play/Pause button | Toggle playback |
| Click (select) | Show/hide overlay |
| Long press | Open visualizer picker |
| Menu button | Back / exit settings |
| Swipe up | Open library browser |
| Swipe down | Open settings |

---

## Apple Developer Account Requirements

- **Apple TV entitlement**: Required to run on hardware (free account runs in Simulator only)
- **MusicKit entitlement**: Required for library access — requested in App Store Connect
- **Paid developer account ($99/year)**: Required for TestFlight + App Store distribution

For local development and Simulator testing, a free Apple ID is sufficient.

---

## Implementation Phases

### Phase 1 — Rust Static Library
- [ ] Add `tvos` feature to Cargo.toml (mirrors `wasm`, excludes terminal crates)
- [ ] Implement C FFI functions in `src/ffi.rs` gated behind `tvos` feature
- [ ] Write `tvos/build-rust.sh` script for cross-compilation
- [ ] Verify `aarch64-apple-tvos` target compiles cleanly

### Phase 2 — Xcode Project Skeleton ✓
- [x] Create Xcode project targeting tvOS 17+
- [x] Link `libaudio_viz.a` and bridging header
- [x] Verify Swift can call `aviz_create` / `aviz_list_visualizers`

### Phase 3 — Audio Engine ✓
- [x] `AudioEngine.swift`: AVAudioEngine setup, output tap, vDSP FFT
- [x] Verify tap fires correctly with a test tone
- [x] Wire tap output to `aviz_tick`

### Phase 4 — MusicKit Player ✓
- [x] `MusicPlayer.swift`: library browsing, catalog search, ApplicationMusicPlayer playback
- [x] Handle permissions flow (MusicKit authorization + entitlements)
- [x] Queue management (next/previous, play song/album/playlist)

### Phase 5 — Renderer ✓
- [x] `VisualizerView.swift`: TimelineView + Canvas at 45 fps calling bridge.render()
- [x] `CellRenderer.swift`: JSON cell decode + CoreGraphics draw, 80×45 grid on 1920×1080 pt
- [x] NSLock added to RustBridge serialising tick() vs render() across threads

### Phase 6 — SwiftUI Interface ✓
- [x] `NowPlayingView`: fullscreen visualizer + always-visible bottom control strip
- [x] `LibraryView`: MusicKit browser — Songs/Albums/Playlists/Search tabs, auth prompt
- [x] `VisualizerPickerView`: horizontal carousel with SF Symbol previews + focus scale
- [x] `SettingsView`: dynamic form — float (+/− buttons), enum (Picker), bool (Toggle)
- [x] Siri Remote: `.onPlayPauseCommand`, `.hoverEffect()`, `@FocusState` scale animations

### Phase 7 — MVP for TestFlight ✓
- [x] Now Playing metadata in Control Center (automatic via `ApplicationMusicPlayer`)
- [x] App icon + launch screen (`Assets.xcassets` with layered icon structure; run `make-placeholder-icons.sh` to generate PNGs)

### Phase 8 — Screensaver + CI
- [ ] Screensaver mode (tvOS 17 screen saver extension)
- [ ] TestFlight build via GitHub Actions

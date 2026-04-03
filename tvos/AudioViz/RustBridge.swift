import Foundation

// ── RustBridge ────────────────────────────────────────────────────────────────
//
// Swift wrapper around the Rust audio_viz C FFI (audio_viz.h).
//
// Memory contract
// ───────────────
// Every `const char *` returned by an aviz_* function is valid only until the
// next call to the same function on the same handle.  This class copies each
// C string into a Swift String immediately so callers never touch a raw pointer.
//
// Thread safety
// ─────────────
// RustBridge is not thread-safe on its own.  Callers must ensure that tick()
// and render() are always called from the same serial queue (the audio/render
// loop uses a dedicated DispatchQueue in AudioEngine).

// @unchecked Sendable: thread safety is provided by `lock` — tick() fires
// from the CoreAudio thread; render/config are called from the main thread.
// All FFI calls acquire `lock` before touching the handle.
final class RustBridge: @unchecked Sendable {

    // MARK: - Handle + lock

    private let handle: AvizHandle
    /// Serialises concurrent access from the audio thread (tick) and the
    /// main/render thread (render, getConfig, setConfig).
    private let lock = NSLock()

    // MARK: - Init / deinit

    /// Create a visualizer by name (e.g. "spectrum", "matrix").
    /// Falls back to the first registered visualizer if the name is not found.
    init(name: String, cols: UInt16, rows: UInt16) {
        handle = aviz_create(name, cols, rows)
    }

    deinit {
        aviz_destroy(handle)
    }

    // MARK: - Per-frame

    /// Notify the visualizer that the character grid has been resized.
    func resize(cols: UInt16, rows: UInt16) {
        aviz_resize(handle, cols, rows)
    }

    /// Advance the visualizer by one audio frame.
    ///
    /// - Parameters:
    ///   - fft:        Magnitude spectrum; must contain exactly FFT_SIZE/2+1 = 2049 floats.
    ///   - left:       Left-channel PCM; must contain exactly FFT_SIZE = 4096 floats.
    ///   - right:      Right-channel PCM; must contain exactly FFT_SIZE = 4096 floats.
    ///   - dt:         Seconds elapsed since the previous tick.
    ///   - sampleRate: Audio sample rate (typically 44100).
    func tick(fft: [Float], left: [Float], right: [Float], dt: Float, sampleRate: UInt32) {
        lock.lock()
        defer { lock.unlock() }
        fft.withUnsafeBufferPointer { fftBuf in
            left.withUnsafeBufferPointer { leftBuf in
                right.withUnsafeBufferPointer { rightBuf in
                    aviz_tick(
                        handle,
                        fftBuf.baseAddress,  fft.count,
                        leftBuf.baseAddress,
                        rightBuf.baseAddress, left.count,
                        dt,
                        sampleRate
                    )
                }
            }
        }
    }

    /// Render the current frame and return a JSON string of cell objects.
    ///
    /// Format: `[{"ch":"█","col":3,"row":7,"r":255,"g":64,"b":0,"bold":true,"dim":false}, …]`
    func render(fps: Float) -> String {
        lock.lock()
        defer { lock.unlock() }
        return String(cString: aviz_render(handle, fps))
    }

    // MARK: - Metadata

    /// The active visualizer's name (e.g. "spectrum").
    var name: String {
        // aviz_name returns a pointer stable for the handle's lifetime — no lock needed.
        String(cString: aviz_name(handle))
    }

    // MARK: - Config

    /// The default config JSON for the active visualizer.
    func getConfig() -> String {
        lock.lock()
        defer { lock.unlock() }
        return String(cString: aviz_get_config(handle))
    }

    /// Apply a (possibly partial) config JSON; returns the full merged config.
    @discardableResult
    func setConfig(_ json: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        return String(cString: aviz_set_config(handle, json))
    }

    // MARK: - Registry (static)

    /// All compiled-in visualizer descriptors.
    static var allVisualizers: [VisualizerInfo] {
        let json = String(cString: aviz_list_visualizers())
        guard
            let data  = json.data(using: .utf8),
            let array = try? JSONDecoder().decode([VisualizerInfo].self, from: data)
        else { return [] }
        return array
    }

    /// Just the names, for convenience.
    static var allVisualizerNames: [String] {
        allVisualizers.map(\.name)
    }
}

// MARK: - Supporting types

/// A visualizer entry from `aviz_list_visualizers`.
struct VisualizerInfo: Decodable, Identifiable {
    let name:        String
    let description: String
    var id: String { name }
}

/// A single rendered cell decoded from `aviz_render`.
struct VisualizerCell: Decodable {
    let ch:   String
    let col:  Int
    let row:  Int
    let r:    UInt8
    let g:    UInt8
    let b:    UInt8
    let bold: Bool
    let dim:  Bool
}

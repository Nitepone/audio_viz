import Foundation
import Combine

// ── AppState ──────────────────────────────────────────────────────────────────
//
// Single source of truth shared across all views via @EnvironmentObject.
//
// Owns:
//  • AudioEngine  — AVAudioEngine + output tap + vDSP FFT
//  • RustBridge   — active Rust visualizer instance
//  • MusicPlayer  — MusicKit library browser + ApplicationMusicPlayer wrapper
//
// Threading
// ─────────
// @MainActor throughout.  The audio callback captures `bridge` and `heartbeat`
// directly to avoid crossing the actor boundary on every frame.
// RustBridge.tick() and .render() are each guarded by an internal NSLock.

// MARK: - AudioHeartbeat

/// Thread-safe stamp of the last real audio frame.  Lets the idle ticker
/// suppress itself while the audio engine is delivering real frames, avoiding
/// double-ticking and the 2× animation speed that would result.
private final class AudioHeartbeat: @unchecked Sendable {
    private let lock = NSLock()
    private var lastPulse = Double.zero          // ProcessInfo.systemUptime seconds

    func pulse() {
        lock.withLock { lastPulse = ProcessInfo.processInfo.systemUptime }
    }

    /// Seconds since the last pulse.  Large when no real audio is flowing.
    var age: Double {
        ProcessInfo.processInfo.systemUptime - lock.withLock { lastPulse }
    }
}

// MARK: - AppState

@MainActor
final class AppState: ObservableObject {

    // MARK: - Published

    @Published var isRunning            = false
    @Published var currentVizName       = ""
    @Published var availableVisualizers: [VisualizerInfo] = []

    // MARK: - Owned objects

    let audioEngine  = AudioEngine()
    let musicPlayer  = MusicPlayer()

    /// The active Rust bridge.  Non-nil after the first call to start().
    private(set) var bridge: RustBridge?

    private var idleTickerTask: Task<Void, Never>?
    private let audioHeartbeat = AudioHeartbeat()

    // MARK: - Init

    init() {
        availableVisualizers = RustBridge.allVisualizers
    }

    // MARK: - Engine lifecycle

    /// Start the audio engine and begin feeding the named visualizer.
    /// Stops any previous session first.  Falls back to the first available
    /// visualizer if `name` is not found.
    func start(visualizerName: String? = nil) {
        if isRunning { stop() }

        let name = visualizerName
            ?? availableVisualizers.first?.name
            ?? "spectrum"

        let b  = RustBridge(name: name, cols: UInt16(CellRenderer.cols),
                                        rows: UInt16(CellRenderer.rows))
        bridge        = b
        currentVizName = b.name

        // Capture `b` and `hb` (not `self`) so the @Sendable closure never
        // crosses the @MainActor boundary on every audio callback.
        let hb = audioHeartbeat
        audioEngine.onAudioFrame = { fft, left, right, dt, sampleRate in
            hb.pulse()
            b.tick(fft: fft, left: left, right: right, dt: dt, sampleRate: sampleRate)
        }

        do {
            try audioEngine.start()
            isRunning = true
        } catch {
            print("[AppState] AudioEngine start failed: \(error)")
        }

        // Always start the idle ticker.  It self-suppresses while real audio
        // frames are flowing (heartbeat age < 75 ms), so it only fires when
        // the engine is silent or couldn't start (e.g. tvOS Simulator).
        startIdleTicker(bridge: b)
    }

    func stop() {
        idleTickerTask?.cancel()
        idleTickerTask = nil
        audioEngine.stop()
        audioEngine.onAudioFrame = nil
        isRunning = false
    }

    /// Hot-swap the visualizer without restarting audio.
    func switchVisualizer(to name: String) {
        idleTickerTask?.cancel()
        idleTickerTask = nil
        let b = RustBridge(name: name, cols: UInt16(CellRenderer.cols),
                                       rows: UInt16(CellRenderer.rows))
        bridge        = b
        currentVizName = b.name
        let hb = audioHeartbeat
        audioEngine.onAudioFrame = { fft, left, right, dt, sampleRate in
            hb.pulse()
            b.tick(fft: fft, left: left, right: right, dt: dt, sampleRate: sampleRate)
        }
        startIdleTicker(bridge: b)
    }

    // MARK: - Idle ticker

    private func startIdleTicker(bridge b: RustBridge) {
        let hb      = audioHeartbeat
        let zeroFFT = [Float](repeating: 0, count: AudioEngine.fftBins)
        let zeroPCM = [Float](repeating: 0, count: AudioEngine.fftSize)
        let dt      = Float(1.0 / 45.0)
        idleTickerTask = Task.detached {
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(22))
                // Yield to real audio: only tick when no frame has arrived
                // in the last 75 ms (≈ 3 missed frames at 45 fps).
                guard hb.age > 0.075 else { continue }
                b.tick(fft: zeroFFT, left: zeroPCM, right: zeroPCM,
                       dt: dt, sampleRate: 44100)
            }
        }
    }

    // MARK: - Convenience

    /// Request MusicKit authorization and start the engine if not already running.
    func requestMusicAuthAndStart() async {
        await musicPlayer.requestAuthorization()
        if !isRunning { start() }
    }

    /// Play a test tone to verify the audio tap is active.
    func playTestTone() {
        if !isRunning { start() }
        audioEngine.playTestTone()
    }
}

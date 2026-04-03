import AVFoundation
import Accelerate

// ── AudioEngine ───────────────────────────────────────────────────────────────
//
// Owns AVAudioEngine and AVAudioPlayerNode.  An output tap on the main mixer
// captures everything the engine plays, computes a per-frame FFT via vDSP,
// and delivers (fft, left, right, dt, sampleRate) to `onAudioFrame`.
//
// Threading
// ─────────
// • Public API (start, stop, playTestTone) — call from the main thread.
// • `onAudioFrame` fires on the CoreAudio real-time thread; do not touch
//   UIKit/SwiftUI or allocate memory there.  RustBridge.tick() is safe to
//   call directly — it is pure Rust computation with no allocations in
//   steady state.
// • All mutable state shared between the main thread and the tap is
//   protected by `lock`.  FFT scratch buffers are only ever touched from
//   the tap thread and need no lock.

final class AudioEngine: @unchecked Sendable {

    // MARK: - Constants

    static let fftSize   = 4096
    static let fftBins   = fftSize / 2 + 1    // 2049 — matches Rust FFT_SIZE/2+1
    static let fpsTarget = Float(45)

    // MARK: - Callback

    /// Fires on the CoreAudio real-time thread at approximately `fpsTarget` Hz.
    /// Capture only `@Sendable`-safe types; do not update UI directly.
    var onAudioFrame: (@Sendable (
        _ fft:        [Float],
        _ left:       [Float],
        _ right:      [Float],
        _ dt:         Float,
        _ sampleRate: UInt32
    ) -> Void)?

    // MARK: - Engine nodes

    let engine     = AVAudioEngine()
    let playerNode = AVAudioPlayerNode()

    // MARK: - Shared state (guarded by `lock`)

    private let lock              = NSLock()
    private var sampleBufferL     = [Float](repeating: 0, count: AudioEngine.fftSize)
    private var sampleBufferR     = [Float](repeating: 0, count: AudioEngine.fftSize)
    private var newSampleCount    = 0
    private var hopSize           = 980          // samples between ticks ≈ sr/45
    private var actualSampleRate  = UInt32(44100)
    private var lastTickTime      = CACurrentMediaTime()

    // MARK: - FFT resources (tap-thread only — no lock required)

    private let log2n    = vDSP_Length(log2(Float(AudioEngine.fftSize)))
    private let fftSetup: FFTSetup
    private var hannWin  = [Float](repeating: 0, count: AudioEngine.fftSize)

    // MARK: - Init / deinit

    init() {
        fftSetup = vDSP_create_fftsetup(
            vDSP_Length(log2(Float(AudioEngine.fftSize))),
            FFTRadix(kFFTRadix2)
        )!
        vDSP_hann_window(&hannWin, vDSP_Length(AudioEngine.fftSize), Int32(vDSP_HANN_NORM))
    }

    deinit {
        vDSP_destroy_fftsetup(fftSetup)
    }

    // MARK: - Public API

    /// Configure the audio session, wire the player node into the engine, install
    /// the output tap, and start the engine.  Call once from the main thread.
    func start() throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playback, mode: .default)
        try session.setActive(true)

        engine.attach(playerNode)
        engine.connect(playerNode, to: engine.mainMixerNode, format: nil)

        // Determine the mixer output format so we know the real sample rate
        let mixerFormat = engine.mainMixerNode.outputFormat(forBus: 0)
        let sr = Float(mixerFormat.sampleRate)

        lock.lock()
        actualSampleRate = UInt32(mixerFormat.sampleRate)
        hopSize = max(1, Int(sr / AudioEngine.fpsTarget))
        lock.unlock()

        // The tap captures all audio routed through the main mixer.
        // bufferSize is advisory; CoreAudio may deliver different counts.
        engine.mainMixerNode.installTap(
            onBus: 0,
            bufferSize: AVAudioFrameCount(AudioEngine.fftSize),
            format: mixerFormat
        ) { [weak self] buffer, _ in
            self?.processTap(buffer)
        }

        engine.prepare()
        try engine.start()
    }

    /// Remove the tap and stop the engine.
    func stop() {
        engine.mainMixerNode.removeTap(onBus: 0)
        playerNode.stop()
        engine.stop()
        try? AVAudioSession.sharedInstance().setActive(false)
    }

    /// Schedule a 440 Hz sine tone for `duration` seconds through the player node.
    /// Use this to verify the tap is receiving audio before MusicKit is wired up.
    func playTestTone(duration: Double = 3.0) {
        let sr         = engine.mainMixerNode.outputFormat(forBus: 0).sampleRate
        guard let fmt  = AVAudioFormat(standardFormatWithSampleRate: sr, channels: 2),
              let buf  = AVAudioPCMBuffer(pcmFormat: fmt,
                                          frameCapacity: AVAudioFrameCount(sr * duration))
        else { return }

        buf.frameLength = buf.frameCapacity
        let n      = Int(buf.frameLength)
        let twoPiF = Float(2.0 * .pi * 440.0 / sr)

        for i in 0..<n {
            let s = 0.5 * sin(twoPiF * Float(i))
            buf.floatChannelData?[0][i] = s
            buf.floatChannelData?[1][i] = s
        }

        playerNode.scheduleBuffer(buf)
        playerNode.play()
    }

    // MARK: - Tap processing (CoreAudio thread)

    private func processTap(_ buffer: AVAudioPCMBuffer) {
        guard let channels = buffer.floatChannelData else { return }
        let frameCount = Int(buffer.frameLength)
        guard frameCount > 0 else { return }

        let channelCount = Int(buffer.format.channelCount)
        let newL = Array(UnsafeBufferPointer(start: channels[0], count: frameCount))
        let newR = channelCount > 1
            ? Array(UnsafeBufferPointer(start: channels[1], count: frameCount))
            : newL

        // ── Update sliding window ──────────────────────────────────────────────
        // We keep the latest `fftSize` samples by removing the oldest and
        // appending the newest.  Capped at fftSize so we never overshoot.

        lock.lock()

        let drop = min(frameCount, AudioEngine.fftSize)
        sampleBufferL.removeFirst(drop)
        sampleBufferR.removeFirst(drop)
        sampleBufferL.append(contentsOf: newL.suffix(drop))
        sampleBufferR.append(contentsOf: newR.suffix(drop))

        newSampleCount += frameCount
        guard newSampleCount >= hopSize else {
            lock.unlock()
            return
        }
        newSampleCount = 0

        // Snapshot under lock, then release before doing heavy FFT work
        let snapL = sampleBufferL
        let snapR = sampleBufferR
        let sr    = actualSampleRate
        let now   = CACurrentMediaTime()
        let dt    = Float(now - lastTickTime)
        lastTickTime = now

        lock.unlock()

        // ── FFT ───────────────────────────────────────────────────────────────
        let fft = computeFFT(left: snapL, right: snapR)
        onAudioFrame?(fft, snapL, snapR, dt, sr)
    }

    // MARK: - FFT (tap thread only)

    /// Compute magnitude spectrum from stereo PCM using vDSP.
    /// Returns `fftBins` (= fftSize/2+1 = 2049) linear-scale magnitudes,
    /// matching the layout expected by Rust's AudioFrame.fft.
    private func computeFFT(left: [Float], right: [Float]) -> [Float] {
        let n     = AudioEngine.fftSize
        let halfN = n / 2

        // 1. Mono mix: (L + R) * 0.5
        var mono = [Float](repeating: 0, count: n)
        vDSP_vadd(left, 1, right, 1, &mono, 1, vDSP_Length(n))
        var half = Float(0.5)
        vDSP_vsmul(mono, 1, &half, &mono, 1, vDSP_Length(n))

        // 2. Hann window to reduce spectral leakage
        vDSP_vmul(mono, 1, hannWin, 1, &mono, 1, vDSP_Length(n))

        // 3. Pack the N real samples into N/2 complex pairs for vDSP real-FFT.
        //    vDSP_ctoz interprets the array as interleaved (re, im) pairs, so
        //    even indices → real part, odd indices → imaginary part.
        var real = [Float](repeating: 0, count: halfN)
        var imag = [Float](repeating: 0, count: halfN)
        var mags = [Float](repeating: 0, count: halfN + 1)

        mono.withUnsafeBytes { rawMono in
            let cPtr = rawMono.baseAddress!.assumingMemoryBound(to: DSPComplex.self)
            real.withUnsafeMutableBufferPointer { rBuf in
                imag.withUnsafeMutableBufferPointer { iBuf in
                    var sc = DSPSplitComplex(realp: rBuf.baseAddress!,
                                             imagp: iBuf.baseAddress!)
                    vDSP_ctoz(cPtr, 2, &sc, 1, vDSP_Length(halfN))
                    vDSP_fft_zrip(fftSetup, &sc, 1, log2n, FFTDirection(FFT_FORWARD))

                    // Magnitudes for bins 1 … N/2-1
                    mags.withUnsafeMutableBufferPointer { magBuf in
                        var sc2 = sc   // copy so we can pass both &sc2 and magBuf
                        vDSP_zvabs(&sc2, 1, magBuf.baseAddress!, 1, vDSP_Length(halfN))
                    }
                }
            }
        }

        // vDSP packing convention: DC stored in real[0], Nyquist in imag[0]
        mags[0]     = abs(real[0])
        mags[halfN] = abs(imag[0])

        // Normalise so magnitudes are in a visualiser-friendly range
        var scale = 2.0 / Float(n)
        vDSP_vsmul(mags, 1, &scale, &mags, 1, vDSP_Length(halfN + 1))

        return mags
    }
}

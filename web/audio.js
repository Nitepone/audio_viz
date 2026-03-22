/**
 * audio.js — Microphone capture via the Web Audio API.
 *
 * Uses AudioWorklet (processor.worklet.js) for PCM capture, which is
 * correctly supported on iOS Safari 15+.  ScriptProcessorNode was
 * previously used but is unreliable on iOS — it silently delivers
 * zero-filled buffers and its callback often never fires.
 *
 * The AudioContext sample rate is left at the browser/hardware default.
 * Forcing 44100 causes a cross-context mismatch warning in Firefox and
 * breaks audio capture on iOS entirely.  The actual rate is exposed via
 * the sampleRate getter and passed to the WASM tick() call so the
 * visualizers can compute correct FFT bin frequencies.
 */

const FFT_SIZE = 4096;

export class AudioCapture {
  constructor() {
    this._ctx      = null;
    this._analyser = null;
    this._fftBuf   = null;
    this._left     = new Float32Array(FFT_SIZE);
    this._right    = new Float32Array(FFT_SIZE);
    this._stream   = null;
    this._worklet  = null;
    this._started  = false;
  }

  /**
   * Request microphone access and start the audio graph.
   * Must be called from a user gesture (click/tap).
   * Returns a Promise that resolves once audio is flowing.
   */
  async start() {
    if (this._started) return;

    this._stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl:  false,
        // Requesting stereo forces iOS Safari into a non-voice AVAudioSession
        // category, which enables the mic mode picker in Control Center
        // (including the "Wide Spectrum" option).  On devices with only a
        // mono microphone the browser falls back to mono gracefully.
        channelCount:     { ideal: 2 },
        sampleRate:       { ideal: 44100 },
      },
      video: false,
    });

    // Do not force a sample rate — let the browser match the hardware.
    // iOS Safari suspends the context immediately even inside a gesture
    // handler, so we explicitly resume() after creation.
    this._ctx = new AudioContext();
    await this._ctx.resume();

    const source = this._ctx.createMediaStreamSource(this._stream);

    // ── Analyser for FFT magnitude ────────────────────────────────────────
    this._analyser = this._ctx.createAnalyser();
    this._analyser.fftSize               = FFT_SIZE;
    this._analyser.smoothingTimeConstant = 0.0;
    this._fftBuf   = new Float32Array(this._analyser.frequencyBinCount);
    source.connect(this._analyser);

    // ── AudioWorklet for raw PCM ──────────────────────────────────────────
    // The worklet file must be served from the same origin as the page.
    await this._ctx.audioWorklet.addModule('./processor.worklet.js');

    this._worklet = new AudioWorkletNode(this._ctx, 'capture-processor', {
      // Request up to 2 input channels; the worklet handles mono gracefully
      channelCount:          2,
      channelCountMode:      'explicit',
      channelInterpretation: 'discrete',
    });

    // The worklet posts complete FFT_SIZE windows back to the main thread
    const leftBuf  = this._left;
    const rightBuf = this._right;
    this._worklet.port.onmessage = (ev) => {
      leftBuf.set(ev.data.left);
      rightBuf.set(ev.data.right);
    };

    source.connect(this._worklet);
    // Worklet must be connected to destination to keep the audio graph alive
    this._worklet.connect(this._ctx.destination);

    this._started = true;
  }

  /** Stop the audio graph and release the microphone. */
  stop() {
    if (!this._started) return;
    this._worklet?.disconnect();
    this._analyser?.disconnect();
    this._stream?.getTracks().forEach(t => t.stop());
    this._ctx?.close();
    this._started = false;
  }

  /** Returns true once audio is flowing. */
  get isRunning() { return this._started; }

  /**
   * The actual sample rate negotiated with the hardware.
   * Only valid after start() has resolved.
   */
  get sampleRate() { return this._ctx?.sampleRate ?? 44100; }

  /**
   * Snapshot the current audio state.
   * Returns { fft: Float32Array, left: Float32Array, right: Float32Array }
   * where fft contains linear magnitude values (not dBFS).
   */
  getFrame() {
    if (!this._analyser) {
      return {
        fft:   new Float32Array(FFT_SIZE / 2 + 1),
        left:  this._left,
        right: this._right,
      };
    }

    // AnalyserNode returns dBFS; convert to linear magnitude to match rustfft output.
    this._analyser.getFloatFrequencyData(this._fftBuf);
    const fft = new Float32Array(this._fftBuf.length);
    for (let i = 0; i < this._fftBuf.length; i++) {
      fft[i] = Math.min(1.0, Math.pow(10, this._fftBuf[i] / 20));
    }

    return { fft, left: this._left, right: this._right };
  }
}

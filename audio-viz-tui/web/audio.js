/**
 * audio.js — Microphone and optional system audio capture via the Web Audio API.
 *
 * Two capture modes are supported:
 *
 *   Microphone  — getUserMedia(), works on all browsers and platforms.
 *
 *   System audio — getDisplayMedia() with audio:true.  The user shares a
 *     browser tab and checks "Share tab audio" in the picker.  Supported on
 *     Chrome and Edge (desktop only).  Firefox and Safari silently omit the
 *     audio track; mobile browsers do not support it at all.
 *
 * Uses AudioWorklet (processor.worklet.js) for PCM capture, which is
 * correctly supported on iOS Safari 15+.  ScriptProcessorNode was
 * previously used but is unreliable on iOS.
 *
 * The AudioContext sample rate is left at the browser/hardware default.
 * Forcing 44100 causes a cross-context mismatch warning in Firefox and
 * breaks audio capture on iOS.  The actual rate is exposed via the
 * sampleRate getter and passed to the WASM tick() call.
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
   * Returns true if getDisplayMedia system audio is likely supported.
   * Used to decide whether to show the system audio button.
   */
  static systemAudioSupported() {
    return (
      typeof navigator.mediaDevices?.getDisplayMedia === 'function' &&
      // Firefox and Safari implement getDisplayMedia but drop the audio track;
      // only offer it on Chromium-based browsers where it actually works.
      /Chrome|Chromium|Edg/.test(navigator.userAgent) &&
      !/Mobile|Android|iPhone|iPad/.test(navigator.userAgent)
    );
  }

  /**
   * Start microphone capture.
   * Must be called from a user gesture (click/tap).
   */
  async startMic() {
    if (this._started) return;
    this._stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl:  false,
        // Requesting stereo nudges iOS into a non-voice AVAudioSession category,
        // making the mic mode picker (Wide Spectrum etc.) available in Control Center.
        channelCount:     { ideal: 2 },
        sampleRate:       { ideal: 44100 },
      },
      video: false,
    });
    await this._initGraph();
  }

  /**
   * Start system audio capture via getDisplayMedia.
   * Prompts the user to share a tab; they must check "Share tab audio".
   * Resolves even if no audio track is present — call hasAudio() to check.
   */
  async startSystem() {
    if (this._started) return;
    const display = await navigator.mediaDevices.getDisplayMedia({
      video: true,   // required by the API even though we don't use the video
      audio: {
        systemAudio:              'include',
        suppressLocalAudioPlayback: false,
      },
    });

    // Extract only the audio tracks; discard video to avoid rendering overhead
    const audioTracks = display.getAudioTracks();
    if (audioTracks.length === 0) {
      display.getTracks().forEach(t => t.stop());
      throw new Error(
        'No audio track in the shared stream.\n' +
        'Make sure to check "Share tab audio" in the sharing dialog.'
      );
    }

    this._stream = new MediaStream(audioTracks);
    // Stop the video tracks we don't need
    display.getVideoTracks().forEach(t => t.stop());

    await this._initGraph();
  }

  /** Shared graph initialisation used by both capture modes. */
  async _initGraph() {
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
    await this._ctx.audioWorklet.addModule('./processor.worklet.js');

    this._worklet = new AudioWorkletNode(this._ctx, 'capture-processor', {
      channelCount:          2,
      channelCountMode:      'explicit',
      channelInterpretation: 'discrete',
    });

    const leftBuf  = this._left;
    const rightBuf = this._right;
    this._worklet.port.onmessage = (ev) => {
      leftBuf.set(ev.data.left);
      rightBuf.set(ev.data.right);
    };

    source.connect(this._worklet);
    this._worklet.connect(this._ctx.destination);

    this._started = true;
  }

  /** Stop the audio graph and release all tracks. */
  stop() {
    if (!this._started) return;
    this._worklet?.disconnect();
    this._analyser?.disconnect();
    this._stream?.getTracks().forEach(t => t.stop());
    this._ctx?.close();
    this._ctx      = null;
    this._analyser = null;
    this._worklet  = null;
    this._stream   = null;
    this._started  = false;
  }

  get isRunning() { return this._started; }

  /** The actual sample rate negotiated with the hardware. */
  get sampleRate() { return this._ctx?.sampleRate ?? 44100; }

  /**
   * Snapshot the current audio state.
   * Returns { fft: Float32Array, left: Float32Array, right: Float32Array }
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

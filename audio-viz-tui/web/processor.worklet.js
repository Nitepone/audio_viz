/**
 * processor.worklet.js — AudioWorkletProcessor for PCM capture.
 *
 * Runs on the audio rendering thread.  Collects interleaved stereo samples
 * into a fixed-size ring buffer and posts them to the main thread in chunks
 * matching FFT_SIZE, so the visualizer always has a full window to work with.
 *
 * This replaces ScriptProcessorNode, which is unreliable on iOS Safari.
 */

const FFT_SIZE = 4096;

class CaptureProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    // Accumulate samples here until we have a full FFT_SIZE window
    this._left  = new Float32Array(FFT_SIZE);
    this._right = new Float32Array(FFT_SIZE);
    this._fill  = 0;
  }

  process(inputs) {
    const input = inputs[0];
    if (!input || input.length === 0) return true;

    const L = input[0];                          // always present
    const R = input.length > 1 ? input[1] : L;  // mirror mono → stereo

    // Web Audio delivers 128-sample quanta; accumulate until FFT_SIZE
    let offset = 0;
    while (offset < L.length) {
      const space = FFT_SIZE - this._fill;
      const take  = Math.min(space, L.length - offset);

      this._left .set(L.subarray(offset, offset + take), this._fill);
      this._right.set(R.subarray(offset, offset + take), this._fill);
      this._fill += take;
      offset     += take;

      if (this._fill === FFT_SIZE) {
        // Post a copy to the main thread and reset
        this.port.postMessage({
          left:  this._left.slice(),
          right: this._right.slice(),
        });
        this._fill = 0;
      }
    }

    return true; // keep processor alive
  }
}

registerProcessor('capture-processor', CaptureProcessor);

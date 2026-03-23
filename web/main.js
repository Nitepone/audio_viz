/**
 * main.js — Application entry point.
 *
 * Loads the WASM module, sets up the audio capture and canvas renderer,
 * and runs the animation loop.
 */

import { AudioCapture } from './audio.js';
import { Renderer }     from './renderer.js';
import init, { WebViz } from './pkg/audio_viz_web.js';

const canvas       = document.getElementById('canvas');
const vizSelect    = document.getElementById('viz-select');
const startBtn     = document.getElementById('start-btn');
const systemBtn    = document.getElementById('system-btn');
const overlayEl    = document.getElementById('overlay');
const overlayStart = document.getElementById('overlay-start-btn');
const overlaySystem = document.getElementById('overlay-system-btn');
const statusEl     = document.getElementById('status');

const audio    = new AudioCapture();
const renderer = new Renderer(canvas);

let wasm    = null;
let viz     = null;
let running = false;
let rafId   = null;
let lastTs  = null;
let frameCount = 0;
let fpsSmooth  = 0;

// ── Initialise WASM ───────────────────────────────────────────────────────────

async function initWasm() {
  wasm = await init();

  const names = JSON.parse(WebViz.all_names());
  for (const name of names) {
    const opt  = document.createElement('option');
    opt.value  = name;
    opt.text   = name;
    if (name === 'scope') opt.selected = true;
    vizSelect.appendChild(opt);
  }

  makeViz(vizSelect.value);

  // Show system audio button only on supported browsers (Chrome/Edge desktop)
  if (AudioCapture.systemAudioSupported()) {
    systemBtn.style.display     = '';
    overlaySystem.style.display = '';
  }
}

function makeViz(name) {
  viz?.free?.();
  viz = new WebViz(name, renderer.cols, renderer.rows);
}

// ── Resize handling ───────────────────────────────────────────────────────────

function handleResize() {
  renderer.resize();
  viz?.resize(renderer.cols, renderer.rows);
}

window.addEventListener('resize', handleResize);
handleResize();

// ── Animation loop ────────────────────────────────────────────────────────────

function loop(ts) {
  if (!running) return;

  rafId = requestAnimationFrame(loop);

  const dt = lastTs === null ? 1 / 60 : Math.min((ts - lastTs) / 1000, 0.15);
  lastTs = ts;

  frameCount++;
  fpsSmooth = fpsSmooth * 0.92 + (1 / dt) * 0.08;
  if (frameCount % 30 === 0) {
    statusEl.textContent = `${fpsSmooth.toFixed(0)} fps`;
  }

  const { fft, left, right } = audio.getFrame();
  viz.tick(fft, left, right, dt, audio.sampleRate);

  const cellsJson = viz.render(fpsSmooth);
  const cells     = JSON.parse(cellsJson);
  renderer.drawFrame(cells);
}

// ── Start / stop ──────────────────────────────────────────────────────────────

async function start(mode) {
  if (running) return;

  startBtn.textContent     = '…';
  startBtn.disabled        = true;
  systemBtn.disabled       = true;
  overlayStart.disabled    = true;
  overlaySystem.disabled   = true;

  try {
    if (mode === 'system') {
      await audio.startSystem();
    } else {
      await audio.startMic();
    }
  } catch (err) {
    statusEl.textContent   = err.message || 'Audio access denied.';
    startBtn.textContent   = 'Microphone';
    startBtn.disabled      = false;
    systemBtn.disabled     = false;
    overlayStart.disabled  = false;
    overlaySystem.disabled = false;
    return;
  }

  overlayEl.classList.add('hidden');
  running = true;
  lastTs  = null;
  startBtn.textContent  = 'Stop';
  startBtn.disabled     = false;
  systemBtn.style.display = 'none'; // hide the alternate button while running
  rafId = requestAnimationFrame(loop);
}

function stop() {
  running = false;
  if (rafId !== null) { cancelAnimationFrame(rafId); rafId = null; }
  audio.stop();
  startBtn.textContent = 'Microphone';
  if (AudioCapture.systemAudioSupported()) {
    systemBtn.style.display = '';
    systemBtn.disabled = false;
  }
  statusEl.textContent = 'Stopped.';
}

startBtn.addEventListener('click',    () => running ? stop() : start('mic'));
systemBtn.addEventListener('click',   () => start('system'));
overlayStart.addEventListener('click',  () => start('mic'));
overlaySystem.addEventListener('click', () => start('system'));

// ── Visualizer switching ──────────────────────────────────────────────────────

vizSelect.addEventListener('change', () => makeViz(vizSelect.value));

// ── Boot ──────────────────────────────────────────────────────────────────────

initWasm().catch(err => {
  statusEl.textContent = `Failed to load WASM: ${err}`;
  console.error(err);
});

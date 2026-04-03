#ifndef AUDIO_VIZ_H
#define AUDIO_VIZ_H

/// audio_viz.h — C interface to the Rust audio_viz static library.
///
/// Use this file as the Xcode "Objective-C Bridging Header" so Swift can call
/// into the Rust core directly.
///
/// Memory contract
/// ───────────────
/// Functions that return `const char *` own the pointed-to string inside the
/// handle (or in a process-wide static).  The pointer is valid until the next
/// call to the same function on the same handle, or until aviz_destroy.
/// Swift callers should copy immediately:
///
///   let json = String(cString: aviz_render(handle, fps))

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Opaque handle to a live visualizer instance.
typedef void *AvizHandle;

// ── Lifecycle ─────────────────────────────────────────────────────────────────

/// Create a visualizer by name (e.g. "spectrum", "matrix", "fire").
/// Pass the initial character-grid dimensions in cols/rows.
/// Returns NULL only if no visualizers were compiled in (should never happen).
/// Free with aviz_destroy.
AvizHandle aviz_create(const char *name, uint16_t cols, uint16_t rows);

/// Destroy a handle and release all associated memory.
void aviz_destroy(AvizHandle handle);

// ── Per-frame ─────────────────────────────────────────────────────────────────

/// Notify the visualizer that the character grid has been resized.
void aviz_resize(AvizHandle handle, uint16_t cols, uint16_t rows);

/// Advance the visualizer by one audio frame (~45 fps).
///
/// fft       magnitude spectrum; fft_len must be FFT_SIZE/2+1 = 2049
/// left      PCM samples for the left channel;  pcm_len must be FFT_SIZE = 4096
/// right     PCM samples for the right channel; pcm_len must be FFT_SIZE = 4096
/// dt        seconds elapsed since the previous tick
/// sample_rate  audio sample rate (e.g. 44100)
void aviz_tick(AvizHandle handle,
               const float *fft,   size_t fft_len,
               const float *left,
               const float *right, size_t pcm_len,
               float        dt,
               uint32_t     sample_rate);

/// Render the current frame.
///
/// Returns a JSON array of non-space cell objects:
///   [{"ch":"█","col":3,"row":7,"r":255,"g":64,"b":0,"bold":true,"dim":false}, …]
///
/// fps is passed through to the visualizer for any HUD / FPS display.
///
/// Pointer is valid until the next aviz_render call on this handle.
const char *aviz_render(AvizHandle handle, float fps);

// ── Metadata ──────────────────────────────────────────────────────────────────

/// Return the active visualizer's name (e.g. "spectrum").
/// Pointer is stable for the lifetime of the handle.
const char *aviz_name(AvizHandle handle);

// ── Config ────────────────────────────────────────────────────────────────────

/// Return the default config JSON for the active visualizer.
///
/// Example:
///   {"settings":[{"key":"gain","label":"Gain","type":"float","value":1.5,
///                 "min":0.5,"max":4.0,"step":0.1}, …]}
///
/// Pointer valid until the next aviz_get_config or aviz_set_config on this handle.
const char *aviz_get_config(AvizHandle handle);

/// Apply a (possibly partial) config JSON and return the full merged config.
/// Pointer valid until the next aviz_get_config or aviz_set_config on this handle.
const char *aviz_set_config(AvizHandle handle, const char *json);

// ── Registry ──────────────────────────────────────────────────────────────────

/// Return a JSON array of all compiled-in visualizers:
///   [{"name":"fire","description":"Flame simulation"}, …]
///
/// Computed once; pointer is valid for the lifetime of the process.
const char *aviz_list_visualizers(void);

#ifdef __cplusplus
}
#endif

#endif /* AUDIO_VIZ_H */

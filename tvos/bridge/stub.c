// stub.c — Fallback aviz_* symbols for builds where neither libaudio_viz.a
// nor libaudio_viz_sim.a is available.
//
// Under normal circumstances this file is EXCLUDED from both device and
// simulator builds (see EXCLUDED_SOURCE_FILE_NAMES in project.yml).  The
// real libraries produced by build-rust.sh are used instead.
//
// This file is kept only as a last-resort compile target so the project can
// still be opened and the UI inspected without a Rust toolchain available.

#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <stdio.h>

// Track the active visualizer name so aviz_name() reflects the selection.
static char s_name[256] = "stub";

void *aviz_create(const char *name, uint16_t cols, uint16_t rows) {
    if (name) { strncpy(s_name, name, sizeof(s_name) - 1); s_name[sizeof(s_name) - 1] = '\0'; }
    return (void *)1;
}
void  aviz_destroy(void *handle) {}
void  aviz_resize(void *handle, uint16_t cols, uint16_t rows) {}

void  aviz_tick(void *handle,
                const float *fft,   size_t fft_len,
                const float *left,
                const float *right, size_t pcm_len,
                float dt, uint32_t sample_rate) {}

// Animated render: scrolling rainbow text showing the active visualizer name.
// Uses only integer arithmetic — no floating point, no math.h required.
static char s_render_buf[8192];
static uint32_t s_frame = 0;

const char *aviz_render(void *handle, float fps) {
    s_frame++;

    // Grid dimensions matching CellRenderer.cols / CellRenderer.rows
    const int COLS = 80;
    const int ROWS = 45;

    // Center the name vertically and horizontally
    int name_len = 0;
    while (s_name[name_len]) name_len++;

    int row = ROWS / 2;
    int col_start = (COLS - name_len) / 2;
    if (col_start < 0) col_start = 0;

    // Build JSON array: one cell per character.
    // CellRenderer expects: [{"ch":"X","col":int,"row":int,"r":int,"g":int,"b":int,"bold":bool,"dim":bool}]
    // Precomputed RGB for 6 vivid rainbow hues:
    static const int pal_r[6] = { 255,  215, 215,   0,   0, 135 };
    static const int pal_g[6] = {   0,  135, 215, 215,   0,   0 };
    static const int pal_b[6] = {   0,    0,   0,   0, 215, 215 };

    char *p = s_render_buf;
    *p++ = '[';
    int first = 1;

    for (int i = 0; i < name_len && col_start + i < COLS; i++) {
        char ch = s_name[i];
        if (ch == 0) break;

        // Cycle colour per character, shifting with frame for animation
        int ci = ((int)(s_frame / 3) + i) % 6;
        int r = pal_r[ci], g = pal_g[ci], b = pal_b[ci];

        if (!first) *p++ = ',';
        first = 0;

        int cx = col_start + i;
        int written = snprintf(p, s_render_buf + sizeof(s_render_buf) - p,
            "{\"ch\":\"%c\",\"col\":%d,\"row\":%d,\"r\":%d,\"g\":%d,\"b\":%d,\"bold\":false,\"dim\":false}",
            ch, cx, row, r, g, b);
        if (written < 0 || p + written >= s_render_buf + sizeof(s_render_buf) - 2) break;
        p += written;
    }

    *p++ = ']';
    *p   = '\0';
    return s_render_buf;
}
const char *aviz_name(void *handle)              { return s_name; }

// Sample config with one field of each type — lets the Settings UI be
// exercised in the simulator even though no real visualizer is running.
static char s_config[1024];
static const char *stub_config(void) {
    snprintf(s_config, sizeof(s_config),
        "{\"visualizer_name\":\"%s\",\"version\":1,\"config\":["
        "{\"name\":\"gain\",\"display_name\":\"Gain\","
         "\"type\":\"float\",\"value\":1.0,\"min\":0.0,\"max\":4.0},"
        "{\"name\":\"theme\",\"display_name\":\"Theme\","
         "\"type\":\"enum\",\"value\":\"hifi\","
         "\"variants\":[\"classic\",\"hifi\",\"led\"]},"
        "{\"name\":\"mirror\",\"display_name\":\"Mirror\","
         "\"type\":\"bool\",\"value\":true}"
        "]}",
        s_name);
    return s_config;
}

const char *aviz_get_config(void *handle)                   { return stub_config(); }
const char *aviz_set_config(void *handle, const char *json) { return stub_config(); }
const char *aviz_list_visualizers(void) {
    // Mirror of the visualizers compiled into the real library.
    // Kept in sync manually; lets the simulator picker show the full list
    // even though aviz_render returns an empty frame.
    return
        "[{\"name\":\"spectrum\",\"description\":\"Classic log-spaced frequency bars\"},"
        "{\"name\":\"radial\",\"description\":\"Polar spectrum radiating from the centre\"},"
        "{\"name\":\"vu\",\"description\":\"Stereo / mono VU meter\"},"
        "{\"name\":\"waterfall\",\"description\":\"Scrolling spectrogram \\u2014 frequency vs time\"},"
        "{\"name\":\"scope\",\"description\":\"Dual-channel time-domain oscilloscope\"},"
        "{\"name\":\"lissajous\",\"description\":\"Full-terminal XY scope \\u2014 beat rotation, planets, vocal stars, ripples\"},"
        "{\"name\":\"classic_lissajous\",\"description\":\"Classic XY phosphor oscilloscope \\u2014 Lissajous figure\"},"
        "{\"name\":\"polar\",\"description\":\"Polar waveform \\u2014 circular oscilloscope\"},"
        "{\"name\":\"fire\",\"description\":\"Audio-reactive ASCII fire\"},"
        "{\"name\":\"matrix\",\"description\":\"Audio-reactive falling character rain\"},"
        "{\"name\":\"plasma\",\"description\":\"Interference plasma \\u2014 audio-reactive sine wave colour field\"},"
        "{\"name\":\"ripple\",\"description\":\"2-D wave propagation excited by audio beats\"},"
        "{\"name\":\"tunnel\",\"description\":\"Perspective fly-through tunnel with audio-reactive walls\"},"
        "{\"name\":\"aurora\",\"description\":\"Sinusoidal curtains of light driven by frequency bands\"},"
        "{\"name\":\"missiles\",\"description\":\"Missile Command: audio-driven missile rain, interceptors, and city damage\"},"
        "{\"name\":\"night_sky\",\"description\":\"Real star map with constellation lines and beat-driven camera panning\"},"
        "{\"name\":\"orbit\",\"description\":\"Stereo phase constellation \\u2014 mid/side polar scatter plot\"},"
        "{\"name\":\"pulsar\",\"description\":\"Radial waveform ring \\u2014 concentric history, scope overlay, beat wobble\"},"
        "{\"name\":\"crystal\",\"description\":\"Kaleidoscope symmetry mandala driven by spectrum energy\"}]";
}

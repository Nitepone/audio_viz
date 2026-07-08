#!/usr/bin/env bash
# =============================================================================
#  audio_viz.sh — Multi-mode ASCII Audio Visualizer (256-colour)
#                 Works on Linux (PulseAudio / PipeWire) and macOS.
#
#  Usage:  ./audio_viz.sh [VISUALIZER] [SOURCE]
#
#  Visualizers:
#    spectrum   Classic log-spaced frequency bars          (default)
#    scope      Dual-channel oscilloscope (time domain)
#    matrix     Audio-reactive Matrix rain
#    radial     Polar spectrum — bands radiate from centre
#    lissajous  X-Y oscilloscope (Lissajous figure)
#    fire       Audio-reactive ASCII fire
#
#  SOURCE (optional):
#    Linux  — PulseAudio/PipeWire source name (auto-detected if omitted)
#             Run `pactl list short sources` to list available sources.
#    macOS  — AVFoundation device index or name (auto-detected if omitted)
#             Run `ffmpeg -f avfoundation -list_devices true -i ""` to list.
#             For system audio capture you need a loopback driver:
#               • BlackHole  (free)   https://existential.audio/blackhole/
#               • Loopback   (paid)   https://rogueamoeba.com/loopback/
#             Set that device as your system audio output, then this script
#             will find and capture it automatically.
#
#  Dependencies:
#    Linux  — pulseaudio-utils (or pipewire-pulse), python3, python3-numpy
#               sudo apt install pulseaudio-utils python3-numpy
#    macOS  — ffmpeg, python3, python3-numpy
#               brew install ffmpeg
#               pip3 install numpy
# =============================================================================

set -euo pipefail

# ── Detect OS ─────────────────────────────────────────────────────────────────
OS="$(uname -s)"

usage() {
cat <<EOF
Usage: $(basename "$0") [VISUALIZER] [SOURCE]

Visualizers:
  spectrum    Log-spaced frequency bars (default)
  scope       Dual-channel oscilloscope
  matrix      Audio-reactive Matrix rain
  radial      Polar spectrum radiating from centre
  lissajous   X-Y oscilloscope (Lissajous figure)
  fire        Audio-reactive ASCII fire

Options:
  -l, --list   List visualizer names and exit
  -h, --help   Show this help and exit

SOURCE:
  Linux   PulseAudio/PipeWire monitor source name (auto-detected if omitted)
  macOS   AVFoundation device index, e.g. "2"     (auto-detected if omitted)
          Requires BlackHole or another loopback driver for system audio.

Examples:
  $(basename "$0")                     # spectrum, auto-detect source
  $(basename "$0") matrix              # matrix rain, auto-detect source
  $(basename "$0") lissajous 2         # lissajous on macOS device :2
  $(basename "$0") fire alsa.monitor   # fire on a specific Linux source
EOF
}

# ── Argument parsing ──────────────────────────────────────────────────────────
VIZ=""
SOURCE_ARG=""

for arg in "$@"; do
    case "$arg" in
        -h|--help)  usage; exit 0 ;;
        -l|--list)  echo "spectrum scope matrix radial lissajous fire" | tr ' ' '\n'; exit 0 ;;
        spectrum|scope|matrix|radial|lissajous|fire) VIZ="$arg" ;;
        *)          SOURCE_ARG="$arg" ;;
    esac
done

[[ -z "$VIZ" ]] && VIZ="spectrum"

# ── Shared: python3 + numpy ───────────────────────────────────────────────────
MISSING=()
command -v python3 &>/dev/null || MISSING+=("python3")
python3 -c "import numpy" 2>/dev/null || MISSING+=("python3-numpy")

# =============================================================================
#  LINUX path  (parec / PulseAudio / PipeWire)
# =============================================================================
if [[ "$OS" == "Linux" ]]; then

    for cmd in parec pactl; do
        command -v "$cmd" &>/dev/null || MISSING+=("$cmd")
    done

    if [[ ${#MISSING[@]} -gt 0 ]]; then
        echo "ERROR: Missing dependencies: ${MISSING[*]}"
        echo "Install: sudo apt install pulseaudio-utils python3-numpy"
        echo "PipeWire: sudo apt install pipewire-pulse python3-numpy"
        exit 1
    fi

    MONITOR="$SOURCE_ARG"
    if [[ -z "$MONITOR" ]]; then
        MONITOR=$(pactl list short sources 2>/dev/null \
                  | awk '{print $2}' | grep -i '\.monitor$' | head -1)
    fi

    if [[ -z "$MONITOR" ]]; then
        echo "ERROR: No monitor source found. Is PulseAudio/PipeWire running?"
        echo "List sources with: pactl list short sources"
        exit 1
    fi

    CAPTURE_BACKEND="pulse"
    CAPTURE_DEVICE="$MONITOR"

# =============================================================================
#  macOS path  (ffmpeg + AVFoundation)
# =============================================================================
elif [[ "$OS" == "Darwin" ]]; then

    command -v ffmpeg &>/dev/null || MISSING+=("ffmpeg")

    if [[ ${#MISSING[@]} -gt 0 ]]; then
        echo "ERROR: Missing dependencies: ${MISSING[*]}"
        echo ""
        echo "Install with Homebrew:"
        echo "  brew install ffmpeg"
        echo "  pip3 install numpy"
        echo ""
        echo "For system audio capture (music, browser, etc.) you also need"
        echo "a loopback audio driver:"
        echo "  • BlackHole (free):  https://existential.audio/blackhole/"
        echo "  • Loopback  (paid):  https://rogueamoeba.com/loopback/"
        echo ""
        echo "After installing BlackHole, set it as your audio output (or use"
        echo "a Multi-Output Device in Audio MIDI Setup), then re-run this script."
        exit 1
    fi

    # ── Auto-detect the best capture device ───────────────────────────────────
    # Preference order:
    #   1. Explicit user argument
    #   2. BlackHole (any channel count) — dedicated loopback
    #   3. Loopback (Rogue Amoeba)
    #   4. Any device whose name contains "monitor" or "output"
    #   5. Device index 0 (usually the built-in microphone — fallback only)

    CAPTURE_DEVICE=""

    if [[ -n "$SOURCE_ARG" ]]; then
        CAPTURE_DEVICE="$SOURCE_ARG"
    else
        # Enumerate AVFoundation audio devices via ffmpeg
        DEV_LIST=$(ffmpeg -f avfoundation -list_devices true -i "" 2>&1 || true)

        # Try loopback drivers first
        for pattern in "BlackHole" "Loopback" "[Mm]onitor" "[Oo]utput"; do
            if [[ -z "$CAPTURE_DEVICE" ]]; then
                CAPTURE_DEVICE=$(echo "$DEV_LIST" \
                    | grep -E "^\[AVFoundation.*\] \[[0-9]+\]" \
                    | grep -i "$pattern" \
                    | head -1 \
                    | grep -oE '^\[AVFoundation[^]]*\] \[([0-9]+)\]' \
                    | grep -oE '\[([0-9]+)\]' \
                    | tr -d '[]' || true)
            fi
        done

        # Absolute fallback: first audio device (index 0)
        if [[ -z "$CAPTURE_DEVICE" ]]; then
            CAPTURE_DEVICE="0"
            echo "WARNING: No loopback device found. Falling back to device :0"
            echo "         (likely your microphone — install BlackHole for system audio)"
            echo "         https://existential.audio/blackhole/"
            sleep 2
        fi
    fi

    CAPTURE_BACKEND="avfoundation"
    MONITOR=":${CAPTURE_DEVICE}"   # ffmpeg avfoundation audio-only syntax

else
    echo "ERROR: Unsupported OS: $OS  (supported: Linux, macOS/Darwin)"
    exit 1
fi

echo "Visualizer : $VIZ"
echo "Platform   : $OS"
echo "Source     : $MONITOR  (backend: $CAPTURE_BACKEND)"
echo "Press Ctrl+C to quit."
sleep 0.6

# ── Launch Python visualizer ──────────────────────────────────────────────────
python3 - "$MONITOR" "$VIZ" "$CAPTURE_BACKEND" << 'PYTHON_EOF'
import sys, os, math, time, signal, subprocess, threading, atexit, random
from collections import deque
import numpy as np

MONITOR  = sys.argv[1]
VIZ_NAME = sys.argv[2]
BACKEND  = sys.argv[3]   # "pulse" | "avfoundation"

# ─────────────────────────────────────────────────────────────────────────────
#  AUDIO CONSTANTS
# ─────────────────────────────────────────────────────────────────────────────
SR          = 44100
CH          = 2
FFT_N       = 4096
CHUNK       = FFT_N
CHUNK_BYTES = CHUNK * CH * 2

# ─────────────────────────────────────────────────────────────────────────────
#  DYNAMICS
# ─────────────────────────────────────────────────────────────────────────────
RISE      = 0.80
FALL      = 0.55
PEAK_HOLD = 1.2
PEAK_DRP  = 0.018
DB_MIN    = -72.0
DB_MAX    = -12.0
FPS_TGT   = 45

# ─────────────────────────────────────────────────────────────────────────────
#  TERMINAL
# ─────────────────────────────────────────────────────────────────────────────
_FG = [f'\033[38;5;{i}m' for i in range(256)]

def fg(c): return _FG[int(c) & 0xFF]

RST  = '\033[0m'
BLD  = '\033[1m'
DIM  = '\033[2m'
HIDE = '\033[?25l'
SHOW = '\033[?25h'
HOME = '\033[H'
CLR  = '\033[2J'
EL   = '\033[K'

def tsize():
    s = os.get_terminal_size()
    return s.lines, s.columns

# ─────────────────────────────────────────────────────────────────────────────
#  COLOUR GRADIENTS
# ─────────────────────────────────────────────────────────────────────────────
_SPEC = [196,202,208,214,220,226,190,154,118,82,46,47,48,49,50,51,45,39,33,27,21,57,93,129]

def specgrad(frac):
    i = int(frac * (len(_SPEC) - 1))
    return _SPEC[max(0, min(i, len(_SPEC) - 1))]

# ─────────────────────────────────────────────────────────────────────────────
#  SHARED AUDIO STATE
# ─────────────────────────────────────────────────────────────────────────────
_lock  = threading.Lock()
_LEFT  = np.zeros(CHUNK, np.float32)
_RIGHT = np.zeros(CHUNK, np.float32)
_MONO  = np.zeros(CHUNK, np.float32)
_RUN   = True

def get_audio():
    with _lock:
        return _LEFT.copy(), _RIGHT.copy(), _MONO.copy()

def _reader(proc):
    global _LEFT, _RIGHT, _MONO, _RUN
    buf = b''
    while _RUN:
        try:
            data = proc.stdout.read(CHUNK_BYTES)
            if not data:
                break
            buf += data
            while len(buf) >= CHUNK_BYTES:
                frame, buf = buf[:CHUNK_BYTES], buf[CHUNK_BYTES:]
                s = np.frombuffer(frame, '<i2').astype(np.float32) / 32768.0
                L = s[0::2];  R = s[1::2]
                with _lock:
                    _LEFT[:]  = L
                    _RIGHT[:] = R
                    _MONO[:]  = (L + R) * 0.5
        except Exception:
            break

# ─────────────────────────────────────────────────────────────────────────────
#  DSP
# ─────────────────────────────────────────────────────────────────────────────
_HANN = np.hanning(FFT_N).astype(np.float32)

def compute_fft(mono):
    if len(mono) < FFT_N:
        p = np.zeros(FFT_N, np.float32)
        p[:len(mono)] = mono
        mono = p
    return np.abs(np.fft.rfft(mono[:FFT_N] * _HANN)) / FFT_N

def build_binmap(n, fmin=30., fmax=18000.):
    freqs = np.fft.rfftfreq(FFT_N, 1. / SR)
    edges = np.logspace(math.log10(fmin), math.log10(fmax), n + 1)
    lo    = np.searchsorted(freqs, edges[:-1]).clip(1, len(freqs) - 2)
    hi    = np.searchsorted(freqs, edges[1:] ).clip(2, len(freqs) - 1)
    return lo, np.maximum(hi, lo + 1)

def spec_to_bars(spec, lo, hi):
    n   = len(lo)
    raw = np.array([np.sqrt(np.mean(spec[lo[i]:hi[i]] ** 2)) for i in range(n)])
    with np.errstate(divide='ignore'):
        db = 20. * np.log10(np.maximum(raw, 1e-9))
    return np.clip((db - DB_MIN) / (DB_MAX - DB_MIN), 0., 1.)

# ─────────────────────────────────────────────────────────────────────────────
#  HELPERS
# ─────────────────────────────────────────────────────────────────────────────
def _pad(lines, rows, cols):
    while len(lines) < rows:
        lines.append(' ' * cols)
    return lines[:rows]

# ─────────────────────────────────────────────────────────────────────────────
#  BASE VISUALIZER
# ─────────────────────────────────────────────────────────────────────────────
class Viz:
    NAME = 'base'
    DESC = ''

    def __init__(self, rows, cols):
        self.rows = rows
        self.cols = cols
        self.sm   = np.zeros(cols, np.float32)
        self.pk   = np.zeros(cols, np.float32)
        self.pkt  = np.zeros(cols, np.float32)
        self._lo, self._hi = build_binmap(cols)
        self._init(rows, cols)

    def _init(self, rows, cols):
        pass

    def resize(self, rows, cols):
        if cols != self.cols:
            self.sm  = np.zeros(cols, np.float32)
            self.pk  = np.zeros(cols, np.float32)
            self.pkt = np.zeros(cols, np.float32)
            self._lo, self._hi = build_binmap(cols)
        old_r, old_c = self.rows, self.cols
        self.rows = rows
        self.cols = cols
        self._on_resize(old_r, old_c, rows, cols)

    def _on_resize(self, or_, oc, rows, cols):
        pass

    def _update_bars(self, mono, dt):
        spec = compute_fft(mono)
        norm = spec_to_bars(spec, self._lo, self._hi)
        rise = norm > self.sm
        a    = np.where(rise, RISE, FALL)
        self.sm  = a * self.sm + (1. - a) * norm
        new      = self.sm > self.pk
        self.pk[new]  = self.sm[new]
        self.pkt[new] = 0.
        self.pkt += dt
        drop = self.pkt > PEAK_HOLD
        self.pk[drop] -= PEAK_DRP
        self.pk = np.clip(self.pk, 0., 1.)

    def tick(self, L, R, mono, dt):
        self._update_bars(mono, dt)
        self._tick(L, R, mono, dt)

    def _tick(self, L, R, mono, dt):
        pass

    def frame(self, rows, cols, fps) -> list:
        return [' ' * cols] * rows

    def _sbar(self, cols, fps, extra=''):
        s = f' {fps:4.0f} fps | {self.NAME}{extra} | {MONITOR[:max(1, cols - 30)]}'
        return DIM + fg(240) + s[:cols].ljust(cols) + RST

    def _hline(self, cols, color=238):
        return DIM + fg(color) + '-' * cols + RST

    def _title(self, cols, text, color=255):
        pad = (cols - len(text)) // 2
        return BLD + fg(color) + ' ' * max(0, pad) + text + RST


# =============================================================================
#  1. SPECTRUM  —  classic log-spaced vertical bars
# =============================================================================
class SpectrumViz(Viz):
    NAME = 'spectrum'
    DESC = 'Classic log-spaced frequency bars'

    def frame(self, rows, cols, fps):
        n   = cols
        vis = max(rows - 5, 4)
        lines = [
            self._title(cols, ' SPECTRUM ANALYZER '),
            self._hline(cols),
        ]
        for row in range(vis - 1, -1, -1):
            thr   = row / vis
            parts = []
            for bi in range(n):
                bh   = float(self.sm[bi])
                ph   = float(self.pk[bi])
                frac = bi / max(n - 1, 1)
                code = specgrad(frac)
                pkr  = int(ph * vis) - 1
                if bh >= thr:
                    ht  = thr
                    pfx = (BLD if ht > 0.75 else DIM if ht < 0.25 else '') + fg(code)
                    parts.append(pfx + '|' + RST)
                elif row == pkr and ph > 0.03:
                    parts.append(BLD + fg(code) + '*' + RST)
                else:
                    parts.append(' ')
            lines.append(''.join(parts))

        lines.append(self._hline(cols))

        lr = [' '] * cols
        for freq, lbl in [(30,'30'),(60,'60'),(125,'125'),(250,'250'),
                          (500,'500'),(1000,'1k'),(2000,'2k'),(4000,'4k'),
                          (8000,'8k'),(16000,'16k')]:
            f = (math.log10(freq) - math.log10(30)) / (math.log10(18000) - math.log10(30))
            c = int(f * (n - 1)) % cols
            for i, ch in enumerate(lbl):
                if c + i < cols:
                    lr[c + i] = ch
        lines.append(fg(245) + ''.join(lr) + RST)
        lines.append(self._sbar(cols, fps))
        return _pad(lines, rows, cols)


# =============================================================================
#  2. SCOPE  —  dual-channel time-domain oscilloscope
# =============================================================================
class ScopeViz(Viz):
    NAME = 'scope'
    DESC = 'Dual-channel oscilloscope'

    def _init(self, rows, cols):
        self._L = np.zeros(CHUNK, np.float32)
        self._R = np.zeros(CHUNK, np.float32)

    def _tick(self, L, R, mono, dt):
        self._L = L.copy()
        self._R = R.copy()

    def _draw_wave(self, samples, height, color_hi, color_lo):
        cols = self.cols
        chars  = [[' '] * cols for _ in range(height)]
        colors = [[0]   * cols for _ in range(height)]
        bolds  = [[False] * cols for _ in range(height)]
        zero   = height // 2

        for c in range(cols):
            chars [zero][c] = '-'
            colors[zero][c] = 234

        if len(samples) < 2:
            return [''.join(chars[r]) for r in range(height)]

        xs   = np.linspace(0, len(samples) - 1, cols).astype(int)
        amps = samples[xs].astype(float)
        rpos = np.clip(((1. - amps) * 0.5 * (height - 1)).astype(int), 0, height - 1)

        prev = int(rpos[0])
        for xi in range(cols):
            cur  = int(rpos[xi])
            amp  = abs(float(amps[xi]))
            code = color_hi if amp > 0.45 else color_lo
            bold = amp > 0.3

            lo_r, hi_r = min(prev, cur), max(prev, cur)
            for r in range(lo_r, hi_r + 1):
                chars [r][xi] = '|' if r != cur else ('*' if bold else '.')
                colors[r][xi] = code
                bolds [r][xi] = bold and (r == cur)
            prev = cur

        lines = []
        for r in range(height):
            parts = []
            for c in range(cols):
                ch   = chars[r][c]
                code = colors[r][c]
                if code:
                    pfx = (BLD if bolds[r][c] else '') + fg(code)
                    parts.append(pfx + ch + RST)
                else:
                    parts.append(ch)
            lines.append(''.join(parts))
        return lines

    def _sep(self, cols, label, lcolor):
        lbl = f' {label} '
        ld  = 3
        rd  = max(0, cols - ld - len(lbl))
        return (DIM + fg(238) + '-' * ld + RST +
                BLD + fg(lcolor) + lbl + RST +
                DIM + fg(238) + '-' * rd + RST)

    def frame(self, rows, cols, fps):
        vis  = max(rows - 5, 4)
        half = vis // 2
        lines = [
            self._title(cols, ' OSCILLOSCOPE ', color=51),
            self._sep(cols, 'LEFT  ch.1', 51),
        ]
        lines += self._draw_wave(self._L, half, 51, 39)
        lines.append(self._sep(cols, 'RIGHT ch.2', 214))
        lines += self._draw_wave(self._R, vis - half, 214, 208)
        lines.append(self._hline(cols))
        lines.append(self._sbar(cols, fps))
        return _pad(lines, rows, cols)


# =============================================================================
#  3. MATRIX  —  audio-reactive falling-character rain
# =============================================================================
_MCHARS = ('ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz'
           '0123456789!@#$%^&*()_+-=[]{}|;:.,<>?/~`\\')
_GREEN  = [46, 40, 34, 28, 22, 238]

class MatrixViz(Viz):
    NAME = 'matrix'
    DESC = 'Audio-reactive matrix rain'

    def _init(self, rows, cols):
        self._drops = [self._new_drop(rows) for _ in range(cols)]

    def _new_drop(self, rows):
        return {
            'y':     random.uniform(-rows, 0),
            'spd':   random.uniform(0.4, 1.3),
            'trail': random.randint(5, 18),
            'seq':   [random.choice(_MCHARS) for _ in range(24)],
            'ft':    0.,
            'hue':   random.choice([46, 40, 82, 118, 51, 45]),
        }

    def _on_resize(self, or_, oc, rows, cols):
        while len(self._drops) < cols:
            self._drops.append(self._new_drop(rows))
        self._drops = self._drops[:cols]

    def _tick(self, L, R, mono, dt):
        rows, cols = self.rows, self.cols
        n = len(self.sm)
        for ci, d in enumerate(self._drops):
            band   = int(ci / cols * n)
            energy = float(self.sm[min(band, n - 1)])
            d['y'] += d['spd'] * (0.35 + energy * 2.8) * dt * rows * 0.7
            d['ft'] += dt
            if d['ft'] > 0.08:
                d['ft'] = 0.
                d['seq'][random.randrange(len(d['seq']))] = random.choice(_MCHARS)
            if d['y'] - d['trail'] > rows:
                d.update(self._new_drop(rows))

    def frame(self, rows, cols, fps):
        vis = rows - 1
        grid = {}
        for ci, d in enumerate(self._drops[:cols]):
            yh, trl, seq = d['y'], d['trail'], d['seq']
            for pos in range(trl + 1):
                r = int(yh) - pos
                if 0 <= r < vis:
                    bright = 1.0 if pos == 0 else max(0., 1. - pos / trl)
                    grid[(r, ci)] = (seq[pos % len(seq)], bright, d['hue'])

        lines = []
        for r in range(vis):
            parts = []
            for c in range(cols):
                cell = grid.get((r, c))
                if cell:
                    ch, bright, hue = cell
                    if bright >= 0.95:
                        parts.append(BLD + fg(231) + ch + RST)
                    else:
                        si    = int((1. - bright) * (len(_GREEN) - 1))
                        shade = hue if bright > 0.5 else _GREEN[min(si, len(_GREEN) - 1)]
                        parts.append(fg(shade) + ch + RST)
                else:
                    parts.append(' ')
            lines.append(''.join(parts))

        lines.append(self._sbar(cols, fps))
        return _pad(lines, rows, cols)


# =============================================================================
#  4. RADIAL  —  polar spectrum: bands radiate from centre
# =============================================================================
class RadialViz(Viz):
    NAME = 'radial'
    DESC = 'Polar spectrum radiating from centre'

    def _init(self, rows, cols):
        self._precompute(rows, cols)

    def _on_resize(self, or_, oc, rows, cols):
        self._precompute(rows, cols)

    def _precompute(self, rows, cols):
        cy = rows / 2.;  cx = cols / 2.
        ys = np.arange(rows)[:, np.newaxis].astype(np.float32)
        xs = np.arange(cols)[np.newaxis, :].astype(np.float32)
        # Compensate for character aspect ratio (~2× taller than wide)
        dy = ys - cy
        dx = (xs - cx) * 0.5
        self._r     = np.sqrt(dx ** 2 + dy ** 2)
        self._theta = np.arctan2(dy, dx)   # -π … π
        self._maxr  = float(min(cy, cx * 0.5))
        self._rnorm = self._r / max(self._maxr, 1.)

    def frame(self, rows, cols, fps):
        if rows != self.rows or cols != self.cols:
            self._precompute(rows, cols)

        n   = len(self.sm)
        bi  = (((self._theta + math.pi) / (2. * math.pi)) * n).astype(int) % n
        bh  = self.sm[bi]
        act = (self._rnorm < bh) & (self._rnorm < 1.0)

        vis = rows - 1
        cy2 = vis  // 2
        cx2 = cols // 2
        lines = []

        for r in range(vis):
            parts = []
            for c in range(cols):
                if r < act.shape[0] and act[r, c]:
                    rn   = float(self._rnorm[r, c])
                    frac = int(bi[r, c]) / max(n - 1, 1)
                    code = specgrad(frac)
                    if   rn < 0.12: ch = '@'
                    elif rn < 0.28: ch = '#'
                    elif rn < 0.48: ch = '*'
                    elif rn < 0.68: ch = '+'
                    elif rn < 0.84: ch = '.'
                    else:           ch = '`'
                    parts.append((BLD if rn < 0.35 else '') + fg(code) + ch + RST)
                else:
                    on_x = (r == cy2)
                    on_y = (c == cx2)
                    if on_x and on_y:
                        parts.append(DIM + fg(237) + '+' + RST)
                    elif on_x or on_y:
                        dist = abs(r - cy2) + abs(c - cx2)
                        if dist > 2:
                            parts.append(DIM + fg(234) + ('|' if on_y else '-') + RST)
                        else:
                            parts.append(' ')
                    else:
                        parts.append(' ')
            lines.append(''.join(parts))

        lines.append(self._sbar(cols, fps))
        return _pad(lines, rows, cols)


# =============================================================================
#  5. LISSAJOUS  —  full-terminal X-Y scope with beat rotation & inner detail
#
#  Layers (back to front):
#   1. Spectrum shell  — polar frequency ring at the outer edge
#   2. Orbit rings     — 3 faint concentric reference ellipses
#   3. Persistence grid— the actual L/R Lissajous trail with chromatic fade
#   4. Inner detail    — RMS-scaled radial spokes + phase-dot constellation
#   5. Beat ripple     — expanding ring pulse from centre on transients
#
#  Rotation:
#   The entire XY signal is rotated in signal-space by an angle that
#   accumulates based on a beat-onset detector.  On each detected beat
#   the angular velocity gets a kick; it then decays back to a slow
#   baseline drift.  Net effect: the figure spins freely with the music.
# =============================================================================

# Colour palettes for the trace
_LP_DEEP  = [17, 18, 19, 20, 21]          # dark blue (oldest)
_LP_MID   = [27, 33, 39, 45, 51]          # cyan
_LP_BRIGHT= [159, 195, 231]               # white-cyan (freshest)
_LP_ALL   = _LP_DEEP + _LP_MID + _LP_BRIGHT

# Chromatic accent palette that cycles with the rotation angle
_LP_HUE   = [196,202,208,214,220,226,154,118,82,46,51,45,39,33,27,21,57,93,129,165,201]

class LissajousViz(Viz):
    NAME = 'lissajous'
    DESC = 'Full-terminal XY scope — beat-driven rotation, inner detail'

    # ── init / resize ─────────────────────────────────────────────────────────
    def _init(self, rows, cols):
        vis = max(rows - 1, 1)
        # Persistence buffer — float brightness [0,1]
        self._grid  = np.zeros((vis, cols), np.float32)
        # Separate "age" layer for chromatic colouring: 0=fresh … 1=old
        self._age   = np.ones((vis, cols), np.float32)

        self._L = np.zeros(CHUNK, np.float32)
        self._R = np.zeros(CHUNK, np.float32)

        # ── rotation state ────────────────────────────────────────────────────
        self._rot_angle   = 0.0   # current signal-space rotation (radians)
        self._rot_vel     = 0.02  # rad/s baseline drift
        self._rot_vel_max = 3.8   # rad/s cap
        self._rot_baseline= 0.02  # rad/s idle drift

        # ── beat detector state ───────────────────────────────────────────────
        # Running average of bass RMS energy for onset comparison
        self._beat_avg    = 0.0
        self._beat_alpha  = 0.15   # smoothing for average
        self._beat_thresh = 1.55   # ratio above avg that triggers a beat
        self._beat_min_dt = 0.18   # minimum seconds between beats (≈ 333 BPM max)
        self._last_beat   = 0.0    # time of last beat
        # Ripple list — each entry: [radius, brightness]
        # radius: normalised 0..1+ (expands outward from centre)
        # brightness: 1.0 at spawn, fades as ring travels
        self._ripples     = []

        # ── inner detail state ────────────────────────────────────────────────
        self._spoke_phase = 0.0   # animates spoke rotation independently
        self._rms_smooth  = 0.0
        self._phase_dots  = [(random.uniform(0, 2*math.pi),
                              random.uniform(0.15, 0.42))
                             for _ in range(24)]  # (angle, radius_frac)

        # ── hue ───────────────────────────────────────────────────────────────
        self._hue_t = 0.0          # drives accent colour cycle

        # ── geometry caches (built lazily in frame, invalidated on resize) ─────
        self._ring_cache   = None   # list of (rc, cc, ring_col) per ring
        self._shell_sincos = None   # (sin_a, cos_a) array for shell sectors
        self._shell_n      = 0      # n_spec value the shell cache was built for

        # ── vocal stars ───────────────────────────────────────────────────────
        # Each star: {a, r, vr, life, max_life, col}
        #   a        = angle (radians, in screen-polar space, fixed at spawn)
        #   r        = current normalised radius (0=centre … 1=edge)
        #   vr       = radial velocity (normalised units / second)
        #   life     = remaining life [0,1], fades as star travels
        #   max_life = total life at spawn (seconds), for colour interpolation
        #   col      = 256-colour code
        self._vocal_stars   = []
        self._vocal_energy  = 0.0   # smoothed vocal-band RMS
        self._vocal_prev    = 0.0   # one frame ago, for onset detection
        self._vocal_avg     = 0.0   # slow background average for onset ratio
        # Pre-compute vocal FFT bin range (300–3400 Hz)
        _freqs = np.fft.rfftfreq(FFT_N, 1. / SR)
        self._vlo = int(np.searchsorted(_freqs, 300.))
        self._vhi = int(np.searchsorted(_freqs, 3400.))

        # ── planets ───────────────────────────────────────────────────────────
        # Number of planets scales with terminal size (3 small … 6 large).
        # Each planet orbits at a fixed normalised radius and an angular
        # velocity driven by the energy in its assigned frequency band.
        # Innermost = highest frequencies (fast), outermost = bass (slow).
        #
        # Planet dict keys:
        #   angle    – current orbital angle (radians)
        #   orbit_r  – normalised orbit radius (0..1)
        #   band_lo  – Hz lower edge of frequency band
        #   band_hi  – Hz upper edge of frequency band
        #   lo_bin   – FFT bin index for band_lo
        #   hi_bin   – FFT bin index for band_hi
        #   energy   – smoothed band energy [0..1]
        #   col      – 256-colour code (fixed per planet)
        #   trail    – list of (angle, alpha) for dot trail (newest first)
        #   trail_len– max trail entries kept
        #
        # Frequency bands from inside out (highest → lowest):
        #   inner:  4000–12000 Hz  (presence / air / hi-hat)
        #   …
        #   outer:  40–150 Hz      (sub-bass / kick)
        _planet_bands = [
            (4000., 12000., 0.20, 141),   # innermost – hi-freq – magenta
            (1500.,  4000., 0.35,  51),   # cyan
            ( 500.,  1500., 0.50, 226),   # yellow
            ( 150.,   500., 0.65,  82),   # green
            (  40.,   150., 0.80, 196),   # red  (bass)
            (  20.,    40., 0.92,  57),   # violet (sub-bass) – outermost
        ]
        _freqs2 = np.fft.rfftfreq(FFT_N, 1. / SR)
        # How many planets to show: scale with terminal area
        term_area = rows * cols
        if   term_area < 2000:  n_planets = 3
        elif term_area < 6000:  n_planets = 4
        elif term_area < 12000: n_planets = 5
        else:                   n_planets = 6
        self._planets = []
        for i in range(n_planets):
            blo, bhi, orbit_r, col = _planet_bands[i]
            lo_bin = int(np.searchsorted(_freqs2, blo))
            hi_bin = int(np.searchsorted(_freqs2, bhi))
            hi_bin = max(hi_bin, lo_bin + 1)
            self._planets.append({
                'angle':     random.uniform(0., 2. * math.pi),
                'orbit_r':   orbit_r,
                'band_lo':   blo,
                'band_hi':   bhi,
                'lo_bin':    lo_bin,
                'hi_bin':    hi_bin,
                'energy':    0.0,
                'col':       col,
                'trail':     deque(maxlen=18),
            })

    def _on_resize(self, or_, oc, rows, cols):
        vis = max(rows - 1, 1)
        self._grid = np.zeros((vis, cols), np.float32)
        self._age  = np.ones( (vis, cols), np.float32)
        self._ring_cache   = None   # invalidate on resize
        self._shell_sincos = None   # invalidate on resize

    # ── per-frame audio processing ────────────────────────────────────────────
    def _tick(self, L, R, mono, dt):
        self._L = L.copy()
        self._R = R.copy()

        now = time.perf_counter()

        # ── Beat detection via bass-band RMS onset ────────────────────────────
        # Use a broader window so the bass RMS is stable
        bass_rms = float(np.sqrt(np.mean(mono ** 2) + 1e-9))
        self._beat_avg = (self._beat_alpha * bass_rms
                          + (1. - self._beat_alpha) * self._beat_avg)

        is_beat = (bass_rms > self._beat_thresh * self._beat_avg
                   and now - self._last_beat > self._beat_min_dt
                   and bass_rms > 0.01)

        if is_beat:
            self._last_beat = now
            # Kick the angular velocity; direction alternates with phase for
            # a natural feel instead of always spinning the same way
            kick_dir   = 1.0 if math.sin(self._rot_angle * 3.1) >= 0 else -1.0
            kick_mag   = 0.8 + bass_rms * 4.0
            self._rot_vel = max(-self._rot_vel_max,
                                min(self._rot_vel_max,
                                    self._rot_vel + kick_dir * kick_mag))
            # Spawn an expanding ripple from the centre
            self._ripples.append([0.0, 1.0])

        # Decay velocity back toward baseline (signed)
        sign = 1.0 if self._rot_vel >= 0 else -1.0
        decay_rate    = 1.8   # rad/s²
        new_vel       = self._rot_vel - sign * decay_rate * dt
        # Don't overshoot the baseline
        if abs(new_vel) < self._rot_baseline:
            new_vel = self._rot_baseline
        self._rot_vel = new_vel

        # Advance rotation angle
        self._rot_angle = (self._rot_angle + self._rot_vel * dt) % (2 * math.pi)

        # Advance ripples outward and fade them
        for rp in self._ripples:
            rp[0] += dt * 1.4   # expand ~0.7 screen widths/sec
            rp[1] -= dt * 2.2   # fade out
        self._ripples = [rp for rp in self._ripples
                         if rp[1] > 0.0 and rp[0] < 1.3]

        # Hue cycle (tied to rotation for synchrony)
        self._hue_t = (self._rot_angle / (2 * math.pi)) % 1.0

        # ── Spoke phase ───────────────────────────────────────────────────────
        self._spoke_phase = (self._spoke_phase + dt * 0.35) % (2 * math.pi)

        # ── RMS for inner detail sizing ───────────────────────────────────────
        rms = float(np.sqrt(np.mean(mono ** 2)))
        self._rms_smooth = 0.7 * self._rms_smooth + 0.3 * rms

        # ── Single FFT — shared by vocal-band and planet-band analysis ─────────
        spec_v   = compute_fft(mono)   # computed once, reused below
        vlo, vhi = self._vlo, self._vhi
        v_rms    = float(np.sqrt(np.mean(spec_v[vlo:vhi] ** 2) + 1e-12)) * 60.
        # Smooth current energy (fast attack, slower decay)
        a_v      = 0.55 if v_rms > self._vocal_energy else 0.20
        self._vocal_energy = a_v * v_rms + (1. - a_v) * self._vocal_energy
        # Very slow background average for onset ratio
        self._vocal_avg = 0.02 * self._vocal_energy + 0.98 * self._vocal_avg

        # Onset: current energy suddenly exceeds background by threshold
        onset_ratio = (self._vocal_energy / max(self._vocal_avg, 1e-6))
        is_vocal_onset = onset_ratio > 1.35 and self._vocal_energy > 0.04

        # Spawn stars on onset — number scales with how dramatic the jump is
        if is_vocal_onset:
            n_new = int(min(6, 1 + (onset_ratio - 1.35) * 10))
            for _ in range(n_new):
                # Random angle, fixed in screen space (not co-rotating)
                angle = random.uniform(0., 2. * math.pi)
                speed = 0.18 + self._vocal_energy * 0.55 + random.uniform(0., 0.12)
                life  = 0.6 + random.uniform(0., 0.5)
                # Warm palette: white → yellow → orange for vocal stars
                col   = random.choice([231, 230, 229, 228, 227, 226, 220, 214])
                self._vocal_stars.append({
                    'a': angle, 'r': 0.02,
                    'vr': speed, 'life': life, 'max_life': life, 'col': col,
                })

        # Also continuously emit a trickle while vocals are present
        if self._vocal_energy > 0.06 and random.random() < self._vocal_energy * 0.4:
            angle = random.uniform(0., 2. * math.pi)
            speed = 0.10 + self._vocal_energy * 0.30
            life  = 0.4 + random.uniform(0., 0.3)
            col   = random.choice([195, 159, 231, 230, 229])
            self._vocal_stars.append({
                'a': angle, 'r': 0.01,
                'vr': speed, 'life': life, 'max_life': life, 'col': col,
            })

        # Integrate star positions and age them out
        surviving = []
        for s in self._vocal_stars:
            s['r']    += s['vr'] * dt
            s['life'] -= dt
            if s['life'] > 0. and s['r'] < 1.05:
                surviving.append(s)
        self._vocal_stars = surviving

        self._vocal_prev = self._vocal_energy

        # ── Planets — update band energy + advance orbital angle ──────────────
        for p in self._planets:   # reuse spec_v from above (same FFT result)
            lo, hi = p['lo_bin'], p['hi_bin']
            raw_e  = float(np.sqrt(np.mean(spec_v[lo:hi] ** 2) + 1e-12)) * 80.
            raw_e  = min(raw_e, 1.0)
            # Smooth energy: fast attack, slower decay
            a_p    = 0.50 if raw_e > p['energy'] else 0.15
            p['energy'] = a_p * raw_e + (1. - a_p) * p['energy']

            # Angular velocity: baseline + audio kick
            # Inner orbits spin faster baseline; energy adds on top.
            # orbit_r 0.20 → baseline ~0.55 rad/s; 0.92 → ~0.08 rad/s
            baseline_omega = 0.55 * (1. - p['orbit_r']) + 0.06
            omega  = baseline_omega + p['energy'] * 1.8
            old_angle = p['angle']
            p['angle'] = (p['angle'] + omega * dt) % (2. * math.pi)

            # Record trail: store old angle with full alpha; decay existing entries
            trail = p['trail']
            trail.appendleft([old_angle, 1.0])
            # Decay alpha of all entries (numpy-free; list is short ≤18)
            for t in trail:
                t[1] *= 0.82
            # (deque handles maxlen automatically; prune below threshold)
            while p['trail'] and p['trail'][-1][1] < 0.05:
                p['trail'].pop()

        # ── Update persistence grid ───────────────────────────────────────────
        vis  = max(self.rows - 1, 1)
        cols = self.cols
        cx   = (cols - 1) / 2.
        cy   = (vis  - 1) / 2.

        # Rotate the XY signal by rot_angle before plotting
        ca = math.cos(self._rot_angle)
        sa = math.sin(self._rot_angle)
        Lf = self._L.astype(np.float64)
        Rf = self._R.astype(np.float64)
        Xr =  ca * Lf + sa * Rf   # rotated X  (maps to screen X)
        Yr = -sa * Lf + ca * Rf   # rotated Y  (maps to screen Y, inverted)

        # Scale: use 96% of the half-extents so the figure fills the terminal
        # Correct for the ~2:1 character aspect ratio on the Y axis
        half_x = cx * 0.96
        half_y = cy * 0.96

        xi = np.clip((Xr * half_x + cx).astype(int), 0, cols - 1)
        yi = np.clip((-Yr * half_y + cy).astype(int), 0, vis  - 1)

        # Decay brightness and age old points
        decay = 0.84 - self._rms_smooth * 0.12   # louder → faster decay = more dramatic
        decay = max(0.72, min(0.92, decay))
        self._grid *= decay
        self._age   = np.minimum(self._age + dt * 0.9, 1.0)

        # Stamp new points bright & fresh
        self._grid[yi, xi] = 1.0
        self._age  [yi, xi] = 0.0

        # Anti-alias: 4-neighbour sub-pixel spread
        for dy2, dx2, w in ((-1,0,.55),(1,0,.55),(0,-1,.45),(0,1,.45)):
            ny = np.clip(yi + dy2, 0, vis  - 1)
            nx = np.clip(xi + dx2, 0, cols - 1)
            mask = self._grid[ny, nx] < w
            self._grid[ny, nx] = np.where(mask, w, self._grid[ny, nx])
            self._age [ny, nx] = np.where(mask, 0.1, self._age[ny, nx])

    # ── render ────────────────────────────────────────────────────────────────
    def frame(self, rows, cols, fps):
        vis  = max(rows - 1, 1)
        cx   = (cols - 1) / 2.
        cy   = (vis  - 1) / 2.
        icx  = cols // 2
        icy  = vis  // 2

        g   = self._grid[:vis, :cols]
        age = self._age [:vis, :cols]

        # Accent colour from hue cycle
        hi      = int(self._hue_t * len(_LP_HUE)) % len(_LP_HUE)
        accent  = _LP_HUE[hi]
        accent2 = _LP_HUE[(hi + len(_LP_HUE) // 3) % len(_LP_HUE)]

        # ── Pre-build inner-detail overlay (sparse dict for speed) ───────────
        detail = {}   # (r,c) -> (char, color_code, bold)

        rx_full = cx * 0.96
        ry_full = cy * 0.96

        # 1. Orbit reference rings — cached geometry (rebuild only on resize)
        if self._ring_cache is None:
            cache = []
            for ring_frac, ring_col in ((0.25, 235), (0.52, 236), (0.80, 237)):
                rx = rx_full * ring_frac
                ry = ry_full * ring_frac
                steps = max(int((rx + ry) * 2.5), 64)
                angles = np.linspace(0., 2. * math.pi, steps, endpoint=False)
                rcs = (icy - np.sin(angles) * ry + 0.5).astype(int)
                ccs = (icx + np.cos(angles) * rx + 0.5).astype(int)
                mask = (rcs >= 0) & (rcs < vis) & (ccs >= 0) & (ccs < cols)
                for rc_, cc_ in zip(rcs[mask], ccs[mask]):
                    cache.append((int(rc_), int(cc_), ring_col))
            self._ring_cache = cache
        for rc_, cc_, ring_col in self._ring_cache:
            if (rc_, cc_) not in detail:
                detail[(rc_, cc_)] = ('.', ring_col, False)

        # 2. Radial spokes — vectorised per spoke (8 × 18 = 144 trig ops → numpy)
        spoke_len = 0.10 + self._rms_smooth * 0.50
        n_spokes  = 8
        fracs = np.linspace(0.03, spoke_len, 18)
        for si in range(n_spokes):
            a        = self._spoke_phase + si * (2. * math.pi / n_spokes)
            sin_a    = math.sin(a)
            cos_a    = math.cos(a)
            abs_sin  = abs(sin_a)
            rcs = (icy - sin_a * ry_full * fracs + 0.5).astype(int)
            ccs = (icx + cos_a * rx_full * fracs + 0.5).astype(int)
            brights = 1.0 - fracs / spoke_len
            ch_base = '|' if abs_sin > 0.7 else '-'
            for j in range(len(fracs)):
                rc_, cc_ = int(rcs[j]), int(ccs[j])
                if 0 <= rc_ < vis and 0 <= cc_ < cols:
                    bright = float(brights[j])
                    ch  = '+' if fracs[j] < 0.06 else ch_base
                    col = accent if bright > 0.7 else accent2 if bright > 0.4 else 238
                    detail[(rc_, cc_)] = (ch, col, bright > 0.6)

        # 3. Phase-dot constellation — 24 dots fixed to signal-space, co-rotate
        rms = self._rms_smooth
        for (base_a, r_frac) in self._phase_dots:
            a    = base_a + self._rot_angle
            rdot = r_frac * (0.6 + rms * 0.9)
            xd   =  math.cos(a) * rdot
            yd   = -math.sin(a) * rdot
            rc   = int(icy + yd * ry_full + 0.5)
            cc   = int(icx + xd * rx_full + 0.5)
            if 0 <= rc < vis and 0 <= cc < cols:
                col = accent if r_frac < 0.28 else accent2
                detail[(rc, cc)] = ('*', col, True)

        # 4. Dead-centre nucleus — scales with RMS
        nuc_r = int(self._rms_smooth * 3.5 + 0.5)   # 0..3 rows
        for dr in range(-nuc_r, nuc_r + 1):
            for dc in range(-nuc_r, nuc_r + 1):
                dist = math.sqrt(dr**2 + (dc * 0.5)**2)
                if dist <= nuc_r + 0.5:
                    rc, cc = icy + dr, icx + dc
                    if 0 <= rc < vis and 0 <= cc < cols:
                        ch  = '@' if dist < 0.8 else '#' if dist < 1.5 else '*'
                        detail[(rc, cc)] = (ch, accent, True)

        # 5. Vocal stars — particles spawned by voice/vocal-range energy,
        #    travelling outward from the nucleus in screen-polar space.
        #    Fixed angle (not co-rotating) so they streak away from centre.
        for s in self._vocal_stars:
            life_frac = s['life'] / max(s['max_life'], 1e-6)   # 1=fresh 0=dying
            r_s = s['r']
            a_s = s['a']
            # Screen coords: compensate for char aspect ratio on Y
            xd  =  math.cos(a_s) * r_s
            yd  = -math.sin(a_s) * r_s
            rc  = int(icy + yd * ry_full + 0.5)
            cc  = int(icx + xd * rx_full + 0.5)
            if 0 <= rc < vis and 0 <= cc < cols:
                # Character: bright '*' when fresh, fades to '+' then '.'
                if life_frac > 0.65:
                    ch = '*'
                elif life_frac > 0.30:
                    ch = '+'
                else:
                    ch = '.'
                # Colour: star's own warm hue, dims as it dies
                # Override whatever was already in detail so stars are visible
                col  = s['col']
                bold = life_frac > 0.50
                detail[(rc, cc)] = (ch, col, bold)

            # Also paint a short trail one step behind to give motion sense
            trail_r = max(0., r_s - s['vr'] * 0.04)
            xd2 =  math.cos(a_s) * trail_r
            yd2 = -math.sin(a_s) * trail_r
            rc2 = int(icy + yd2 * ry_full + 0.5)
            cc2 = int(icx + xd2 * rx_full + 0.5)
            if (0 <= rc2 < vis and 0 <= cc2 < cols
                    and (rc2, cc2) not in detail
                    and life_frac > 0.40):
                detail[(rc2, cc2)] = ('.', s['col'], False)

        # 6. Planets — orbiting bodies with dot trails
        #    Rendered AFTER vocal stars (overwrite them) and BEFORE spectrum
        #    shell so the shell sits on top.  The Lissajous trace overwrites
        #    planets when the signal is bright, which looks intentional.
        for p in self._planets:
            orb_r = p['orbit_r']
            col_p = p['col']

            # ── Trail dots (behind the planet head) ───────────────────────────
            for t_angle, t_alpha in p['trail']:
                xd = math.cos(t_angle) * orb_r
                yd = math.sin(t_angle) * orb_r
                rc = int(icy - yd * ry_full + 0.5)
                cc = int(icx + xd * rx_full + 0.5)
                if 0 <= rc < vis and 0 <= cc < cols:
                    # Fade trail through dim colour codes toward background
                    if t_alpha > 0.65:
                        trail_col = col_p
                        trail_bold = False
                    elif t_alpha > 0.35:
                        trail_col = 240
                        trail_bold = False
                    else:
                        trail_col = 236
                        trail_bold = False
                    # Only write if this cell isn't already a brighter trail dot
                    existing = detail.get((rc, cc))
                    if existing is None or existing[0] == '.':
                        detail[(rc, cc)] = ('.', trail_col, trail_bold)

            # ── Planet head — "o" character ───────────────────────────────────
            xd = math.cos(p['angle']) * orb_r
            yd = math.sin(p['angle']) * orb_r
            rc = int(icy - yd * ry_full + 0.5)
            cc = int(icx + xd * rx_full + 0.5)
            if 0 <= rc < vis and 0 <= cc < cols:
                detail[(rc, cc)] = ('o', col_p, True)

        # 7. Beat ripples — expanding concentric rings from the centre.
        #    Each ripple is a thin ellipse drawn into the detail dict.
        #    Brightness fades and character lightens as the ring expands.
        #    Uses numpy vectorisation: one np.sin/cos call per ripple.
        for rp_r, rp_b in self._ripples:
            if rp_b <= 0. or rp_r <= 0.:
                continue
            # Character and colour based on brightness
            if rp_b > 0.70:
                rp_ch  = 'o'
                rp_col = accent
                rp_bold= True
            elif rp_b > 0.35:
                rp_ch  = '+'
                rp_col = accent2
                rp_bold= False
            else:
                rp_ch  = '.'
                rp_col = _LP_MID[min(int(rp_b * len(_LP_MID)), len(_LP_MID)-1)]
                rp_bold= False
            # Vectorised ellipse at radius rp_r (corrected for char aspect ratio)
            rx_rp = rx_full * rp_r
            ry_rp = ry_full * rp_r
            steps_rp = max(int((rx_rp + ry_rp) * 3.0), 48)
            if steps_rp < 4:
                continue
            a_arr  = np.linspace(0., 2. * math.pi, steps_rp, endpoint=False)
            rcs_rp = (icy - np.sin(a_arr) * ry_rp + 0.5).astype(int)
            ccs_rp = (icx + np.cos(a_arr) * rx_rp + 0.5).astype(int)
            mask_rp = (rcs_rp >= 0) & (rcs_rp < vis) & (ccs_rp >= 0) & (ccs_rp < cols)
            for rc_rp, cc_rp in zip(rcs_rp[mask_rp].tolist(),
                                     ccs_rp[mask_rp].tolist()):
                # Ripples overwrite background detail but not planets/stars
                existing = detail.get((rc_rp, cc_rp))
                if existing is None or existing[0] in ('.', '-', '|'):
                    detail[(rc_rp, cc_rp)] = (rp_ch, rp_col, rp_bold)

        # ── Spectrum shell (outermost ring, log-spaced frequency) ─────────────
        # sin/cos per band are geometry — cached; only the energy varies.
        n_spec = len(self.sm)
        if self._shell_sincos is None or self._shell_n != n_spec:
            angles = (np.arange(n_spec) * (2. * math.pi / n_spec)
                      - math.pi / 2.)
            self._shell_sincos = np.stack([np.sin(angles), np.cos(angles)])
            self._shell_n      = n_spec
        sin_sh, cos_sh = self._shell_sincos   # shape (n_spec,) each
        shell_r   = 0.94
        shell_fracs = np.linspace(0., 0.10, 5)   # relative tick lengths
        for si in range(n_spec):
            e = float(self.sm[si])
            if e < 0.01:
                continue   # skip silent bands entirely
            code  = specgrad(si / max(n_spec - 1, 1))
            bold  = e > 0.6
            t_len = e * 0.10
            for df in shell_fracs:
                frac = shell_r + df * (t_len / 0.10)
                rc   = int(icy - sin_sh[si] * ry_full * frac + 0.5)
                cc   = int(icx + cos_sh[si] * rx_full * frac + 0.5)
                if 0 <= rc < vis and 0 <= cc < cols:
                    detail[(rc, cc)] = ('|', code, bold)

        # ── Compose frame ─────────────────────────────────────────────────────
        # Pre-build a flat ANSI string for each cell of the Lissajous grid,
        # then overwrite with detail-dict cells — avoiding per-cell Python
        # attribute lookups in the inner loop.
        #
        # Strategy:
        #   1. Compute b_eff for entire grid at once (numpy).
        #   2. For grid-active cells, vectorise colour+char selection.
        #   3. Splat the resulting per-cell strings into row buffers.
        #   4. Overwrite only the (sparse) detail-dict cells.

        b_eff_grid = np.clip(g[:vis, :cols], 0., 1.)
        age_grid   = age[:vis, :cols]

        # -- Vectorised colour index for trace cells --
        # age < 0.15 → accent, 0.15-0.45 → LP_MID, else → LP_DEEP
        n_mid  = len(_LP_MID)
        n_deep = len(_LP_DEEP)
        age_f  = age_grid                              # shape (vis, cols)
        mid_i  = np.clip((age_f * n_mid).astype(int),  0, n_mid  - 1)
        deep_i = np.clip((age_f * n_deep).astype(int), 0, n_deep - 1)
        # colour code per cell (only used where b_eff > 0.06)
        col_grid = np.where(age_f < 0.15, accent,
                   np.where(age_f < 0.45,
                            np.array(_LP_MID, dtype=np.int32)[mid_i],
                            np.array(_LP_DEEP, dtype=np.int32)[deep_i]))

        # char selection thresholds → integer tag 0-4
        # 4=@ 3=# 2=* 1=+ 0=.
        char_tag = np.zeros(b_eff_grid.shape, dtype=np.int8)
        char_tag[b_eff_grid > 0.20] = 1
        char_tag[b_eff_grid > 0.40] = 2
        char_tag[b_eff_grid > 0.65] = 3
        char_tag[b_eff_grid > 0.88] = 4
        _CHARS = ['.', '+', '*', '#', '@']

        # active mask
        active = b_eff_grid > 0.06

        # Group detail dict by row so the inner loop is O(detail/row)
        # instead of O(detail_total) per row.
        detail_by_row = {}
        for (dr, dc), (dch, dcode, dbold) in detail.items():
            if 0 <= dr < vis:
                detail_by_row.setdefault(dr, []).append((dc, dch, dcode, dbold))

        # Build row strings
        lines = []
        for r in range(vis):
            act_row  = active[r]
            brow     = b_eff_grid[r]
            crow     = col_grid[r]
            ctrow    = char_tag[r]

            # Start with a list of single chars/spaces for this row
            # We use a bytearray-sized list and join at the end.
            row_parts = [''] * cols

            # Fill active (trace) cells
            act_cols = np.where(act_row)[0]
            for c in act_cols:
                c = int(c)
                code = int(crow[c])
                ch   = _CHARS[int(ctrow[c])]
                pfx  = BLD if brow[c] > 0.70 else ''
                row_parts[c] = pfx + _FG[code] + ch + RST

            # Overwrite with detail-dict cells for this row
            # (detail_by_row is pre-grouped once outside the loop — O(1) lookup)
            for dc, dch, dcode, dbold in detail_by_row.get(r, ()):
                if 0 <= dc < cols and not act_row[dc]:
                    row_parts[dc] = (BLD if dbold else DIM) + _FG[dcode] + dch + RST

            # Fill remaining empty slots with space
            for c in range(cols):
                if row_parts[c] == '':
                    row_parts[c] = ' '

            lines.append(''.join(row_parts))

        # ── Status bar ────────────────────────────────────────────────────────
        vel_deg = self._rot_vel * 180 / math.pi
        ang_deg = int(self._rot_angle * 180 / math.pi) % 360
        beat_ind = fg(accent) + BLD + '●' + RST if self._ripples else ' '
        extra = f' | {beat_ind} {ang_deg:3d}° {vel_deg:+.1f}°/s'
        lines.append(self._sbar(cols, fps, extra=extra))
        return _pad(lines, rows, cols)


# =============================================================================
#  6. FIRE  —  audio-reactive rising ASCII fire
# =============================================================================
_FIRE_PAL   = [232, 52, 88, 124, 160, 196, 202, 208, 214, 220, 226, 227, 228, 229, 230, 231]
_FIRE_CHARS = ' .`^\' |*#$@'

class FireViz(Viz):
    NAME = 'fire'
    DESC = 'Audio-reactive ASCII fire'

    def _init(self, rows, cols):
        self._heat = np.zeros((rows, cols), np.float32)

    def _on_resize(self, or_, oc, rows, cols):
        self._heat = np.zeros((rows, cols), np.float32)

    def _tick(self, L, R, mono, dt):
        rows, cols = self.rows, self.cols
        n    = len(self.sm)
        bass = float(np.mean(self.sm[:max(1, n // 6)]))
        mid  = float(np.mean(self.sm[n // 6:n // 3]))
        bot  = rows - 2   # bottom seeding row

        if bot < 0:
            return

        # Seed bottom row: always slightly hot, blazes with audio
        base   = 0.12 + bass * 1.3 + mid * 0.25
        noise  = np.random.uniform(0.5, 1.5, cols).astype(np.float32)
        self._heat[bot, :] = np.clip(base * noise, 0., 1.)

        # Per-column intensity from spectrum gives varied flames
        col_e = self.sm.copy()
        if len(col_e) != cols:
            col_e = np.interp(np.linspace(0, 1, cols),
                              np.linspace(0, 1, len(col_e)), col_e).astype(np.float32)
        self._heat[bot, :] = np.clip(self._heat[bot, :] + col_e * 0.4, 0., 1.)

        # Propagate heat upward (vectorised)
        if bot > 0:
            below   = self._heat[1:bot + 1, :]
            bl      = np.empty_like(below)
            br      = np.empty_like(below)
            bl[:, 1:]  = below[:, :-1];  bl[:, 0]  = below[:, 0]
            br[:, :-1] = below[:, 1:];   br[:, -1] = below[:, -1]
            avg    = (below * 2. + bl + br) / 4.
            flick  = np.random.uniform(0., 0.025, avg.shape).astype(np.float32)
            self._heat[:bot, :] = np.clip(avg * 0.92 - flick, 0., 1.)

    def frame(self, rows, cols, fps):
        vis  = rows - 1
        heat = self._heat
        lines = []
        for r in range(vis):
            parts = []
            for c in range(cols):
                h = float(heat[r, c]) if r < heat.shape[0] else 0.
                if h > 0.015:
                    pi   = int(h * (len(_FIRE_PAL)   - 1))
                    ci   = int(h * (len(_FIRE_CHARS)  - 1))
                    code = _FIRE_PAL  [min(pi, len(_FIRE_PAL)   - 1)]
                    ch   = _FIRE_CHARS[min(ci, len(_FIRE_CHARS) - 1)]
                    parts.append((BLD if h > 0.7 else '') + fg(code) + ch + RST)
                else:
                    parts.append(' ')
            lines.append(''.join(parts))
        lines.append(self._sbar(cols, fps))
        return _pad(lines, rows, cols)


# =============================================================================
#  REGISTRY
# =============================================================================
VIZZES = {
    'spectrum':  SpectrumViz,
    'scope':     ScopeViz,
    'matrix':    MatrixViz,
    'radial':    RadialViz,
    'lissajous': LissajousViz,
    'fire':      FireViz,
}

# =============================================================================
#  CROSS-PLATFORM AUDIO CAPTURE
# =============================================================================
def _start_capture(monitor, backend):
    if backend == "pulse":
        cmd = [
            'parec',
            '--device',       monitor,
            '--rate',         str(SR),
            '--channels',     str(CH),
            '--format',       's16le',
            '--latency-msec', '15',
        ]
    elif backend == "avfoundation":
        cmd = [
            'ffmpeg',
            '-loglevel',  'quiet',
            '-f',         'avfoundation',
            '-i',         monitor,
            '-ar',        str(SR),
            '-ac',        str(CH),
            '-f',         's16le',
            'pipe:1',
        ]
    else:
        raise ValueError(f"Unknown capture backend: {backend!r}")
    return subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)


# =============================================================================
#  MAIN LOOP
# =============================================================================
def main():
    global _RUN

    if VIZ_NAME not in VIZZES:
        print(f"Unknown visualizer '{VIZ_NAME}'. Available: {', '.join(VIZZES)}")
        sys.exit(1)

    def cleanup(*_):
        global _RUN
        _RUN = False
        sys.stdout.write(SHOW + RST + '\n')
        sys.stdout.flush()

    atexit.register(cleanup)
    signal.signal(signal.SIGINT,  lambda *_: (cleanup(), sys.exit(0)))
    signal.signal(signal.SIGTERM, cleanup)

    proc = _start_capture(MONITOR, BACKEND)
    threading.Thread(target=_reader, args=(proc,), daemon=True).start()

    rows, cols = tsize()
    viz = VIZZES[VIZ_NAME](rows, cols)

    sys.stdout.write(HIDE + CLR)
    sys.stdout.flush()

    last_r, last_c = rows, cols
    fps_disp = float(FPS_TGT)
    fps_a    = 0.08
    t_prev   = time.perf_counter()
    frame_dt = 1. / FPS_TGT

    while _RUN:
        t0 = time.perf_counter()

        rows, cols = tsize()
        if rows != last_r or cols != last_c:
            viz.resize(rows, cols)
            sys.stdout.write(CLR)
            last_r, last_c = rows, cols

        L, R, mono = get_audio()
        dt = time.perf_counter() - t_prev
        t_prev = time.perf_counter()
        dt = max(min(dt, 0.15), 1e-4)

        viz.tick(L, R, mono, dt)

        frame_lines = viz.frame(rows, cols, fps_disp)
        out = [HOME]
        for line in frame_lines[:rows]:
            out.append(line + EL + '\n')
        sys.stdout.write(''.join(out))
        sys.stdout.flush()

        elapsed  = time.perf_counter() - t0
        fps_disp = fps_a * (1. / max(elapsed, 1e-6)) + (1. - fps_a) * fps_disp
        sleep    = frame_dt - elapsed
        if sleep > 0:
            time.sleep(sleep)

    proc.terminate()
    proc.wait()

main()
PYTHON_EOF

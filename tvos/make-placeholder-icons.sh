#!/usr/bin/env bash
# Generate solid-color placeholder icons for all required tvOS asset catalog slots.
# Run this once from the tvos/ directory before archiving.
# Replace the output PNGs with real artwork before App Store submission.
#
# Usage: cd tvos && ./make-placeholder-icons.sh

set -euo pipefail

ASSETS="AudioViz/Assets.xcassets/App Icon & Top Shelf Image.brandassets"

python3 - "$ASSETS" <<'PYEOF'
import sys, struct, zlib, os

def png(w, h, r, g, b):
    """Create a minimal solid-color RGB PNG."""
    def chunk(t, d):
        crc = zlib.crc32(t + d) & 0xffffffff
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", crc)
    scanline = b"\x00" + bytes([r, g, b]) * w
    raw = scanline * h
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 6))
        + chunk(b"IEND", b"")
    )

def write(path, w, h, r, g, b):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(png(w, h, r, g, b))
    print(f"  {w}x{h}  {path}")

a = sys.argv[1]  # assets base dir

# ── Home screen icon layers (parallax) ───────────────────────────────────────
# Back: deep space blue-black
back1 = f"{a}/App Icon.imagestack/Back.imagestacklayer/Content.imageset/back@1x.png"
back2 = f"{a}/App Icon.imagestack/Back.imagestacklayer/Content.imageset/back@2x.png"
write(back1,  400, 240,  10,  10,  26)
write(back2,  800, 480,  10,  10,  26)

# Middle: dark indigo
mid1 = f"{a}/App Icon.imagestack/Middle.imagestacklayer/Content.imageset/middle@1x.png"
mid2 = f"{a}/App Icon.imagestack/Middle.imagestacklayer/Content.imageset/middle@2x.png"
write(mid1,  400, 240,  20,  15,  60)
write(mid2,  800, 480,  20,  15,  60)

# Front: accent purple (waveform layer — replace with artwork)
fr1 = f"{a}/App Icon.imagestack/Front.imagestacklayer/Content.imageset/front@1x.png"
fr2 = f"{a}/App Icon.imagestack/Front.imagestacklayer/Content.imageset/front@2x.png"
write(fr1,  400, 240,  58,  47, 110)
write(fr2,  800, 480,  58,  47, 110)

# ── App Store icon (1280×768, single layer) ──────────────────────────────────
appstore = f"{a}/App Icon - App Store.imagestack/Front.imagestacklayer/Content.imageset/appstore.png"
write(appstore, 1280, 768,  10,  10,  26)

# ── Top Shelf ─────────────────────────────────────────────────────────────────
write(f"{a}/Top Shelf Image.imageset/top-shelf.png",      1920, 720,  10,  10,  26)
write(f"{a}/Top Shelf Image Wide.imageset/top-shelf-wide.png", 2320, 720,  10,  10,  26)

print("Done. Replace PNGs with real artwork before App Store submission.")
PYEOF

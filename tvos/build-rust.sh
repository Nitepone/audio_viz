#!/usr/bin/env bash
# build-rust.sh — Cross-compile the Rust core to Apple TV static libraries.
#
# Prerequisites
# ─────────────
#   rustup toolchain install nightly
#   rustup component add rust-src --toolchain nightly
#
# All targets are Tier 3 in Rust and require -Z build-std.
#
# Outputs
#   bridge/libaudio_viz.a      — aarch64-apple-tvos           (device)
#   bridge/libaudio_viz_sim.a  — arm64 + x86_64 fat           (simulator)
#
# Usage
#   ./tvos/build-rust.sh            # release build (default)
#   ./tvos/build-rust.sh --debug    # debug build (faster compile, larger binary)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$REPO_ROOT/tvos/bridge"
DEVICE_TARGET="aarch64-apple-tvos"
SIM_ARM_TARGET="aarch64-apple-tvos-sim"
SIM_X86_TARGET="x86_64-apple-tvos"

# ── Parse arguments ───────────────────────────────────────────────────────────

PROFILE="release"
CARGO_PROFILE_FLAG="--release"
for arg in "$@"; do
    case "$arg" in
        --debug)
            PROFILE="debug"
            CARGO_PROFILE_FLAG=""
            ;;
    esac
done

cd "$REPO_ROOT"

build_target() {
    local target="$1"
    echo "==> Building audio_viz for $target ($PROFILE)"
    cargo +nightly build \
        -Z build-std \
        --target "$target" \
        $CARGO_PROFILE_FLAG \
        --no-default-features \
        --features tvos \
        --lib
}

# ── Device build ──────────────────────────────────────────────────────────────

build_target "$DEVICE_TARGET"

DEVICE_SRC="$REPO_ROOT/target/$DEVICE_TARGET/$PROFILE/libaudio_viz.a"
[ -f "$DEVICE_SRC" ] || { echo "ERROR: not found: $DEVICE_SRC" >&2; exit 1; }
cp "$DEVICE_SRC" "$OUT_DIR/libaudio_viz.a"
echo "    $(du -sh "$OUT_DIR/libaudio_viz.a" | cut -f1)  libaudio_viz.a"

# ── Simulator build (fat: arm64 + x86_64) ─────────────────────────────────────

echo ""
build_target "$SIM_ARM_TARGET"
SIM_ARM_SRC="$REPO_ROOT/target/$SIM_ARM_TARGET/$PROFILE/libaudio_viz.a"
[ -f "$SIM_ARM_SRC" ] || { echo "ERROR: not found: $SIM_ARM_SRC" >&2; exit 1; }

echo ""
build_target "$SIM_X86_TARGET"
SIM_X86_SRC="$REPO_ROOT/target/$SIM_X86_TARGET/$PROFILE/libaudio_viz.a"
[ -f "$SIM_X86_SRC" ] || { echo "ERROR: not found: $SIM_X86_SRC" >&2; exit 1; }

echo ""
echo "==> Combining simulator slices into fat library"
lipo -create "$SIM_ARM_SRC" "$SIM_X86_SRC" -output "$OUT_DIR/libaudio_viz_sim.a"
echo "    $(lipo -info "$OUT_DIR/libaudio_viz_sim.a" 2>&1)"
echo "    $(du -sh "$OUT_DIR/libaudio_viz_sim.a" | cut -f1)  libaudio_viz_sim.a"

# ── Verify symbols ────────────────────────────────────────────────────────────

echo ""
echo "==> Exported aviz_* symbols:"
nm "$OUT_DIR/libaudio_viz.a" 2>/dev/null \
    | grep " T aviz_" \
    | awk '{print "    " $3}' \
    | sort \
    || echo "    (nm not available)"

echo ""
echo "==> Done"
echo "    $OUT_DIR/libaudio_viz.a"
echo "    $OUT_DIR/libaudio_viz_sim.a"

#!/bin/bash
# VeridianOS Rust std Platform Layer -- Build Script
#
# Copyright (c) 2025-2026 VeridianOS Contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Builds the veridian-std crate for VeridianOS user-space targets.
# Requires:
#   - Rust nightly toolchain with rust-src component
#   - VeridianOS user-space target JSON files in targets/
#
# Usage:
#   ./build.sh [x86_64|aarch64|riscv64] [dev|release]
#
# Examples:
#   ./build.sh x86_64 dev       # Build for x86_64 (debug)
#   ./build.sh aarch64 release  # Build for AArch64 (release)
#   ./build.sh                  # Build for x86_64 (debug, default)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TARGETS_DIR="$PROJECT_ROOT/targets"

ARCH="${1:-x86_64}"
MODE="${2:-dev}"

# Map architecture to target JSON
case "$ARCH" in
    x86_64)
        TARGET_JSON="$TARGETS_DIR/x86_64-veridian-user.json"
        ;;
    aarch64)
        TARGET_JSON="$TARGETS_DIR/aarch64-veridian-user.json"
        ;;
    riscv64)
        TARGET_JSON="$TARGETS_DIR/riscv64gc-veridian-user.json"
        ;;
    *)
        echo "Error: Unknown architecture '$ARCH'. Use x86_64, aarch64, or riscv64."
        exit 1
        ;;
esac

if [ ! -f "$TARGET_JSON" ]; then
    echo "Error: Target JSON not found: $TARGET_JSON"
    exit 1
fi

# Build mode flags
BUILD_FLAGS=""
if [ "$MODE" = "release" ]; then
    BUILD_FLAGS="--release"
fi

echo "Building veridian-std for $ARCH ($MODE)..."
echo "  Target: $TARGET_JSON"

# Build with -Zbuild-std=core since we are targeting a custom OS
cd "$SCRIPT_DIR"
cargo build \
    --target "$TARGET_JSON" \
    -Zbuild-std=core \
    $BUILD_FLAGS

echo "Build complete."
echo "  Output: target/$(basename "$TARGET_JSON" .json)/$MODE/libveridian_std.rlib"

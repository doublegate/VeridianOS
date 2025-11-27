#!/bin/bash
# Build script for creating bootable disk images using bootloader 0.11+
# This script builds the bootimage-builder tool in /tmp to avoid workspace config conflicts

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BUILDER_SRC="$SCRIPT_DIR/bootimage-builder"
BUILD_DIR="/tmp/veridian-bootimage-builder"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${YELLOW}VeridianOS Bootimage Builder${NC}"

# Parse arguments
KERNEL_PATH="${1:-}"
OUTPUT_DIR="${2:-}"
MODE="${3:-bios}"

if [ -z "$KERNEL_PATH" ]; then
    echo "Usage: $0 <kernel-elf-path> <output-dir> [bios|uefi|both]"
    echo ""
    echo "Example:"
    echo "  $0 target/x86_64-veridian/debug/veridian-kernel target/x86_64-veridian/debug bios"
    exit 1
fi

if [ -z "$OUTPUT_DIR" ]; then
    OUTPUT_DIR="$(dirname "$KERNEL_PATH")"
fi

# Ensure kernel exists (check both relative and project-relative paths)
if [ -f "$KERNEL_PATH" ]; then
    FULL_KERNEL_PATH="$(realpath "$KERNEL_PATH")"
elif [ -f "$PROJECT_DIR/$KERNEL_PATH" ]; then
    FULL_KERNEL_PATH="$(realpath "$PROJECT_DIR/$KERNEL_PATH")"
else
    echo -e "${RED}Error: Kernel not found at $KERNEL_PATH${NC}"
    exit 1
fi

# Resolve output directory
if [ -d "$OUTPUT_DIR" ]; then
    FULL_OUTPUT_DIR="$(realpath "$OUTPUT_DIR")"
elif [ -d "$PROJECT_DIR/$OUTPUT_DIR" ]; then
    FULL_OUTPUT_DIR="$(realpath "$PROJECT_DIR/$OUTPUT_DIR")"
else
    FULL_OUTPUT_DIR="$PROJECT_DIR/$OUTPUT_DIR"
    mkdir -p "$FULL_OUTPUT_DIR"
fi

echo "Kernel: $FULL_KERNEL_PATH"
echo "Output: $FULL_OUTPUT_DIR"

# Copy builder source to temp directory (to avoid workspace config)
echo "Preparing bootimage builder..."
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"
cp -r "$BUILDER_SRC"/* "$BUILD_DIR/"

# Remove the .cargo config that tried to override workspace settings
rm -rf "$BUILD_DIR/.cargo"

# Build the tool in isolation (requires nightly for bootloader build.rs)
echo "Building bootimage-builder tool..."
cd "$BUILD_DIR"
if ! cargo +nightly build --release 2>&1; then
    echo -e "${RED}Failed to build bootimage-builder${NC}"
    echo "Make sure nightly Rust is installed: rustup install nightly"
    exit 1
fi

# Run the builder
echo "Creating disk image..."
"$BUILD_DIR/target/release/bootimage-builder" \
    --kernel "$FULL_KERNEL_PATH" \
    --output "$FULL_OUTPUT_DIR" \
    --mode "$MODE"

echo -e "${GREEN}Done!${NC}"

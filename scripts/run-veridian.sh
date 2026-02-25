#!/usr/bin/env bash
# run-veridian.sh -- Convenience QEMU launcher for VeridianOS (x86_64)
#
# Usage:
#   ./scripts/run-veridian.sh                    # Boot with TAR rootfs (default)
#   ./scripts/run-veridian.sh --blockfs           # Boot with persistent BlockFS image
#   ./scripts/run-veridian.sh --display           # Enable framebuffer display
#   ./scripts/run-veridian.sh --blockfs --display  # Both
#   ./scripts/run-veridian.sh --release           # Use release build
#   ./scripts/run-veridian.sh --help              # Show help
#
# The script auto-detects OVMF firmware location, kills stale QEMU processes,
# validates required files, and prints the exact command being run.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# -------------------------------------------------------------------------
# Defaults
# -------------------------------------------------------------------------
USE_BLOCKFS=0
USE_DISPLAY=0
BUILD_MODE="debug"

# -------------------------------------------------------------------------
# Parse arguments
# -------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --blockfs)
            USE_BLOCKFS=1
            shift
            ;;
        --display)
            USE_DISPLAY=1
            shift
            ;;
        --release)
            BUILD_MODE="release"
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --blockfs    Boot with persistent BlockFS image (2048M RAM)"
            echo "  --display    Enable framebuffer display (remove -display none)"
            echo "  --release    Use release build instead of debug"
            echo "  --help       Show this help message"
            echo ""
            echo "Examples:"
            echo "  $0                        # TAR rootfs, serial only"
            echo "  $0 --blockfs              # Persistent storage, serial only"
            echo "  $0 --blockfs --display    # Persistent storage + framebuffer"
            echo "  $0 --release --display    # Release build with framebuffer"
            exit 0
            ;;
        *)
            echo "Error: unknown option '$1'"
            echo "Run '$0 --help' for usage."
            exit 1
            ;;
    esac
done

# -------------------------------------------------------------------------
# Auto-detect OVMF firmware
# -------------------------------------------------------------------------
OVMF=""
for candidate in \
    /usr/share/edk2/x64/OVMF.4m.fd \
    /usr/share/OVMF/OVMF_CODE.fd \
    /usr/share/edk2/ovmf/OVMF_CODE.fd; do
    if [[ -f "$candidate" ]]; then
        OVMF="$candidate"
        break
    fi
done

if [[ -z "$OVMF" ]]; then
    echo "Error: OVMF firmware not found."
    echo "Searched:"
    echo "  /usr/share/edk2/x64/OVMF.4m.fd       (CachyOS/Arch)"
    echo "  /usr/share/OVMF/OVMF_CODE.fd          (Ubuntu/Debian)"
    echo "  /usr/share/edk2/ovmf/OVMF_CODE.fd     (Fedora)"
    echo ""
    echo "Install OVMF/EDK2 for your distribution:"
    echo "  Arch:   pacman -S edk2-ovmf"
    echo "  Debian: apt install ovmf"
    echo "  Fedora: dnf install edk2-ovmf"
    exit 1
fi

# -------------------------------------------------------------------------
# Determine build paths
# -------------------------------------------------------------------------
UEFI_IMG="${PROJECT_ROOT}/target/x86_64-veridian/${BUILD_MODE}/veridian-uefi.img"
TAR_ROOTFS="${PROJECT_ROOT}/target/rootfs-busybox.tar"
BLOCKFS_IMG="${PROJECT_ROOT}/target/rootfs-blockfs.img"

# -------------------------------------------------------------------------
# Validate required files
# -------------------------------------------------------------------------
if [[ ! -f "$UEFI_IMG" ]]; then
    echo "Error: UEFI disk image not found: $UEFI_IMG"
    echo ""
    echo "Build it first:"
    echo "  ./build-kernel.sh x86_64 ${BUILD_MODE/#debug/dev}"
    exit 1
fi

if [[ "$USE_BLOCKFS" -eq 1 ]]; then
    if [[ ! -f "$BLOCKFS_IMG" ]]; then
        echo "Error: BlockFS image not found: $BLOCKFS_IMG"
        echo ""
        echo "Create it first:"
        echo "  ./scripts/build-busybox-rootfs.sh all      # Build cross-compiled rootfs"
        echo "  ./scripts/build-busybox-rootfs.sh blockfs   # Create BlockFS image"
        exit 1
    fi
    ROOTFS_FILE="$BLOCKFS_IMG"
    RAM="2048M"
else
    if [[ ! -f "$TAR_ROOTFS" ]]; then
        echo "Warning: TAR rootfs not found: $TAR_ROOTFS"
        echo "Booting without rootfs (kernel-only mode)."
        echo ""
        ROOTFS_FILE=""
    else
        ROOTFS_FILE="$TAR_ROOTFS"
    fi
    RAM="256M"
fi

# -------------------------------------------------------------------------
# Kill stale QEMU processes
# -------------------------------------------------------------------------
echo "Checking for stale QEMU processes..."
pkill -9 -f qemu-system 2>/dev/null || true
sleep 2

# -------------------------------------------------------------------------
# Build QEMU command
# -------------------------------------------------------------------------
QEMU_CMD=(
    qemu-system-x86_64 -enable-kvm
    -drive "if=pflash,format=raw,readonly=on,file=${OVMF}"
    -drive "id=disk0,if=none,format=raw,file=${UEFI_IMG}"
    -device ide-hd,drive=disk0
)

if [[ -n "${ROOTFS_FILE:-}" ]]; then
    QEMU_CMD+=(
        -drive "file=${ROOTFS_FILE},if=none,id=vd0,format=raw"
        -device virtio-blk-pci,drive=vd0
    )
fi

QEMU_CMD+=(-serial stdio -m "$RAM")

if [[ "$USE_DISPLAY" -eq 0 ]]; then
    QEMU_CMD+=(-display none)
fi

# -------------------------------------------------------------------------
# Print and execute
# -------------------------------------------------------------------------
echo ""
echo "=== VeridianOS QEMU Launcher ==="
echo "  Build:    ${BUILD_MODE}"
echo "  Rootfs:   $(if [[ "$USE_BLOCKFS" -eq 1 ]]; then echo "BlockFS (persistent)"; elif [[ -n "${ROOTFS_FILE:-}" ]]; then echo "TAR (read-only)"; else echo "none"; fi)"
echo "  Display:  $(if [[ "$USE_DISPLAY" -eq 1 ]]; then echo "framebuffer"; else echo "serial only"; fi)"
echo "  RAM:      ${RAM}"
echo "  OVMF:     ${OVMF}"
echo ""
echo "Command:"
echo "  ${QEMU_CMD[*]}"
echo ""

exec "${QEMU_CMD[@]}"

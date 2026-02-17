#!/bin/bash
# build-rootfs.sh — Cross-compile test programs and package into rootfs.tar
#
# Usage: ./scripts/build-rootfs.sh [toolchain-prefix]
#
# Creates a TAR archive containing /bin/<test programs> for loading into
# VeridianOS via the virtio-blk TAR loader at boot time.
#
# QEMU usage:
#   -drive file=rootfs.tar,if=none,id=vd0,format=raw \
#   -device virtio-blk-pci,drive=vd0

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TOOLCHAIN_PREFIX="${1:-$HOME/veridian-toolchain}"

CC="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-gcc"
STRIP="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-strip"
SYSROOT="${TOOLCHAIN_PREFIX}/sysroot"

TESTS_DIR="${PROJECT_ROOT}/userland/tests"
BUILD_DIR="${PROJECT_ROOT}/target/rootfs-build"
ROOTFS_TAR="${PROJECT_ROOT}/target/rootfs.tar"

# Verify cross-compiler exists
if [ ! -x "$CC" ]; then
    echo "ERROR: Cross-compiler not found at $CC"
    echo "Run scripts/build-cross-toolchain.sh first."
    exit 1
fi

echo "=== VeridianOS rootfs builder ==="
echo "Compiler:  $CC"
echo "Sysroot:   $SYSROOT"
echo "Tests dir: $TESTS_DIR"
echo ""

# Clean and create build directory
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/bin"

# Common compiler flags
CFLAGS="-static -O2 --sysroot=${SYSROOT}"

# Compile each test program
PROGRAMS=()
for src in "$TESTS_DIR"/*.c; do
    name="$(basename "$src" .c)"
    out="$BUILD_DIR/bin/$name"

    # minimal.c provides its own _start -- use -nostdlib -nostdinc
    if [ "$name" = "minimal" ]; then
        EXTRA_FLAGS="-nostdlib -nostdinc -ffreestanding"
    else
        EXTRA_FLAGS=""
    fi

    echo -n "  Compiling $name... "
    if "$CC" $CFLAGS $EXTRA_FLAGS -o "$out" "$src" 2>&1; then
        # Strip debug info to reduce size
        "$STRIP" "$out" 2>/dev/null || true
        size=$(stat -c%s "$out" 2>/dev/null || stat -f%z "$out" 2>/dev/null)
        echo "OK ($(( size / 1024 )) KB)"
        PROGRAMS+=("$name")
    else
        echo "FAILED"
    fi
done

echo ""

if [ ${#PROGRAMS[@]} -eq 0 ]; then
    echo "ERROR: No programs compiled successfully."
    exit 1
fi

# Create the TAR archive
# The TAR loader in the kernel expects paths like /bin/hello, so we
# create the archive from the build directory root with relative paths.
echo "Creating rootfs.tar with ${#PROGRAMS[@]} programs..."
cd "$BUILD_DIR"
tar cf "$ROOTFS_TAR" bin/
cd "$PROJECT_ROOT"

# Show contents
echo ""
echo "=== rootfs.tar contents ==="
tar tvf "$ROOTFS_TAR"
echo ""

size=$(stat -c%s "$ROOTFS_TAR" 2>/dev/null || stat -f%z "$ROOTFS_TAR" 2>/dev/null)
echo "Total: $size bytes ($(( size / 1024 )) KB)"
echo ""
echo "To boot with this disk image:"
echo "  qemu-system-x86_64 -enable-kvm \\"
echo "    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \\"
echo "    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \\"
echo "    -device ide-hd,drive=disk0 \\"
echo "    -drive file=target/rootfs.tar,if=none,id=vd0,format=raw \\"
echo "    -device virtio-blk-pci,drive=vd0 \\"
echo "    -serial stdio -display none -m 256M"

#!/bin/bash
# build-rootfs.sh — Cross-compile user-space programs and package into rootfs.tar
#
# Usage: ./scripts/build-rootfs.sh [toolchain-prefix]
#
# Creates a TAR archive containing /bin/<programs> for loading into
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
PROGRAMS_DIR="${PROJECT_ROOT}/userland/programs"
LIBC_DIR="${PROJECT_ROOT}/userland/libc"
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
echo ""

# Clean and create build directory
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/bin"

# =========================================================================
# Common compiler flags
# =========================================================================

LIBC_INCDIR="${LIBC_DIR}/include"
SYS_INCDIR="${SYSROOT}/usr/include"
LIBC_LIBDIR="${SYSROOT}/usr/lib"
CRT0="${SYSROOT}/usr/lib/crt0.o"

# Flags for libc-linked programs
CFLAGS_LIBC="-std=c11 -static -O2"
CFLAGS_LIBC+=" -nostdinc -isystem ${LIBC_INCDIR} -isystem ${SYS_INCDIR}"
CFLAGS_LIBC+=" -fno-stack-protector -ffreestanding"
CFLAGS_LIBC+=" -mno-red-zone -mcmodel=small"
CFLAGS_LIBC+=" -Wall -Wextra -Wno-unused-parameter"

LDFLAGS_LIBC="-static -nostdlib -L${LIBC_LIBDIR}"

# Flags for no-libc programs
CFLAGS_MINIMAL="-nostdlib -nostdinc -ffreestanding -static -O2"
CFLAGS_MINIMAL+=" -mno-red-zone -mcmodel=small -Wall -Wextra"

BUILT_COUNT=0

# =========================================================================
# Helper: compile a libc-linked program
# =========================================================================
compile_libc_program() {
    local name="$1"
    local src="$2"
    local extra_libs="${3:-}"   # Optional extra libraries (e.g. "-lcurses")
    local out="$BUILD_DIR/bin/$name"

    echo -n "  Compiling $name... "
    if "$CC" $CFLAGS_LIBC $LDFLAGS_LIBC -o "$out" "$CRT0" "$src" $extra_libs -lc 2>&1; then
        "$STRIP" "$out" 2>/dev/null || true
        local size
        size=$(stat -c%s "$out" 2>/dev/null || stat -f%z "$out" 2>/dev/null)
        echo "OK ($(( size / 1024 )) KB)"
        BUILT_COUNT=$((BUILT_COUNT + 1))
    else
        echo "FAILED"
    fi
}

# =========================================================================
# 1. Compile user-space programs from programs/
# =========================================================================
echo "--- User-space programs ---"

# Shell (/bin/sh)
if [ -f "${PROGRAMS_DIR}/sh/sh.c" ]; then
    compile_libc_program "sh" "${PROGRAMS_DIR}/sh/sh.c"
fi

# sysinfo (system information display, inspired by fastfetch)
if [ -f "${PROGRAMS_DIR}/sysinfo/sysinfo.c" ]; then
    compile_libc_program "sysinfo" "${PROGRAMS_DIR}/sysinfo/sysinfo.c"
fi

# edit (nano-inspired text editor)
if [ -f "${PROGRAMS_DIR}/edit/edit.c" ]; then
    compile_libc_program "edit" "${PROGRAMS_DIR}/edit/edit.c" "-lcurses"
fi

# =========================================================================
# 2. Compile test programs from tests/
# =========================================================================
echo "--- Test programs ---"

for src in "$TESTS_DIR"/*.c; do
    [ -f "$src" ] || continue
    name="$(basename "$src" .c)"
    out="$BUILD_DIR/bin/$name"

    if [ "$name" = "minimal" ] || [ "$name" = "fork_test" ] || [ "$name" = "exec_test" ]; then
        # These provide their own _start -- no libc
        echo -n "  Compiling $name... "
        if "$CC" $CFLAGS_MINIMAL -o "$out" "$src" 2>&1; then
            "$STRIP" "$out" 2>/dev/null || true
            local_size=$(stat -c%s "$out" 2>/dev/null || stat -f%z "$out" 2>/dev/null)
            echo "OK ($(( local_size / 1024 )) KB)"
            BUILT_COUNT=$((BUILT_COUNT + 1))
        else
            echo "FAILED"
        fi
    else
        compile_libc_program "$name" "$src"
    fi
done

echo ""

if [ "$BUILT_COUNT" -eq 0 ]; then
    echo "ERROR: No programs compiled successfully."
    exit 1
fi

# =========================================================================
# 3. Validate all binaries are statically linked (no PT_INTERP)
# =========================================================================
echo "--- Static linking validation ---"
READELF="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-readelf"
if [ ! -x "$READELF" ]; then
    # Fall back to host readelf
    READELF="readelf"
fi

VALIDATION_FAILED=0
for bin in "$BUILD_DIR"/bin/*; do
    [ -f "$bin" ] || continue
    name="$(basename "$bin")"
    if "$READELF" -l "$bin" 2>/dev/null | grep -q 'INTERP'; then
        echo "  ERROR: $name is dynamically linked (has PT_INTERP segment)"
        VALIDATION_FAILED=1
    else
        echo "  $name: statically linked OK"
    fi
done

if [ "$VALIDATION_FAILED" -ne 0 ]; then
    echo ""
    echo "ERROR: Some binaries are dynamically linked."
    echo "Ensure -static is in both CFLAGS and LDFLAGS."
    exit 1
fi
echo ""

# =========================================================================
# 4. Create the TAR archive
# =========================================================================

echo "Creating rootfs.tar with $BUILT_COUNT programs..."
cd "$BUILD_DIR"
tar cf "$ROOTFS_TAR" bin/
cd "$PROJECT_ROOT"

# Also copy to project root for convenience
cp "$ROOTFS_TAR" "${PROJECT_ROOT}/rootfs.tar"

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

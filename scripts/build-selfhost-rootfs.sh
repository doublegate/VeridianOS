#!/bin/bash
# build-selfhost-rootfs.sh -- Build rootfs with native GCC toolchain for self-hosting test
#
# Creates a TAR archive containing:
#   - /bin/ programs (from regular rootfs)
#   - /usr/bin/gcc, /usr/bin/as, /usr/bin/ld (native toolchain)
#   - /usr/libexec/gcc/x86_64-veridian/14.2.0/cc1, collect2
#   - /usr/lib/ (libc.a, libgcc.a, crt*.o)
#   - /usr/include/ (C headers)
#   - /usr/src/selfhost_test.c (test source file)
#
# Usage: ./scripts/build-selfhost-rootfs.sh [toolchain-prefix]
#
# The resulting rootfs-selfhost.tar is ~45MB and requires the kernel
# to have a 128MB heap (set in mm/heap.rs) and QEMU with at least 512MB RAM.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TOOLCHAIN_PREFIX="${1:-/opt/veridian/toolchain}"

CC="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-gcc"
STRIP="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-strip"
SYSROOT="${TOOLCHAIN_PREFIX}/sysroot"

NATIVE_GCC_DIR="${PROJECT_ROOT}/target/native-gcc-static"
TESTS_DIR="${PROJECT_ROOT}/userland/tests"
PROGRAMS_DIR="${PROJECT_ROOT}/userland/programs"
LIBC_DIR="${PROJECT_ROOT}/userland/libc"
BUILD_DIR="${PROJECT_ROOT}/target/rootfs-selfhost-build"
ROOTFS_TAR="${PROJECT_ROOT}/target/rootfs-selfhost.tar"

# Verify prerequisites
if [ ! -x "$CC" ]; then
    echo "ERROR: Cross-compiler not found at $CC"
    exit 1
fi

if [ ! -d "$NATIVE_GCC_DIR/usr/bin" ]; then
    echo "ERROR: Native GCC toolchain not found at $NATIVE_GCC_DIR"
    echo "Run scripts/build-native-gcc-static.sh first."
    exit 1
fi

if [ ! -f "$NATIVE_GCC_DIR/usr/libexec/gcc/x86_64-veridian/14.2.0/cc1" ]; then
    echo "ERROR: cc1 not found in native toolchain"
    exit 1
fi

echo "=== VeridianOS Self-Hosting Rootfs Builder ==="
echo "Compiler:     $CC"
echo "Sysroot:      $SYSROOT"
echo "Native GCC:   $NATIVE_GCC_DIR"
echo ""

# Clean and create build directory
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/bin"
mkdir -p "$BUILD_DIR/usr/bin"
mkdir -p "$BUILD_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/include"
mkdir -p "$BUILD_DIR/usr/libexec/gcc/x86_64-veridian/14.2.0"
mkdir -p "$BUILD_DIR/usr/include/sys"
mkdir -p "$BUILD_DIR/usr/include/arpa"
mkdir -p "$BUILD_DIR/usr/include/netinet"
mkdir -p "$BUILD_DIR/usr/src"
mkdir -p "$BUILD_DIR/tmp"
mkdir -p "$BUILD_DIR/var/tmp"

# =========================================================================
# Common compiler flags (same as build-rootfs.sh)
# =========================================================================
LIBC_INCDIR="${LIBC_DIR}/include"
SYS_INCDIR="${SYSROOT}/usr/include"
LIBC_LIBDIR="${SYSROOT}/usr/lib"
CRT0="${SYSROOT}/usr/lib/crt0.o"

CFLAGS_LIBC="-std=c11 -static -O2"
CFLAGS_LIBC+=" -nostdinc -isystem ${LIBC_INCDIR} -isystem ${SYS_INCDIR}"
CFLAGS_LIBC+=" -fno-stack-protector -ffreestanding"
CFLAGS_LIBC+=" -mno-red-zone -mcmodel=small"
CFLAGS_LIBC+=" -Wall -Wextra -Wno-unused-parameter"

LDFLAGS_LIBC="-static -nostdlib -L${LIBC_LIBDIR}"

CFLAGS_MINIMAL="-nostdlib -nostdinc -ffreestanding -static -O2"
CFLAGS_MINIMAL+=" -mno-red-zone -mcmodel=small -Wall -Wextra"

BUILT_COUNT=0

compile_libc_program() {
    local name="$1"
    local src="$2"
    local out="$BUILD_DIR/bin/$name"

    echo -n "  Compiling $name... "
    if "$CC" $CFLAGS_LIBC $LDFLAGS_LIBC -o "$out" "$CRT0" "$src" -lc 2>&1; then
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
# 1. Compile user-space programs
# =========================================================================
echo "--- User-space programs ---"

if [ -f "${PROGRAMS_DIR}/sh/sh.c" ]; then
    compile_libc_program "sh" "${PROGRAMS_DIR}/sh/sh.c"
fi

if [ -f "${PROGRAMS_DIR}/sysinfo/sysinfo.c" ]; then
    compile_libc_program "sysinfo" "${PROGRAMS_DIR}/sysinfo/sysinfo.c"
fi

# =========================================================================
# 2. Compile test programs
# =========================================================================
echo "--- Test programs ---"

for src in "$TESTS_DIR"/*.c; do
    [ -f "$src" ] || continue
    name="$(basename "$src" .c)"
    out="$BUILD_DIR/bin/$name"

    # Skip selfhost_test -- it's a source file for on-OS compilation
    [ "$name" = "selfhost_test" ] && continue

    if [ "$name" = "minimal" ] || [ "$name" = "fork_test" ] || [ "$name" = "exec_test" ]; then
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

# =========================================================================
# 3. Copy native GCC toolchain (minimal set for compilation)
# =========================================================================
echo "--- Native GCC toolchain ---"

# Essential binaries
for tool in gcc as ld; do
    if [ -f "$NATIVE_GCC_DIR/usr/bin/$tool" ]; then
        cp "$NATIVE_GCC_DIR/usr/bin/$tool" "$BUILD_DIR/usr/bin/"
        size=$(stat -c%s "$BUILD_DIR/usr/bin/$tool" 2>/dev/null || echo "?")
        echo "  + /usr/bin/$tool ($(( size / 1024 )) KB)"
    else
        echo "  - /usr/bin/$tool (NOT FOUND)"
    fi
done

# Create cc -> gcc symlink
ln -sf gcc "$BUILD_DIR/usr/bin/cc"
echo "  + /usr/bin/cc -> gcc (symlink)"

# GCC internal tools (cc1 is the actual compiler, collect2 wraps the linker)
for tool in cc1 collect2; do
    src="$NATIVE_GCC_DIR/usr/libexec/gcc/x86_64-veridian/14.2.0/$tool"
    if [ -f "$src" ]; then
        cp "$src" "$BUILD_DIR/usr/libexec/gcc/x86_64-veridian/14.2.0/"
        size=$(stat -c%s "$BUILD_DIR/usr/libexec/gcc/x86_64-veridian/14.2.0/$tool" 2>/dev/null || echo "?")
        echo "  + /usr/libexec/.../14.2.0/$tool ($(( size / 1024 )) KB)"
    else
        echo "  - $tool (NOT FOUND)"
    fi
done

# Libraries
echo "--- Libraries ---"
for f in crt0.o crti.o crtn.o libc.a; do
    if [ -f "$NATIVE_GCC_DIR/usr/lib/$f" ]; then
        cp "$NATIVE_GCC_DIR/usr/lib/$f" "$BUILD_DIR/usr/lib/"
        echo "  + /usr/lib/$f"
    fi
done

for f in libgcc.a crtbegin.o crtend.o; do
    if [ -f "$NATIVE_GCC_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/$f" ]; then
        cp "$NATIVE_GCC_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/$f" \
           "$BUILD_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/"
        echo "  + /usr/lib/gcc/.../14.2.0/$f"
    fi
done

# Headers -- system headers
echo "--- Headers ---"
if [ -d "$NATIVE_GCC_DIR/usr/include" ]; then
    cp -r "$NATIVE_GCC_DIR/usr/include/"* "$BUILD_DIR/usr/include/" 2>/dev/null || true
    hdr_count=$(find "$BUILD_DIR/usr/include" -name '*.h' | wc -l)
    echo "  + $hdr_count system headers"
fi

# GCC internal headers (stdarg.h, stddef.h, stdbool.h, etc.)
gcc_inc="$NATIVE_GCC_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/include"
if [ -d "$gcc_inc" ]; then
    # Copy only essential GCC headers, not the huge x86 intrinsics
    for h in stdarg.h stddef.h stdbool.h varargs.h float.h limits.h \
             stdint.h stdalign.h stdatomic.h stdnoreturn.h stdfix.h \
             iso646.h unwind.h; do
        if [ -f "$gcc_inc/$h" ]; then
            cp "$gcc_inc/$h" "$BUILD_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/include/"
        fi
    done
    gcc_hdr_count=$(find "$BUILD_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/include" -name '*.h' | wc -l)
    echo "  + $gcc_hdr_count GCC internal headers"
fi

# =========================================================================
# 4. Copy test source file for on-OS compilation
# =========================================================================
echo "--- Test source ---"
cp "$TESTS_DIR/selfhost_test.c" "$BUILD_DIR/usr/src/selfhost_test.c"
echo "  + /usr/src/selfhost_test.c"

# =========================================================================
# 5. Validate all binaries in /bin and /usr/bin are statically linked
# =========================================================================
echo ""
echo "--- Static linking validation ---"
READELF="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-readelf"
if [ ! -x "$READELF" ]; then
    READELF="readelf"
fi

for bin in "$BUILD_DIR"/bin/* "$BUILD_DIR"/usr/bin/* "$BUILD_DIR"/usr/libexec/gcc/x86_64-veridian/14.2.0/*; do
    [ -f "$bin" ] || continue
    # Skip non-ELF files (symlinks, etc.)
    file_type="$(file "$bin" 2>/dev/null)" || continue
    echo "$file_type" | grep -q "ELF" || continue

    name="${bin#$BUILD_DIR/}"
    if "$READELF" -l "$bin" 2>/dev/null | grep -q 'INTERP'; then
        echo "  ERROR: $name is dynamically linked!"
    else
        echo "  $name: static OK"
    fi
done

# =========================================================================
# 6. Create the TAR archive
# =========================================================================
echo ""
echo "--- Creating rootfs-selfhost.tar ---"
cd "$BUILD_DIR"
tar cf "$ROOTFS_TAR" bin/ usr/ tmp/ var/
cd "$PROJECT_ROOT"

# Show summary
echo ""
echo "=== rootfs-selfhost.tar summary ==="
tar tvf "$ROOTFS_TAR" | head -40
echo "..."
echo ""

total_files=$(tar tvf "$ROOTFS_TAR" | wc -l)
size=$(stat -c%s "$ROOTFS_TAR" 2>/dev/null || stat -f%z "$ROOTFS_TAR" 2>/dev/null)
echo "Total: $total_files entries, $size bytes ($(( size / 1024 / 1024 )) MB)"
echo ""
echo "To boot with this rootfs:"
echo "  qemu-system-x86_64 -enable-kvm \\"
echo "    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \\"
echo "    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \\"
echo "    -device ide-hd,drive=disk0 \\"
echo "    -drive file=target/rootfs-selfhost.tar,if=none,id=vd0,format=raw \\"
echo "    -device virtio-blk-pci,drive=vd0 \\"
echo "    -serial stdio -display none -m 512M"
echo ""
echo "Self-hosting test commands (run inside VeridianOS, step-by-step):"
echo "  # Step 1: Compile C to assembly"
echo "  /usr/libexec/gcc/x86_64-veridian/14.2.0/cc1 -isystem /usr/include \\"
echo "    -isystem /usr/lib/gcc/x86_64-veridian/14.2.0/include \\"
echo "    /usr/src/selfhost_test.c -o /tmp/test.s -quiet"
echo "  # Step 2: Assemble"
echo "  /usr/bin/as -o /tmp/test.o /tmp/test.s"
echo "  # Step 3: Link with CRT startup objects"
echo "  /usr/bin/ld -static -o /tmp/selfhost_test \\"
echo "    /usr/lib/crt0.o /usr/lib/crti.o \\"
echo "    /usr/lib/gcc/x86_64-veridian/14.2.0/crtbegin.o \\"
echo "    /tmp/test.o -L/usr/lib -L/usr/lib/gcc/x86_64-veridian/14.2.0 \\"
echo "    -lc -lgcc \\"
echo "    /usr/lib/gcc/x86_64-veridian/14.2.0/crtend.o /usr/lib/crtn.o"
echo "  # Step 4: Run"
echo "  /tmp/selfhost_test"
echo "  # Expected output: SELF_HOSTED_PASS"

#!/bin/bash
# build-libcurses.sh -- Build and install libcurses for VeridianOS
#
# Usage: ./scripts/build-libcurses.sh [toolchain-prefix]
#
# Compiles the minimal curses shim into a static library and installs
# it into the cross-compiler sysroot for use by other ports (e.g., nano).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TOOLCHAIN_PREFIX="${1:-/opt/veridian/toolchain}"

CC="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-gcc"
AR="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-ar"
RANLIB="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-ranlib"
SYSROOT="${TOOLCHAIN_PREFIX}/sysroot"

SRC="${PROJECT_ROOT}/userland/libcurses"
BUILD="/tmp/VeridianOS/libcurses-build"

# Verify cross-compiler exists
if [ ! -x "$CC" ]; then
    echo "ERROR: Cross-compiler not found at $CC"
    exit 1
fi

echo "=== Building libcurses for VeridianOS ==="
echo "Source:   $SRC"
echo "Compiler: $CC"
echo "Sysroot:  $SYSROOT"
echo ""

# Clean and create build directory
rm -rf "$BUILD"
mkdir -p "$BUILD"

# Compile
CFLAGS="-static -O2 -Wall -Wextra -Wno-unused-parameter"
CFLAGS+=" -fno-stack-protector -ffreestanding -mno-red-zone -mcmodel=small"
CFLAGS+=" -nostdinc -isystem ${SYSROOT}/usr/include"
CFLAGS+=" -I${SRC}"

echo -n "Compiling curses.c... "
$CC -c $CFLAGS -o "$BUILD/curses.o" "$SRC/curses.c"
echo "OK"

# Archive
echo -n "Creating libcurses.a... "
$AR rcs "$BUILD/libcurses.a" "$BUILD/curses.o"
$RANLIB "$BUILD/libcurses.a"
SIZE=$(stat -c%s "$BUILD/libcurses.a" 2>/dev/null || stat -f%z "$BUILD/libcurses.a" 2>/dev/null)
echo "OK ($(( SIZE / 1024 )) KB)"

# Install to sysroot
echo -n "Installing to sysroot... "
cp "$BUILD/libcurses.a" "$SYSROOT/usr/lib/libcurses.a"
ln -sf libcurses.a "$SYSROOT/usr/lib/libncurses.a"
ln -sf libcurses.a "$SYSROOT/usr/lib/libncursesw.a"
cp "$SRC/curses.h"   "$SYSROOT/usr/include/curses.h"
cp "$SRC/ncurses.h"  "$SYSROOT/usr/include/ncurses.h"
echo "OK"

echo ""
echo "=== libcurses installed ==="
echo "  Library: $SYSROOT/usr/lib/libcurses.a"
echo "  Headers: $SYSROOT/usr/include/curses.h"
echo "           $SYSROOT/usr/include/ncurses.h"
echo "  Symlinks: libncurses.a -> libcurses.a"
echo "            libncursesw.a -> libcurses.a"
echo ""
echo "Link with: -lcurses (or -lncurses)"

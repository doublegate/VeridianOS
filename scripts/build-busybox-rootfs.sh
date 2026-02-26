#!/bin/bash
# build-busybox-rootfs.sh -- Cross-compile BusyBox 1.36.1 and build VeridianOS rootfs
#
# Three-phase approach:
#   Phase A: Cross-compile BusyBox on Linux host for VeridianOS x86_64
#   Phase B: Test in QEMU (manual)
#   Phase C: Native compilation on VeridianOS (separate script)
#
# Prerequisites:
#   - Cross-compiler: /opt/veridian/toolchain/bin/x86_64-veridian-gcc (GCC 14.2.0)
#   - BusyBox source: downloaded automatically to /tmp/VeridianOS/busybox-1.36.1/
#   - Native GCC toolchain: target/native-gcc-static/ (from build-native-gcc-static.sh)
#
# Usage: ./scripts/build-busybox-rootfs.sh [phase]
#   phase: all (default), download, headers, config, patch, build, rootfs

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BUSYBOX_VERSION="1.36.1"
BUSYBOX_URL="https://busybox.net/downloads/busybox-${BUSYBOX_VERSION}.tar.bz2"

# Directories
WORK_DIR="/tmp/VeridianOS"
BB_SRC="${WORK_DIR}/busybox-${BUSYBOX_VERSION}"
BB_BUILD="${WORK_DIR}/busybox-build"
BB_PATCHES="${PROJECT_ROOT}/tools/busybox/patches"
BB_DEFCONFIG="${PROJECT_ROOT}/tools/busybox/veridian_defconfig"

# Toolchain
TOOLCHAIN_PREFIX="/opt/veridian/toolchain"
CC="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-gcc"
STRIP="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-strip"
READELF="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-readelf"
SYSROOT="${TOOLCHAIN_PREFIX}/sysroot"
GCC_LIBDIR="${TOOLCHAIN_PREFIX}/lib/gcc/x86_64-veridian/14.2.0"
GCC_INCDIR="${GCC_LIBDIR}/include"
CC_WRAPPER="${SCRIPT_DIR}/veridian-cc-wrapper.sh"

# Rootfs
NATIVE_GCC_DIR="${PROJECT_ROOT}/target/native-gcc-static"
BUILD_DIR="${PROJECT_ROOT}/target/rootfs-busybox-build"
ROOTFS_TAR="${PROJECT_ROOT}/target/rootfs-busybox.tar"
LIBC_DIR="${PROJECT_ROOT}/userland/libc"
COREUTILS_DIR="${PROJECT_ROOT}/userland/coreutils"
PROGRAMS_DIR="${PROJECT_ROOT}/userland/programs"
TESTS_DIR="${PROJECT_ROOT}/userland/tests"

# =========================================================================
# Phase A-0: Download and extract BusyBox source
# =========================================================================
phase_download() {
    echo "=== Phase A-0: Download BusyBox ${BUSYBOX_VERSION} ==="
    mkdir -p "$WORK_DIR"

    if [ -d "$BB_SRC" ]; then
        echo "  BusyBox source already exists at $BB_SRC"
    else
        local tarball="${WORK_DIR}/busybox-${BUSYBOX_VERSION}.tar.bz2"
        if [ ! -f "$tarball" ]; then
            echo "  Downloading ${BUSYBOX_URL}..."
            curl -L -o "$tarball" "$BUSYBOX_URL"
        fi
        echo "  Extracting..."
        cd "$WORK_DIR"
        tar xjf "$tarball"
    fi

    echo "  BusyBox source: $BB_SRC"
    echo "  Files: $(find "$BB_SRC" -name '*.c' | wc -l) .c files"
    echo ""
}

# =========================================================================
# Phase A-1: Install compatibility headers into sysroot
# =========================================================================
phase_headers() {
    echo "=== Phase A-1: Install compatibility headers ==="

    local src_inc="${LIBC_DIR}/include"
    local dst_inc="${SYSROOT}/usr/include"
    local dst_lib="${SYSROOT}/usr/lib"

    # Copy all repo headers to sysroot
    echo "  Syncing headers to sysroot..."
    for f in "$src_inc"/*.h; do
        [ -f "$f" ] || continue
        cp "$f" "$dst_inc/"
    done
    # Subdirectories
    for subdir in sys arpa netinet veridian; do
        if [ -d "$src_inc/$subdir" ]; then
            mkdir -p "$dst_inc/$subdir"
            for f in "$src_inc/$subdir"/*.h; do
                [ -f "$f" ] || continue
                cp "$f" "$dst_inc/$subdir/"
            done
        fi
    done

    # Rebuild libc.a with posix_stubs3.c
    echo "  Rebuilding libc.a..."
    local libc_src="${LIBC_DIR}/src"
    local libc_build="${WORK_DIR}/libc-build"
    rm -rf "$libc_build"
    mkdir -p "$libc_build"

    local cflags="-std=c11 -static -O2 -nostdinc"
    cflags+=" -isystem ${src_inc}"
    cflags+=" -isystem ${dst_inc}"
    cflags+=" -isystem ${GCC_INCDIR}"
    cflags+=" -fno-stack-protector -ffreestanding"
    cflags+=" -mno-red-zone -mcmodel=small"
    cflags+=" -Wno-unused-parameter -Wno-implicit-function-declaration"

    local obj_count=0
    for src in "$libc_src"/*.c; do
        [ -f "$src" ] || continue
        local name
        name="$(basename "$src" .c)"
        echo -n "    $name.c -> "
        if "$CC" $cflags -c "$src" -o "$libc_build/${name}.o" 2>&1; then
            echo "OK"
            obj_count=$((obj_count + 1))
        else
            echo "FAILED"
        fi
    done

    # Create libc.a
    "$TOOLCHAIN_PREFIX/bin/x86_64-veridian-ar" rcs "$libc_build/libc.a" "$libc_build"/*.o
    cp "$libc_build/libc.a" "$dst_lib/libc.a"
    echo "  libc.a: $obj_count objects"

    local hdr_count
    hdr_count=$(find "$dst_inc" -name '*.h' | wc -l)
    echo "  Sysroot headers: $hdr_count"
    echo ""
}

# =========================================================================
# Phase A-2: Generate BusyBox .config
# =========================================================================
phase_config() {
    echo "=== Phase A-2: Generate BusyBox .config ==="
    mkdir -p "$BB_BUILD"

    if [ ! -f "$BB_DEFCONFIG" ]; then
        echo "  ERROR: veridian_defconfig not found at $BB_DEFCONFIG"
        exit 1
    fi

    # Copy defconfig to BUILD dir (not source tree, to keep source clean)
    cp "$BB_DEFCONFIG" "$BB_BUILD/.config"
    # Run oldconfig to resolve dependencies (non-interactive: accept defaults)
    cd "$BB_SRC"
    yes '' 2>/dev/null | make oldconfig O="$BB_BUILD" >/dev/null 2>&1 || true
    cp "$BB_BUILD/.config" "$BB_BUILD/.config.resolved"

    echo "  Config installed at $BB_BUILD/.config"
    local enabled
    enabled=$(grep '=y' "$BB_BUILD/.config" | wc -l)
    echo "  Enabled options: $enabled"
    echo ""
}

# =========================================================================
# Phase A-3: Apply VeridianOS patches
# =========================================================================
phase_patch() {
    echo "=== Phase A-3: Apply VeridianOS patches ==="

    if [ ! -d "$BB_PATCHES" ]; then
        echo "  No patches directory found at $BB_PATCHES"
        return
    fi

    cd "$BB_SRC"
    local applied=0

    for patch in "$BB_PATCHES"/*.patch; do
        [ -f "$patch" ] || continue
        local pname
        pname="$(basename "$patch")"
        echo -n "  Applying $pname... "
        if patch -p1 --forward --dry-run < "$patch" > /dev/null 2>&1; then
            patch -p1 --forward < "$patch" > /dev/null 2>&1
            echo "OK"
            applied=$((applied + 1))
        else
            echo "SKIPPED (already applied or conflict)"
        fi
    done

    echo "  Applied: $applied patches"
    echo ""
}

# =========================================================================
# Phase A-4 + A-5: Cross-compile BusyBox
# =========================================================================
phase_build() {
    echo "=== Phase A-4/A-5: Cross-compile BusyBox ==="

    if [ ! -x "$CC_WRAPPER" ]; then
        echo "  ERROR: CC wrapper not found at $CC_WRAPPER"
        exit 1
    fi

    cd "$BB_SRC"

    echo "  Building BusyBox..."
    make -j"$(nproc)" \
        CC="$CC_WRAPPER" \
        HOSTCC=gcc \
        LD="${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-ld" \
        EXTRA_CFLAGS="-D__veridian__ -DBB_GLOBAL_CONST=" \
        SKIP_STRIP=y \
        V=1 \
        O="$BB_BUILD" \
        busybox 2>&1 | tee "$WORK_DIR/busybox-build.log" | tail -20

    if [ -f "$BB_BUILD/busybox" ]; then
        echo ""
        echo "  SUCCESS: BusyBox binary produced"
        file "$BB_BUILD/busybox"
        ls -la "$BB_BUILD/busybox"

        # Verify static linking
        if "$READELF" -l "$BB_BUILD/busybox" 2>/dev/null | grep -q 'INTERP'; then
            echo "  WARNING: Binary is dynamically linked!"
        else
            echo "  Static linking: OK"
        fi

        # List compiled-in applets
        echo ""
        echo "  Applet count: $(grep -c '=y' "$BB_BUILD/.config" | head -1 || echo '?')"
    else
        echo ""
        echo "  FAILED: No busybox binary produced"
        echo "  Check build log: $WORK_DIR/busybox-build.log"
        exit 1
    fi
    echo ""
}

# =========================================================================
# Phase A-6: Package rootfs with BusyBox
# =========================================================================
phase_rootfs() {
    echo "=== Phase A-6: Package BusyBox rootfs ==="

    # Find BusyBox binary (may be in-tree or out-of-tree build)
    local BB_BIN=""
    if [ -f "$BB_BUILD/busybox" ]; then
        BB_BIN="$BB_BUILD/busybox"
    elif [ -f "$BB_SRC/busybox" ]; then
        BB_BIN="$BB_SRC/busybox"
    fi
    if [ -z "$BB_BIN" ]; then
        echo "  ERROR: BusyBox binary not found at $BB_BUILD/busybox or $BB_SRC/busybox"
        exit 1
    fi
    echo "  Found BusyBox binary: $BB_BIN"

    # Use .config from wherever it exists
    local BB_CFG=""
    if [ -f "$BB_BUILD/.config" ]; then
        BB_CFG="$BB_BUILD/.config"
    elif [ -f "$BB_SRC/.config" ]; then
        BB_CFG="$BB_SRC/.config"
    fi

    # Clean and create build directory
    rm -rf "$BUILD_DIR"
    mkdir -p "$BUILD_DIR"/{bin,sbin,usr/bin,usr/sbin,usr/src,tmp,var/tmp,dev,proc,etc}

    # Install BusyBox binary
    cp "$BB_BIN" "$BUILD_DIR/bin/busybox"
    chmod 755 "$BUILD_DIR/bin/busybox"
    echo "  + /bin/busybox ($(stat -c%s "$BUILD_DIR/bin/busybox" 2>/dev/null | awk '{printf "%d KB", $1/1024}'))"

    # Create applet symlinks.
    # Derived from applets.h + .config cross-reference.
    echo "  Creating applet symlinks..."
    local applet_count=0

    # bin/ applets (BB_DIR_BIN)
    for applet in ash sh cat chmod chown cp date df echo egrep false fgrep \
                  grep kill ln ls mkdir mv pid ps pwd rm rmdir sed sleep \
                  stat touch true uname; do
        ln -sf busybox "$BUILD_DIR/bin/$applet"
        applet_count=$((applet_count + 1))
    done

    # usr/bin/ applets (BB_DIR_USR_BIN)
    for applet in [ awk basename clear cmp comm cut diff dirname du env \
                  expr find fold free head hexdump id less nproc od paste \
                  printf readlink realpath seq sort strings tail tee test \
                  time tr uniq uptime wc which xargs xxd yes; do
        ln -sf ../../bin/busybox "$BUILD_DIR/usr/bin/$applet"
        applet_count=$((applet_count + 1))
    done

    echo "  Applet symlinks: $applet_count"

    # =====================================================================
    # Cross-compile and install sysinfo + edit (+ libcurses dependency)
    # =====================================================================
    echo "  Adding sysinfo/edit programs..."

    local libc_incdir="${LIBC_DIR}/include"
    local pgm_cflags="-std=c11 -static -O2 -nostdinc"
    pgm_cflags+=" -isystem ${libc_incdir} -isystem ${SYSROOT}/usr/include"
    pgm_cflags+=" -isystem ${GCC_INCDIR}"
    pgm_cflags+=" -fno-stack-protector -ffreestanding"
    pgm_cflags+=" -mno-red-zone -mcmodel=small"
    pgm_cflags+=" -Wall -Wextra -Wno-unused-parameter"
    local pgm_ldflags="-static -nostdlib -L${SYSROOT}/usr/lib"

    # sysinfo (fastfetch-inspired system info display)
    if [ -f "${PROGRAMS_DIR}/sysinfo/sysinfo.c" ]; then
        echo -n "    sysinfo... "
        if "$CC" $pgm_cflags $pgm_ldflags -o "$BUILD_DIR/bin/sysinfo" \
                "${SYSROOT}/usr/lib/crt0.o" "${PROGRAMS_DIR}/sysinfo/sysinfo.c" -lc 2>&1; then
            "$STRIP" "$BUILD_DIR/bin/sysinfo" 2>/dev/null || true
            local sz
            sz=$(stat -c%s "$BUILD_DIR/bin/sysinfo" 2>/dev/null || stat -f%z "$BUILD_DIR/bin/sysinfo" 2>/dev/null)
            echo "OK ($(( sz / 1024 )) KB)"
        else
            echo "FAILED"
        fi
    fi

    # Build libcurses.a for rootfs (needed by edit)
    local curses_src="${PROJECT_ROOT}/userland/libcurses"
    if [ -f "${curses_src}/curses.c" ]; then
        echo -n "    libcurses.a... "
        local curses_build="/tmp/VeridianOS/libcurses-rootfs-build"
        rm -rf "$curses_build"
        mkdir -p "$curses_build"
        local curses_cflags="-static -O2 -Wall -Wextra -Wno-unused-parameter"
        curses_cflags+=" -fno-stack-protector -ffreestanding -mno-red-zone -mcmodel=small"
        curses_cflags+=" -nostdinc -isystem ${SYSROOT}/usr/include -isystem ${GCC_INCDIR}"
        curses_cflags+=" -I${curses_src}"
        if "$CC" -c $curses_cflags -o "$curses_build/curses.o" "$curses_src/curses.c" 2>&1; then
            "${TOOLCHAIN_PREFIX}/bin/x86_64-veridian-ar" rcs "$curses_build/libcurses.a" "$curses_build/curses.o"
            # Install into rootfs
            mkdir -p "$BUILD_DIR/usr/lib" "$BUILD_DIR/usr/include"
            cp "$curses_build/libcurses.a" "$BUILD_DIR/usr/lib/libcurses.a"
            ln -sf libcurses.a "$BUILD_DIR/usr/lib/libncurses.a"
            cp "$curses_src/curses.h" "$BUILD_DIR/usr/include/curses.h"
            cp "$curses_src/ncurses.h" "$BUILD_DIR/usr/include/ncurses.h"
            local csz
            csz=$(stat -c%s "$curses_build/libcurses.a" 2>/dev/null || stat -f%z "$curses_build/libcurses.a" 2>/dev/null)
            echo "OK ($(( csz / 1024 )) KB)"
        else
            echo "FAILED"
        fi
    fi

    # edit (nano-inspired text editor, depends on libcurses)
    if [ -f "${PROGRAMS_DIR}/edit/edit.c" ] && [ -f "$BUILD_DIR/usr/lib/libcurses.a" ]; then
        echo -n "    edit... "
        if "$CC" $pgm_cflags $pgm_ldflags -o "$BUILD_DIR/bin/edit" \
                "${SYSROOT}/usr/lib/crt0.o" "${PROGRAMS_DIR}/edit/edit.c" \
                -L"$BUILD_DIR/usr/lib" -lcurses -lc 2>&1; then
            "$STRIP" "$BUILD_DIR/bin/edit" 2>/dev/null || true
            local esz
            esz=$(stat -c%s "$BUILD_DIR/bin/edit" 2>/dev/null || stat -f%z "$BUILD_DIR/bin/edit" 2>/dev/null)
            echo "OK ($(( esz / 1024 )) KB)"
        else
            echo "FAILED"
        fi
    fi

    # Cross-compile init (PID 1) for user-space boot
    local init_src="${PROJECT_ROOT}/userland/init/init.c"
    if [ -f "$init_src" ]; then
        echo -n "    init... "
        mkdir -p "$BUILD_DIR/sbin"
        if "$CC" $pgm_cflags $pgm_ldflags -o "$BUILD_DIR/sbin/init" \
                "${SYSROOT}/usr/lib/crt0.o" "$init_src" -lc 2>&1; then
            "$STRIP" "$BUILD_DIR/sbin/init" 2>/dev/null || true
            local isz
            isz=$(stat -c%s "$BUILD_DIR/sbin/init" 2>/dev/null || stat -f%z "$BUILD_DIR/sbin/init" 2>/dev/null)
            echo "OK ($(( isz / 1024 )) KB)"
        else
            echo "FAILED"
        fi
    fi

    # Copy source files for native compilation on VeridianOS
    echo "  Adding program source for native compilation..."
    mkdir -p "$BUILD_DIR/usr/src"
    [ -f "${PROGRAMS_DIR}/sysinfo/sysinfo.c" ] && cp "${PROGRAMS_DIR}/sysinfo/sysinfo.c" "$BUILD_DIR/usr/src/sysinfo.c"
    [ -f "${PROGRAMS_DIR}/edit/edit.c" ] && cp "${PROGRAMS_DIR}/edit/edit.c" "$BUILD_DIR/usr/src/edit.c"
    [ -f "${curses_src}/curses.c" ] && cp "${curses_src}/curses.c" "$BUILD_DIR/usr/src/curses.c"
    [ -f "${curses_src}/curses.h" ] && cp "${curses_src}/curses.h" "$BUILD_DIR/usr/src/curses.h"
    echo "    + /usr/src/{sysinfo.c,edit.c,curses.c,curses.h}"

    # Copy native build script (if it exists)
    local NATIVE_PGM_SCRIPT="${PROJECT_ROOT}/tools/native-programs/build-native-programs.sh"
    if [ -f "$NATIVE_PGM_SCRIPT" ]; then
        cp "$NATIVE_PGM_SCRIPT" "$BUILD_DIR/usr/src/build-native-programs.sh"
        chmod 755 "$BUILD_DIR/usr/src/build-native-programs.sh"
        echo "    + /usr/src/build-native-programs.sh"
    fi

    # Copy coreutils source for native compilation on VeridianOS
    echo "  Adding coreutils source for native compilation..."
    mkdir -p "$BUILD_DIR/usr/src/coreutils"
    for src in "$COREUTILS_DIR"/*.c; do
        [ -f "$src" ] || continue
        cp "$src" "$BUILD_DIR/usr/src/coreutils/"
    done
    local cu_count
    cu_count=$(ls "$BUILD_DIR/usr/src/coreutils/"*.c 2>/dev/null | wc -l)
    echo "    + /usr/src/coreutils/*.c ($cu_count files)"

    # Copy native coreutils build script
    local NATIVE_CU_SCRIPT="${PROJECT_ROOT}/tools/native-coreutils/build-native-coreutils.sh"
    if [ -f "$NATIVE_CU_SCRIPT" ]; then
        cp "$NATIVE_CU_SCRIPT" "$BUILD_DIR/usr/src/build-native-coreutils.sh"
        chmod 755 "$BUILD_DIR/usr/src/build-native-coreutils.sh"
        echo "    + /usr/src/build-native-coreutils.sh"
    fi

    # Copy test data files
    echo "  Adding test data..."
    printf 'Hello from VeridianOS\nCAT_PASS\n' > "$BUILD_DIR/usr/src/cat_test.txt"
    printf 'one two\nthree four\nfive\n' > "$BUILD_DIR/usr/src/wc_test.txt"
    printf 'cherry\napple\nbanana\n' > "$BUILD_DIR/usr/src/sort_test.txt"
    echo "  + /usr/src/{cat_test,wc_test,sort_test}.txt"

    # Add test script for automated validation (from repo)
    if [ -f "$PROJECT_ROOT/userland/tests/busybox_test.sh" ]; then
        cp "$PROJECT_ROOT/userland/tests/busybox_test.sh" "$BUILD_DIR/usr/src/busybox_test.sh"
    fi
    chmod 755 "$BUILD_DIR/usr/src/busybox_test.sh"
    echo "  + /usr/src/busybox_test.sh"

    # Include native GCC toolchain if available (for Phase C)
    if [ -d "$NATIVE_GCC_DIR/usr/bin" ]; then
        echo "  Adding native GCC toolchain..."
        mkdir -p "$BUILD_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/include"
        mkdir -p "$BUILD_DIR/usr/libexec/gcc/x86_64-veridian/14.2.0"
        mkdir -p "$BUILD_DIR/usr/include/sys"
        mkdir -p "$BUILD_DIR/usr/include/arpa"
        mkdir -p "$BUILD_DIR/usr/include/netinet"
        mkdir -p "$BUILD_DIR/usr/include/veridian"

        # Binaries
        for tool in gcc as ld ar; do
            if [ -f "$NATIVE_GCC_DIR/usr/bin/$tool" ]; then
                cp "$NATIVE_GCC_DIR/usr/bin/$tool" "$BUILD_DIR/usr/bin/"
                echo "    + /usr/bin/$tool"
            fi
        done
        ln -sf gcc "$BUILD_DIR/usr/bin/cc"

        # cc1, collect2
        for tool in cc1 collect2 ld as; do
            src="$NATIVE_GCC_DIR/usr/libexec/gcc/x86_64-veridian/14.2.0/$tool"
            [ -f "$src" ] && cp "$src" "$BUILD_DIR/usr/libexec/gcc/x86_64-veridian/14.2.0/"
        done

        # Libraries
        for f in crt0.o crti.o crtn.o libc.a; do
            [ -f "$NATIVE_GCC_DIR/usr/lib/$f" ] && cp "$NATIVE_GCC_DIR/usr/lib/$f" "$BUILD_DIR/usr/lib/"
        done
        for f in libgcc.a crtbegin.o crtend.o; do
            [ -f "$NATIVE_GCC_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/$f" ] && \
                cp "$NATIVE_GCC_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/$f" \
                   "$BUILD_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/"
        done

        # Headers -- copy sysroot headers (canonical, freshly rebuilt by phase_headers),
        # then fill gaps from native-gcc-static for any extras not in sysroot.
        # Sysroot must go LAST without -n so updated headers win over stale copies.
        [ -d "$NATIVE_GCC_DIR/usr/include" ] && cp -r "$NATIVE_GCC_DIR/usr/include/"* "$BUILD_DIR/usr/include/" 2>/dev/null || true
        [ -d "${SYSROOT}/usr/include" ] && cp -r "${SYSROOT}/usr/include/"* "$BUILD_DIR/usr/include/" 2>/dev/null || true

        # GCC internal headers (stdbool.h, stddef.h, stdarg.h, etc.) -- essential for compilation
        local gcc_inc="$NATIVE_GCC_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/include"
        if [ -d "$gcc_inc" ]; then
            cp -r "$gcc_inc/"* "$BUILD_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/include/" 2>/dev/null || true
            echo "    + GCC internal headers ($(ls "$gcc_inc" | wc -l) files)"
        fi

        # GCC specs
        cat > "$BUILD_DIR/usr/lib/gcc/x86_64-veridian/14.2.0/specs" << 'SPECEOF'
*linker:
ld
SPECEOF

        # GNU Make (for Phase C-4 stretch goal)
        local NATIVE_TOOLS="${PROJECT_ROOT}/target/native-tools-staging"
        if [ -f "$NATIVE_TOOLS/usr/bin/make" ]; then
            cp "$NATIVE_TOOLS/usr/bin/make" "$BUILD_DIR/usr/bin/"
            echo "    + /usr/bin/make"
        fi

        # readelf, nm, objdump, strip (useful for verification)
        for tool in readelf nm objdump strip; do
            if [ -f "$NATIVE_GCC_DIR/usr/bin/$tool" ]; then
                cp "$NATIVE_GCC_DIR/usr/bin/$tool" "$BUILD_DIR/usr/bin/"
            fi
        done
        echo "    + /usr/bin/{readelf,nm,objdump,strip}"
    fi

    # =====================================================================
    # Phase C: Package BusyBox source tree + generated headers for native
    # compilation on VeridianOS. Pre-generated headers avoid running Kconfig
    # natively; the build-busybox-native.sh script replaces Make.
    # =====================================================================
    if [ -d "$BB_SRC" ]; then
        echo "  Adding BusyBox source tree for Phase C native compilation..."
        local BB_ROOTFS_SRC="$BUILD_DIR/usr/src/busybox-${BUSYBOX_VERSION}"
        mkdir -p "$BB_ROOTFS_SRC/include"

        # Copy BusyBox source directories (only .c and .h files to save space)
        for srcdir in applets archival console-tools coreutils debianutils \
                      editors findutils include libbb libpwdgrp miscutils \
                      procps shell util-linux; do
            if [ -d "$BB_SRC/$srcdir" ]; then
                find "$BB_SRC/$srcdir" -type d | while read dir; do
                    local reldir="${dir#$BB_SRC/}"
                    mkdir -p "$BB_ROOTFS_SRC/$reldir"
                done
                find "$BB_SRC/$srcdir" \( -name '*.c' -o -name '*.h' \) -type f | while read f; do
                    local relf="${f#$BB_SRC/}"
                    cp "$f" "$BB_ROOTFS_SRC/$relf"
                done
            fi
        done
        local src_count
        src_count=$(find "$BB_ROOTFS_SRC" -name '*.c' -o -name '*.h' | wc -l)
        echo "    Source files: $src_count (.c + .h)"

        # Copy pre-generated build headers from cross-compilation
        # These are config-dependent, not source-dependent, so they're
        # valid for native compilation with the same .config.
        local GEN_HEADERS="$BB_BUILD/include"
        if [ -d "$GEN_HEADERS" ]; then
            for hdr in autoconf.h applets.h applet_tables.h \
                       bbconfigopts.h bbconfigopts_bz2.h \
                       common_bufsiz.h embedded_scripts.h \
                       NUM_APPLETS.h usage.h usage_compressed.h; do
                if [ -f "$GEN_HEADERS/$hdr" ]; then
                    cp "$GEN_HEADERS/$hdr" "$BB_ROOTFS_SRC/include/$hdr"
                fi
            done
            echo "    Generated headers: autoconf.h + 9 others"
        else
            echo "    WARNING: Generated headers not found at $GEN_HEADERS"
            echo "    Native compilation will fail without autoconf.h"
        fi

        # Copy native build script + file lists
        local NATIVE_SCRIPT="${PROJECT_ROOT}/tools/busybox/build-busybox-native.sh"
        if [ -f "$NATIVE_SCRIPT" ]; then
            cp "$NATIVE_SCRIPT" "$BUILD_DIR/usr/src/build-busybox-native.sh"
            chmod 755 "$BUILD_DIR/usr/src/build-busybox-native.sh"
            echo "    + /usr/src/build-busybox-native.sh"
        fi
        for lst in busybox-compile-list.txt busybox-obj-list.txt bb_ver.h; do
            if [ -f "${PROJECT_ROOT}/tools/busybox/$lst" ]; then
                cp "${PROJECT_ROOT}/tools/busybox/$lst" "$BUILD_DIR/usr/src/$lst"
                echo "    + /usr/src/$lst"
            fi
        done
    fi

    # Create the TAR archive
    echo ""
    echo "  Creating rootfs-busybox.tar..."
    cd "$BUILD_DIR"
    tar cf "$ROOTFS_TAR" --format=ustar bin/ sbin/ usr/ tmp/ var/ dev/ proc/ etc/
    cd "$PROJECT_ROOT"

    local total_files size
    total_files=$(tar tf "$ROOTFS_TAR" | wc -l)
    size=$(stat -c%s "$ROOTFS_TAR" 2>/dev/null || stat -f%z "$ROOTFS_TAR" 2>/dev/null)
    echo "  Total: $total_files entries, $size bytes ($(( size / 1024 )) KB)"
    echo ""

    echo "To boot with BusyBox rootfs:"
    echo "  ./build-kernel.sh x86_64 dev"
    echo "  qemu-system-x86_64 -enable-kvm \\"
    echo "    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \\"
    echo "    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \\"
    echo "    -device ide-hd,drive=disk0 \\"
    echo "    -drive file=target/rootfs-busybox.tar,if=none,id=vd0,format=raw \\"
    echo "    -device virtio-blk-pci,drive=vd0 \\"
    echo "    -serial stdio -display none -m 1024M"
}

# =========================================================================
# Phase: BlockFS Image (persistent root filesystem)
# =========================================================================
phase_blockfs_image() {
    echo "=== Phase: BlockFS Image ==="
    local MKFS_DIR="${PROJECT_ROOT}/tools/mkfs-blockfs"
    local MKFS_BIN="${MKFS_DIR}/target/x86_64-unknown-linux-gnu/release/mkfs-blockfs"
    local BLOCKFS_IMG="${PROJECT_ROOT}/target/rootfs-blockfs.img"
    local BLOCKFS_SIZE="${BLOCKFS_SIZE:-512}"

    echo "Building mkfs-blockfs tool..."
    (cd "$MKFS_DIR" && cargo build --release)

    if [ ! -x "$MKFS_BIN" ]; then
        echo "ERROR: mkfs-blockfs binary not found at $MKFS_BIN"
        exit 1
    fi

    if [ ! -d "$BUILD_DIR" ]; then
        echo "ERROR: rootfs build directory not found at $BUILD_DIR"
        echo "  Run '$0 rootfs' first to create the rootfs."
        exit 1
    fi

    echo "Creating ${BLOCKFS_SIZE}MB BlockFS image from $BUILD_DIR..."
    "$MKFS_BIN" \
        --output "$BLOCKFS_IMG" \
        --size "$BLOCKFS_SIZE" \
        --populate "$BUILD_DIR"

    echo ""
    echo "BlockFS image created: $BLOCKFS_IMG"
    echo ""
    echo "Boot with persistent storage:"
    echo "  ./build-kernel.sh x86_64 dev"
    echo "  qemu-system-x86_64 -enable-kvm \\"
    echo "    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \\"
    echo "    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \\"
    echo "    -device ide-hd,drive=disk0 \\"
    echo "    -drive file=target/rootfs-blockfs.img,if=none,id=vd0,format=raw \\"
    echo "    -device virtio-blk-pci,drive=vd0 \\"
    echo "    -serial stdio -display none -m 2048M"
}

# =========================================================================
# Main dispatch
# =========================================================================
main() {
    echo "=== VeridianOS BusyBox ${BUSYBOX_VERSION} Build ==="
    echo "Project root: $PROJECT_ROOT"
    echo "Cross-compiler: $CC"
    echo ""

    if [ ! -x "$CC" ]; then
        echo "ERROR: Cross-compiler not found at $CC"
        exit 1
    fi

    local phase="${1:-all}"

    case "$phase" in
        download)   phase_download ;;
        headers)    phase_headers ;;
        config)     phase_config ;;
        patch)      phase_patch ;;
        build)      phase_build ;;
        rootfs)     phase_rootfs ;;
        blockfs)    phase_blockfs_image ;;
        all)
            phase_download
            phase_headers
            phase_config
            phase_patch
            phase_build
            phase_rootfs
            ;;
        *)
            echo "Usage: $0 [download|headers|config|patch|build|rootfs|blockfs|all]"
            exit 1
            ;;
    esac
}

main "$@"

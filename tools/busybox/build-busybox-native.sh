#!/bin/ash
# build-busybox-native.sh -- Native BusyBox 1.36.1 compilation for VeridianOS
# Reads file lists from /usr/src/ to keep script small for ash.
# Usage: ash /usr/src/build-busybox-native.sh

BB_SRC="/usr/src/busybox-1.36.1"
OBJ_DIR="/tmp/busybox-obj"
CC="/usr/bin/gcc"
COMPILE_LIST="/usr/src/busybox-compile-list.txt"
OBJ_LIST_FILE="/usr/src/busybox-obj-list.txt"

CFLAGS_BASE="-std=gnu99 -nostdinc \
 -isystem /usr/include \
 -isystem /usr/lib/gcc/x86_64-veridian/14.2.0/include \
 -include ${BB_SRC}/include/autoconf.h \
 -include /usr/src/bb_ver.h \
 -I ${BB_SRC}/include -I ${BB_SRC}/libbb \
 -D_GNU_SOURCE -DNDEBUG \
 -D_LARGEFILE_SOURCE -D_LARGEFILE64_SOURCE -D_FILE_OFFSET_BITS=64 \
 -D__veridian__ -D__linux__ -DBB_GLOBAL_CONST= \
 -static -fno-stack-protector -ffreestanding \
 -mno-red-zone -mcmodel=small \
 -Wall -funsigned-char \
 -ffunction-sections -fdata-sections \
 -fno-builtin-strlen -fno-builtin-printf \
 -fomit-frame-pointer \
 -fno-unwind-tables -fno-asynchronous-unwind-tables \
 -finline-limit=0 -fno-guess-branch-probability \
 -falign-functions=1 -falign-jumps=1 -falign-labels=1 -falign-loops=1 \
 -Wno-unused-parameter -Wno-implicit-function-declaration \
 -Wno-return-type -Wno-format-security -Wno-int-conversion \
 -fno-optimize-strlen"

LDFLAGS="-static -nostdlib -ffreestanding"
COMPILED=0
FAILED=0
TOTAL=0

echo "=== VeridianOS Native BusyBox 1.36.1 Compilation ==="
echo "Source: $BB_SRC"
echo "Output: $OBJ_DIR"

# Prerequisites
if [ ! -f "$CC" ]; then
    echo "ERROR: gcc not found at $CC"; exit 1
fi
if [ ! -f "${BB_SRC}/include/autoconf.h" ]; then
    echo "ERROR: autoconf.h not found"; exit 1
fi
if [ ! -f "$COMPILE_LIST" ]; then
    echo "ERROR: compile list not found at $COMPILE_LIST"; exit 1
fi
if [ ! -f "$OBJ_LIST_FILE" ]; then
    echo "ERROR: obj list not found at $OBJ_LIST_FILE"; exit 1
fi

# Create output directories
for d in applets archival/libarchive console-tools \
         coreutils/libcoreutils debianutils editors \
         findutils libbb libpwdgrp miscutils procps \
         shell util-linux; do
    mkdir -p "${OBJ_DIR}/$d"
done

echo "=== Phase 1: Compiling source files ==="

# Read compile list: each line is "source.c:extra_include_dir"
while IFS=: read -r src extra; do
    obj="${OBJ_DIR}/${src%.c}.o"
    TOTAL=$(expr $TOTAL + 1)
    extra_flag=""
    if [ -n "$extra" ]; then
        extra_flag="-I ${BB_SRC}/${extra}"
    fi
    # Per-file optimization overrides:
    #   ash.c:  cc1 OOM at -O2 (large file) -- use -O1
    opt="-O2"
    if [ "$src" = "shell/ash.c" ]; then
        opt="-O1"
    fi
    echo "[CC] $src ($TOTAL/207)"
    if $CC $CFLAGS_BASE $opt $extra_flag -c -o "$obj" "${BB_SRC}/$src" 2>&1; then
        COMPILED=$(expr $COMPILED + 1)
    else
        echo "  FAIL: $src"
        FAILED=$(expr $FAILED + 1)
    fi
done < "$COMPILE_LIST"

echo ""
echo "=== Compilation Summary ==="
echo "  Total: $TOTAL  Compiled: $COMPILED  Failed: $FAILED"

if [ "$FAILED" -gt 0 ]; then
    echo "NATIVE_COMPILE_FAIL"; exit 1
fi

# Phase 2: Link
echo "=== Phase 2: Linking busybox binary ==="
OBJ_LIST=""
while read -r f; do
    OBJ_LIST="${OBJ_LIST} ${OBJ_DIR}/${f}"
done < "$OBJ_LIST_FILE"

echo "[LD] busybox (207 objects)"
$CC $LDFLAGS \
    -Wl,--start-group \
    $OBJ_LIST \
    -L /usr/lib -L /usr/lib/gcc/x86_64-veridian/14.2.0 \
    -lc -lgcc \
    -Wl,--end-group \
    -o /tmp/busybox-native 2>&1

echo ""
if [ -f /tmp/busybox-native ]; then
    echo "=== SUCCESS ==="
    echo "Binary: /tmp/busybox-native"
    echo "NATIVE_COMPILE_PASS"
else
    echo "=== LINK FAILED ==="
    echo "NATIVE_LINK_FAIL"
    exit 1
fi

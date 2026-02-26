#!/bin/ash
# build-native-coreutils.sh -- Native compilation of 6 coreutils on VeridianOS
#
# Compiles echo, cat, wc, ls, sort, and pipeline_test natively using
# GCC 14.2.0 running on VeridianOS itself. Follows the same CFLAGS/LDFLAGS
# pattern as build-native-programs.sh.
#
# Usage: ash /usr/src/build-native-coreutils.sh

CC=/usr/bin/gcc
CFLAGS="-std=c11 -static -O2 -nostdinc \
 -isystem /usr/include \
 -isystem /usr/lib/gcc/x86_64-veridian/14.2.0/include \
 -fno-stack-protector -ffreestanding \
 -mno-red-zone -mcmodel=small \
 -Wall -Wextra -Wno-unused-parameter \
 -Wno-implicit-function-declaration"
LDFLAGS="-static -nostdlib -ffreestanding"

SRC_DIR=/usr/src/coreutils
PASS=0
FAIL=0
TOTAL=6

echo "=== VeridianOS Native Coreutils Compilation ==="
echo "Compiler: $CC"
echo ""

# Prerequisites
if [ ! -f "$CC" ]; then
    echo "ERROR: gcc not found at $CC"; exit 1
fi
if [ ! -d "$SRC_DIR" ]; then
    echo "ERROR: coreutils source not found at $SRC_DIR"; exit 1
fi

# Compile each coreutil
for prog in echo cat wc ls sort pipeline_test; do
    src="$SRC_DIR/${prog}.c"
    out="/tmp/${prog}-native"

    if [ ! -f "$src" ]; then
        echo "  $prog: SKIP (source not found)"
        FAIL=$(expr $FAIL + 1)
        continue
    fi

    echo -n "  [CC+LD] ${prog}.c ... "
    $CC $CFLAGS $LDFLAGS -o "$out" \
        /usr/lib/crt0.o "$src" \
        -L /usr/lib -L /usr/lib/gcc/x86_64-veridian/14.2.0 \
        -lc -lgcc 2>&1

    if [ -f "$out" ]; then
        echo "OK"
        PASS=$(expr $PASS + 1)
    else
        echo "FAIL"
        FAIL=$(expr $FAIL + 1)
    fi
done

# Summary
echo ""
echo "=== Native Coreutils Summary ==="
echo "  Passed: $PASS/$TOTAL  Failed: $FAIL"
if [ "$PASS" -eq "$TOTAL" ]; then
    echo "NATIVE_COREUTILS_PASS"
else
    echo "NATIVE_COREUTILS_FAIL"
fi

#!/bin/ash
# build-native-programs.sh -- Native compilation of sysinfo + edit on VeridianOS
#
# Compiles libcurses.a, sysinfo, and edit natively using GCC 14.2.0 running
# on VeridianOS itself. Follows the same CFLAGS/LDFLAGS pattern as
# build-busybox-native.sh.
#
# Usage: ash /usr/src/build-native-programs.sh

CC=/usr/bin/gcc
AR=/usr/bin/ar
CFLAGS="-std=c11 -static -O2 -nostdinc \
 -isystem /usr/include \
 -isystem /usr/lib/gcc/x86_64-veridian/14.2.0/include \
 -fno-stack-protector -ffreestanding \
 -mno-red-zone -mcmodel=small \
 -Wall -Wextra -Wno-unused-parameter"
LDFLAGS="-static -nostdlib -ffreestanding"

PASS=0
FAIL=0

echo "=== VeridianOS Native Programs Compilation ==="
echo "Compiler: $CC"
echo ""

# Prerequisites
if [ ! -f "$CC" ]; then
    echo "ERROR: gcc not found at $CC"; exit 1
fi
if [ ! -f /usr/src/sysinfo.c ]; then
    echo "ERROR: sysinfo.c not found at /usr/src/sysinfo.c"; exit 1
fi
if [ ! -f /usr/src/edit.c ]; then
    echo "ERROR: edit.c not found at /usr/src/edit.c"; exit 1
fi
if [ ! -f /usr/src/curses.c ]; then
    echo "ERROR: curses.c not found at /usr/src/curses.c"; exit 1
fi

# 1. Build libcurses.a
echo "=== Building libcurses.a ==="
echo "[CC] curses.c"
$CC $CFLAGS -I /usr/src -c -o /tmp/curses.o /usr/src/curses.c 2>&1
if [ -f /tmp/curses.o ]; then
    $AR rcs /tmp/libcurses.a /tmp/curses.o
    if [ -f /tmp/libcurses.a ]; then
        echo "  libcurses.a: OK"
        # Install for link step
        cp /tmp/libcurses.a /usr/lib/libcurses.a
    else
        echo "  libcurses.a: FAIL (ar)"
        FAIL=$(expr $FAIL + 1)
    fi
else
    echo "  libcurses.a: FAIL (compile)"
    FAIL=$(expr $FAIL + 1)
fi

# 2. Build sysinfo
echo ""
echo "=== Building sysinfo ==="
echo "[CC+LD] sysinfo.c"
$CC $CFLAGS $LDFLAGS -o /tmp/sysinfo-native \
    /usr/lib/crt0.o /usr/src/sysinfo.c \
    -L /usr/lib -L /usr/lib/gcc/x86_64-veridian/14.2.0 \
    -lc -lgcc 2>&1
if [ -f /tmp/sysinfo-native ]; then
    echo "  sysinfo: OK"
    PASS=$(expr $PASS + 1)
else
    echo "  sysinfo: FAIL"
    FAIL=$(expr $FAIL + 1)
fi

# 3. Build edit (depends on libcurses.a)
echo ""
echo "=== Building edit ==="
echo "[CC+LD] edit.c"
$CC $CFLAGS $LDFLAGS -o /tmp/edit-native \
    /usr/lib/crt0.o /usr/src/edit.c \
    -I /usr/src \
    -L /usr/lib -L /usr/lib/gcc/x86_64-veridian/14.2.0 \
    -lcurses -lc -lgcc 2>&1
if [ -f /tmp/edit-native ]; then
    echo "  edit: OK"
    PASS=$(expr $PASS + 1)
else
    echo "  edit: FAIL"
    FAIL=$(expr $FAIL + 1)
fi

# Summary
echo ""
echo "=== Native Programs Summary ==="
echo "  Passed: $PASS  Failed: $FAIL"
if [ "$FAIL" -eq 0 ]; then
    echo "NATIVE_PROGRAMS_PASS"
else
    echo "NATIVE_PROGRAMS_FAIL"
fi

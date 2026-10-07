#!/usr/bin/env bash
# Check glibc_shim.c's ctype table against the host glibc's C-locale table.
#
# libstdc++.a is built against glibc and tests glibc's _IS* bits (little-endian
# _ISbit values, e.g. _ISupper = 0x0100). The shim once used plain low bits, so
# libstdc++ saw ' ' as upper and 'A' as not upper (review of the v0.26.0 stack,
# PR #14). This extracts the mask defines and the table from the shim, compiles
# them on the host and compares every entry for 0..127 with glibc's own table.
#
# Usage: tools/cross/musl-compat/test-ctype-table.sh   (needs a glibc host cc)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# From the mask defines through the end of the _ctype_b_table initializer.
awk '/^#define _GU /{on=1} on{print} on && /^};/{exit}' "$here/glibc_shim.c" > "$work/table.inc"
grep -q '_ctype_b_table' "$work/table.inc" || { echo "FAIL: table not found in glibc_shim.c" >&2; exit 1; }
# The range ends at the first "};": make sure that was the table's own end.
grep -qF '[128 + 0x7F]' "$work/table.inc" \
  || { echo "FAIL: extracted table is truncated (no entry for 0x7F)" >&2; exit 1; }

cat > "$work/t.c" <<'C'
#include <ctype.h>
#include <locale.h>
#include <stdio.h>
#include "table.inc"
int main(void) {
    setlocale(LC_ALL, "C");
    const unsigned short *glibc = *__ctype_b_loc();
    int bad = 0;
    for (int c = 0; c < 128; c++) {
        if (_ctype_b_table[128 + c] != glibc[c]) {
            printf("FAIL char 0x%02x: shim 0x%04x, glibc 0x%04x\n", c,
                   _ctype_b_table[128 + c], glibc[c]);
            bad++;
        }
    }
    if (!bad)
        printf("ok: shim ctype table matches glibc for 0..127\n");
    return bad != 0;
}
C
cc -std=gnu11 -o "$work/t" "$work/t.c"
"$work/t"

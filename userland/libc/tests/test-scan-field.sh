#!/usr/bin/env bash
# Host check of stdio.c's sscanf numeric field scanner (__scan_field): long
# zero padding, signs, widths and the 0/0x prefixes must read as C's strtoll
# reads them. Extracts the function from stdio.c and compiles it with the
# host cc. Usage: userland/libc/tests/test-scan-field.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
awk '/^static size_t __scan_field/{on=1} on{print} on&&/^}/{exit}' "$here/../src/stdio.c" > "$work/f.inc"
grep -q '__scan_field' "$work/f.inc" || { echo "FAIL: __scan_field not found" >&2; exit 1; }
cat > "$work/t.c" <<'C'
#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "f.inc"
static int check(const char *in, size_t width, int base, long long want, size_t want_adv) {
    char nb[64]; size_t sk;
    __scan_field(in, width, nb, sizeof nb, &sk);
    char *end; long long v = strtoll(nb, &end, base);
    size_t adv = sk + (size_t)(end - nb);
    int ok = v == want && adv == want_adv;
    printf("%s %-24.24s w=%zu -> %lld adv %zu\n", ok ? "ok  " : "FAIL", in, width, v, adv);
    return !ok;
}
int main(void) {
    char padded[120]; memset(padded, '0', 100); strcpy(padded + 100, "42 rest");
    int bad = 0;
    bad |= check(padded, 0, 10, 42, 102);
    bad |= check("-0005x", 0, 10, -5, 5);
    bad |= check("0x1f", 0, 0, 31, 4);
    bad |= check("000x1f", 0, 0, 0, 3);   /* octal "000", then "x1f" is the next input */
    bad |= check("0", 0, 10, 0, 1);
    bad |= check("00012345", 4, 10, 1, 4); /* width counts the zeros */
    bad |= check("12345", 3, 10, 123, 3);
    return bad;
}
C
cc -Wall -o "$work/t" "$work/t.c"
"$work/t"

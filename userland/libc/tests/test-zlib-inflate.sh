#!/usr/bin/env bash
# Host check of the native libc inflater (zlib.c) under AddressSanitizer:
# a dynamic block declaring more length/distance codes than deflate allows
# must be rejected (it overflowed a stack array), and a normal stream must
# still inflate. Usage: userland/libc/tests/test-zlib-inflate.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
lib="$here/.."
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/inc"
cp "$lib/include/zlib.h" "$lib/include/zconf.h" "$work/inc/"
cat > "$work/t.c" <<'C'
#include <stdio.h>
#include <string.h>
#include <zlib.h>
static unsigned char buf[256]; static unsigned bitpos;
static void put(unsigned v, int n) { for (int i = 0; i < n; i++, bitpos++) if (v >> i & 1) buf[bitpos / 8] |= 1u << (bitpos % 8); }
int main(void) {
    int bad = 0;
    /* zlib header (CM 8, no dict) then one dynamic block declaring
     * HLIT = 31 (288 codes) and HDIST = 31 (32 codes): 320 code lengths,
     * more than deflate allows (286 + 30). The code-length code is valid
     * (symbols 0 and 18 of length 1), and three runs of zeros (138, 138,
     * 44) fill all 320 entries. */
    buf[0] = 0x78; buf[1] = 0x9c; bitpos = 16;
    put(1, 1); put(2, 2); put(31, 5); put(31, 5); put(15, 4);
    static const int order[19] = {16,17,18,0,8,7,9,6,10,5,11,4,12,3,13,2,14,1,15};
    for (int i = 0; i < 19; i++) put(order[i] == 18 || order[i] == 0 ? 1 : 0, 3);
    put(1, 1); put(127, 7); put(1, 1); put(127, 7); put(1, 1); put(33, 7);
    unsigned char out[64];
    z_stream s; memset(&s, 0, sizeof s);
    s.next_in = buf; s.avail_in = sizeof buf; s.next_out = out; s.avail_out = sizeof out;
    inflateInit(&s);
    int r = inflate(&s, Z_FINISH);
    inflateEnd(&s);
    if (r != Z_DATA_ERROR) { printf("FAIL oversized code counts: inflate returned %d\n", r); bad = 1; }
    /* A normal stream still inflates. */
    static const unsigned char hello[] = {0x78,0x9c,0xcb,0x48,0xcd,0xc9,0xc9,0x07,0x00,0x06,0x2c,0x02,0x15};
    memset(&s, 0, sizeof s);
    s.next_in = (unsigned char *)hello; s.avail_in = sizeof hello; s.next_out = out; s.avail_out = sizeof out;
    inflateInit(&s); r = inflate(&s, Z_FINISH); inflateEnd(&s);
    if (r != Z_STREAM_END || s.total_out != 5 || memcmp(out, "hello", 5)) { printf("FAIL hello: %d\n", r); bad = 1; }
    puts(bad ? "zlib-inflate: FAIL" : "zlib-inflate: all passed");
    return bad;
}
C
cc -g -fsanitize=address,undefined -fno-sanitize-recover=all -I"$work/inc" \
    -o "$work/t" "$work/t.c" "$lib/src/zlib.c"
ASAN_OPTIONS=detect_leaks=0 "$work/t"

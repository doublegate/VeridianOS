#!/usr/bin/env bash
# Host check of the native libc PNG decoder (libpng_shim.c, inflating with
# the libc's own zlib.c): every color type and bit depth, non-interlaced and
# Adam7, all five row filters, IDAT split across chunks. A Python generator
# writes each PNG and the pixel rows it must decode to.
# Usage: userland/libc/tests/test-png-decode.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
lib="$here/.."
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/inc" "$work/data" "$work/gen"
# Only the headers under test, so the rest come from the host libc.
cp "$lib/include/png.h" "$lib/include/zlib.h" "$lib/include/zconf.h" "$work/inc/"
cat > "$work/gen/gen.py" <<'PY'
import os, random, struct, sys, zlib
out = sys.argv[1]; random.seed(7)
def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
def paeth(a, b, c):
    p = a + b - c; pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    return a if pa <= pb and pa <= pc else (b if pb <= pc else c)
def filt(row, prev, bpp, f):
    o = bytearray()
    for i, x in enumerate(row):
        a = row[i - bpp] if i >= bpp else 0; b = prev[i] if prev else 0
        c = prev[i - bpp] if prev and i >= bpp else 0
        o.append((x - [0, a, b, (a + b) // 2, paeth(a, b, c)][f]) & 0xff)
    return bytes([f]) + bytes(o)
def pack(pix, bits):  # pix: list of pixel ints, bits per pixel
    if bits >= 8: return b"".join(v.to_bytes(bits // 8, "big") for v in pix)
    o = bytearray((len(pix) * bits + 7) // 8)
    for i, v in enumerate(pix):
        bit = i * bits; o[bit // 8] |= v << (8 - bits - bit % 8)
    return bytes(o)
A7 = [(0,0,8,8),(4,0,8,8),(0,4,4,8),(2,0,4,4),(0,2,2,4),(1,0,2,2),(0,1,1,2)]
CH = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}
cases = []
for ct, depths in [(0,[1,2,4,8,16]),(2,[8,16]),(3,[1,2,4,8]),(4,[8,16]),(6,[8,16])]:
    for d in depths:
        for il in (0, 1):
            for (w, h) in [(1,1),(5,3),(13,11),(33,9)]:
                cases.append((ct, d, il, w, h))
with open(os.path.join(out, "list.txt"), "w") as L:
    for n, (ct, d, il, w, h) in enumerate(cases):
        bits = CH[ct] * d; bpp = max(1, bits // 8)
        img = [[random.getrandbits(bits) for _ in range(w)] for _ in range(h)]
        raw = bytearray()
        passes = A7 if il else [(0, 0, 1, 1)]
        for (x0, y0, dx, dy) in passes:
            if w <= x0 or h <= y0: continue
            prev = None
            for y in range(y0, h, dy):
                row = pack([img[y][x] for x in range(x0, w, dx)], bits)
                raw += filt(row, prev, bpp, random.randrange(5)); prev = row
        z = zlib.compress(bytes(raw), random.choice([0, 1, 6, 9]))
        # split IDAT across chunks to exercise accumulation
        idats = b"".join(chunk(b"IDAT", z[i:i + 50]) for i in range(0, len(z), 50))
        ihdr = struct.pack(">IIBBBBB", w, h, d, ct, 0, 0, il)
        extra = chunk(b"PLTE", bytes(range(48))) if ct == 3 else b""
        png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"tEXt", b"k\0v") + extra + idats + chunk(b"IEND", b"")
        open(os.path.join(out, f"{n}.png"), "wb").write(png)
        open(os.path.join(out, f"{n}.raw"), "wb").write(b"".join(pack(r, bits) for r in img))
        L.write(f"{n} {ct} {d} {il} {w} {h}\n")
print(len(cases), "cases")
PY
cat > "$work/t.c" <<'C'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <png.h>
static void rd(png_structp p, png_bytep b, size_t n) { if (fread(b, 1, n, (FILE *)png_get_io_ptr(p)) != n) memset(b, 0, n); }
int main(int argc, char **argv) {
    char path[512]; int n, ct, d, il; unsigned w, h, bad = 0, total = 0;
    snprintf(path, sizeof path, "%s/list.txt", argv[1]);
    FILE *L = fopen(path, "r");
    while (fscanf(L, "%d %d %d %d %u %u", &n, &ct, &d, &il, &w, &h) == 6) {
        snprintf(path, sizeof path, "%s/%d.png", argv[1], n); FILE *f = fopen(path, "rb");
        png_structp p = png_create_read_struct("1.6", NULL, NULL, NULL); png_infop i = png_create_info_struct(p);
        png_set_read_fn(p, f, rd); png_read_info(p, i);
        size_t rb = png_get_rowbytes(p, i);
        unsigned char *img = calloc(h, rb); png_bytep rows[64];
        for (unsigned y = 0; y < h; y++) rows[y] = img + y * rb;
        if (n % 2) png_read_image(p, rows); else png_read_rows(p, rows, NULL, h);
        snprintf(path, sizeof path, "%s/%d.raw", argv[1], n); FILE *r = fopen(path, "rb");
        unsigned char *want = malloc(h * rb); size_t got = fread(want, 1, h * rb, r);
        int ok = got == h * rb && memcmp(want, img, h * rb) == 0; total++;
        if (!ok) { bad++; printf("FAIL case %d: ct=%d depth=%d interlace=%d %ux%u rowbytes=%zu\n", n, ct, d, il, w, h, rb); }
        png_destroy_read_struct(&p, &i, NULL); fclose(f); fclose(r); free(img); free(want);
    }
    printf("png-decode: %s (%u/%u)\n", bad ? "FAIL" : "all passed", total - bad, total); return bad != 0;
}
C
python3 -I "$work/gen/gen.py" "$work/data" >/dev/null
for src in libpng_shim zlib; do
    cc -O2 -Wall -Wextra -Wno-unused-parameter -Werror -I"$work/inc" \
        -c "$lib/src/$src.c" -o "$work/$src.o"
done
cc -O1 -I"$work/inc" -o "$work/t" "$work/t.c" "$work/libpng_shim.o" "$work/zlib.o"
"$work/t" "$work/data"

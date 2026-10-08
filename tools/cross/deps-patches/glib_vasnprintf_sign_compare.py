"""GLib 2.90.1 (vendored gnulib vasnprintf.c): two comparisons of the int
`count` with size_t lengths (GCC -Wsign-compare). A negative count has
already left through the `count < 0` failure path above them, so the cast
to size_t is exact.

Usage: python3 glib_vasnprintf_sign_compare.py <glib>/glib/gnulib/vasnprintf.c
"""
import sys

p = sys.argv[1]
s = open(p).read()
edits = [
    ("                    if (count >= tmp_length)\n",
     "                    if ((size_t) count >= tmp_length)\n"),
    ("                    if (count > allocated - length)\n",
     "                    if ((size_t) count > allocated - length)\n"),
]
if all(old not in s and new in s for old, new in edits):
    sys.exit(0)  # already patched
for old, _ in edits:
    if s.count(old) != 1:
        sys.exit("unexpected vasnprintf.c")
for old, new in edits:
    s = s.replace(old, new)
open(p, "w").write(s)

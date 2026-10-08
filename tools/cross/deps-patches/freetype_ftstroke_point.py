"""FreeType 2.14.3: ft_stroke_border_get_counts() advances a `point` cursor
that is never read (GCC: variable 'point' set but not used). Removing it
changes nothing; the loop walks `tags`.

Usage: python3 freetype_ftstroke_point.py <freetype>/src/base/ftstroke.c
"""
import sys

p = sys.argv[1]
s = open(p).read()
old_decl = "    FT_Vector*  point      = border->points;\n"
old_loop = "for ( ; count > 0; count--, num_points++, point++, tags++ )"
new_loop = "for ( ; count > 0; count--, num_points++, tags++ )"
if old_loop not in s and new_loop in s:
    sys.exit(0)  # already patched (or fixed upstream)
if s.count(old_decl) != 1 or s.count(old_loop) != 1:
    sys.exit("unexpected ftstroke.c")
open(p, "w").write(s.replace(old_decl, "").replace(old_loop, new_loop))

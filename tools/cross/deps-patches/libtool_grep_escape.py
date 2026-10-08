"""Old libtool configure code: `$GREP " \\-L"` escapes a dash, which grep 3.8+
reports ("stray \\ before -"). The pattern means the same unescaped (the
leading space keeps "-L" from being an option), which is how newer libtool
writes it.

Usage: python3 libtool_grep_escape.py <configure>
"""
import sys

p = sys.argv[1]
s = open(p).read()
t = s.replace('$GREP " \\-L"', '$GREP " -L"').replace('$EGREP " \\-L"', '$EGREP " -L"')
if t != s:
    open(p, "w").write(t)

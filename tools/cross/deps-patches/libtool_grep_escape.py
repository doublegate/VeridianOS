"""Old libtool configure code: `$GREP " \\-L"` escapes a dash, which grep 3.8+
reports ("stray \\ before -"). The pattern means the same unescaped (the
leading space keeps "-L" from being an option); libtool 2.5 writes it as
" [-]L". The script fails if it finds neither form, so a regenerated
configure is noticed.

Usage: python3 libtool_grep_escape.py <configure>
"""

import sys
from pathlib import Path

p = sys.argv[1]
s = Path(p).read_text()
t = s.replace('$GREP " \\-L"', '$GREP " -L"').replace('$EGREP " \\-L"', '$EGREP " -L"')
if '" \\-L"' in t:
    sys.exit(f"{p}: an escaped ' \\-L' grep is left")
if t == s and '$GREP " -L"' not in s and '$EGREP " -L"' not in s:
    sys.exit(f"{p}: no libtool ' \\-L' grep found (libtool changed?)")
if t != s:
    Path(p).write_text(t)

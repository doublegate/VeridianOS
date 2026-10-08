"""Old libtool configure code: `$GREP " \\-L"` escapes a dash, which grep 3.8+
reports ("stray \\ before -"). The pattern means the same unescaped (the
leading space keeps "-L" from being an option), which is how newer libtool
writes it.

Usage: python3 libtool_grep_escape.py <configure>
"""

import sys
from pathlib import Path

p = sys.argv[1]
s = Path(p).read_text()
t = s.replace('$GREP " \\-L"', '$GREP " -L"').replace('$EGREP " \\-L"', '$EGREP " -L"')
if t != s:
    Path(p).write_text(t)

"""libxcb 1.17: warn about a missing dot only when doxygen is used.

configure.ac warns "dot not found - doxygen targets will be skipped"
whenever HAVE_DOT is no -- but XORG_WITH_DOXYGEN only looks for dot when
doxygen is enabled, and leaves HAVE_DOT at no otherwise, so a build with
--without-doxygen (no manual) always warns about a tool it never needs.
The warning now also requires doxygen. Both configure.ac and the
generated configure are changed, so a regenerated script keeps it.
Both keep their modification times. Idempotent.

Usage: libxcb_dot_warning.py <libxcb source directory>
"""

import os
import sys
from pathlib import Path


def write_keeping_mtime(path, text):
    """Write PATH, keeping its modification time: make compares the
    autotools files' times and would otherwise run aclocal, autoconf and
    automake again (versions the build host may not have)."""
    st = path.stat()
    path.write_text(text)
    os.utime(path, ns=(st.st_atime_ns, st.st_mtime_ns))


root = Path(sys.argv[1])
fixes = [
    (
        "configure.ac",
        'if test x"$HAVE_DOT" = xno; then',
        'if test x"$have_doxygen" = xyes && test x"$HAVE_DOT" = xno; then',
    ),
    (
        "configure",
        'if test x"$HAVE_DOT" = xno; then',
        'if test x"$have_doxygen" = xyes && test x"$HAVE_DOT" = xno; then',
    ),
]

for name, old, new in fixes:
    path = root / name
    text = path.read_text()
    if new in text:
        continue
    if text.count(old) != 1:
        sys.exit(f"{path}: expected one dot warning check, found {text.count(old)}")
    write_keeping_mtime(path, text.replace(old, new))

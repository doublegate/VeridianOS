"""libXi 1.8: warn about missing man page tools only when they are needed.

configure warns "xmlto or asciidoc not found - cannot create man pages
without it" whenever the tools are missing, but the release tarball ships
the man pages already made (man/*.man; configure's own INSTALL_MANPAGES
test right after it says so, and installs them). The warning now fires
only when they are missing and cannot be made. configure.ac and the
generated configure are both changed, keeping their modification times
(make would otherwise run aclocal and autoconf again). Idempotent.

Usage: libxi_man_warning.py <libXi source directory>
"""

import os
import sys
from pathlib import Path

root = Path(sys.argv[1])
OLD = "if test $have_doctools = no; then\n"
NEW = (
    "if test $have_doctools = no && "
    'test ! -f "$srcdir/man/XAllowDeviceEvents.man"; then\n'
)

for name in ("configure.ac", "configure"):
    path = root / name
    text = path.read_text()
    if NEW in text:
        continue
    if text.count(OLD) != 1:
        sys.exit(f"{path}: expected one man page tools check, found {text.count(OLD)}")
    st = path.stat()
    path.write_text(text.replace(OLD, NEW))
    os.utime(path, ns=(st.st_atime_ns, st.st_mtime_ns))

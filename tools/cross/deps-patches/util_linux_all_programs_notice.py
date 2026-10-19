"""util-linux 2.42: --enable/--disable-all-programs as a notice.

configure reports the default it was asked for with --disable-all-programs
("disable all programs and man pages by default") or --enable-all-programs
("force to build all programs by default") through AC_MSG_WARN, so a
library-only build -- the option's purpose -- always ends with a configure
warning. Neither message reports a problem; both become AC_MSG_NOTICE, in
configure.ac and in the generated configure (where a notice goes to the
log and to standard output, a warning to the log and to standard error).
Both keep their modification times. Idempotent.

Usage: util_linux_all_programs_notice.py <util-linux source directory>
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


MESSAGES = (
    "force to build all programs by default",
    "disable all programs and man pages by default",
)

root = Path(sys.argv[1])

path = root / "configure.ac"
text = path.read_text()
for message in MESSAGES:
    old = f"AC_MSG_WARN([{message}])"
    new = f"AC_MSG_NOTICE([{message}])"
    if new in text:
        continue
    if text.count(old) != 1:
        sys.exit(f"{path}: expected one '{old}', found {text.count(old)}")
    text = text.replace(old, new)
write_keeping_mtime(path, text)

path = root / "configure"
text = path.read_text()
for message in MESSAGES:
    old = (
        f'{{ printf "%s\\n" "$as_me:${{as_lineno-$LINENO}}: WARNING: {message}" >&5\n'
        f'printf "%s\\n" "$as_me: WARNING: {message}" >&2;}}'
    )
    new = (
        f'{{ printf "%s\\n" "$as_me:${{as_lineno-$LINENO}}: {message}" >&5\n'
        f'printf "%s\\n" "$as_me: {message}" >&6;}}'
    )
    if new in text:
        continue
    if text.count(old) != 1:
        sys.exit(f"{path}: expected one warning '{message}', found {text.count(old)}")
    text = text.replace(old, new)
write_keeping_mtime(path, text)

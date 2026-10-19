"""libtool: no warnings for the normal steps of a staged install.

The packages are installed into the sysroot with `make install DESTDIR=...`
while their libraries record /usr/lib, the target's directory. libtool's
install mode warns about three things that follow from that and are what
a staged install is meant to do:

- "relinking 'libX.la'": a library linked against another library of the
  same package is linked again, to drop the build tree from its run path;
- "'libX.la' has not been installed in '/usr/lib'": a program of the
  package is linked again for the same reason, as its library is staged,
  not in /usr/lib;
- "remember to run 'libtool --finish /usr/lib'": the finish step
  (ldconfig) belongs to the target system, and musl has no loader cache.

Under DESTDIR each of these is reported only with --verbose (func_verbose);
an install without DESTDIR still warns as before. Applied to a package's
generated libtool scripts after configure. Idempotent; each script must
have all three messages, in any of the quoting styles libtool 2.4.2 to 2.6
use.

Usage: libtool_staged_install.py <build directory>
"""

import re
import sys
from pathlib import Path

# func_warning "MESSAGE" for the three messages, any quoting.
WARNING = re.compile(
    r'func_warning ("(?:relinking |remember to run |[^"]*has not been installed in )[^"\n]*")'
)


def staged(match):
    message = match.group(1)
    return (
        f'if test -n "$DESTDIR"; then func_verbose {message}; '
        f"else func_warning {message}; fi"
    )


def is_libtool(path):
    try:
        with path.open(errors="replace") as f:
            head = f.read(4096)
    except OSError:
        return False
    return "libtool" in head and "Generated automatically by config.status" in head


root = Path(sys.argv[1])
scripts = [p for p in root.rglob("libtool") if p.is_file() and is_libtool(p)]
for path in scripts:
    text = path.read_text(errors="surrogateescape")
    if 'if test -n "$DESTDIR"; then func_verbose' in text:
        continue
    new, count = WARNING.subn(staged, text)
    if count != 3:
        sys.exit(f"{path}: expected the three staged-install warnings, found {count}")
    path.write_text(new, errors="surrogateescape")

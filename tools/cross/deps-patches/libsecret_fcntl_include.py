"""libsecret 0.21.8.2: secret-file-collection.c calls g_open (GLib's
gstdio.h maps it to open on Unix) with O_CREAT/O_RDWR, but includes only
<sys/file.h>, which pulls in <fcntl.h> on glibc and not on musl. POSIX puts
open and its flags in <fcntl.h>; include it.

Usage: python3 libsecret_fcntl_include.py <libsecret>/libsecret/secret-file-collection.c
"""

import sys
from pathlib import Path

p = sys.argv[1]
s = Path(p).read_text()
old = "#include <sys/file.h>\n"
new = "#include <sys/file.h>\n#include <fcntl.h>\n"
if new in s:
    sys.exit(0)  # already patched
if s.count(old) != 1:
    sys.exit("unexpected secret-file-collection.c")
Path(p).write_text(s.replace(old, new))

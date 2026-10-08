"""libndp 1.9: pass sendto() a struct sockaddr pointer.

mysendto6() passes &sin6 (struct sockaddr_in6 *) to sendto(). glibc's
prototype takes any sockaddr pointer through a transparent union; musl's is
POSIX's const struct sockaddr *, so GCC 14+ rejects the call
(-Wincompatible-pointer-types is an error). The explicit cast is the POSIX
idiom; the address passed is the same.

Usage: libndp_sendto_cast.py <libndp/libndp.c>
"""

import sys
from pathlib import Path

path = sys.argv[1]
text = Path(path).read_text()
old = "\tret = sendto(sockfd, buf, buflen, flags, &sin6, sizeof(sin6));\n"
new = "\tret = sendto(sockfd, buf, buflen, flags, (struct sockaddr *) &sin6, sizeof(sin6));\n"
if not (new in text and old not in text):
    if text.count(old) != 1:
        sys.exit(f"{path}: unexpected source for the sendto call")
    Path(path).write_text(text.replace(old, new))

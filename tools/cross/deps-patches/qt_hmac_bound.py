"""Qt 6.12: make xored()'s block-size bound visible to the optimizer.

qcryptographichash.cpp's xored() (HMAC key padding) gives "hints for the
optimizer" about the block size with Q_ASSERT, which a release build
compiles to nothing. GCC 16 then vectorises the copy loop in 16-byte steps
without knowing that size() <= maxHashBlockSize(), and warns
(-Wstringop-overflow) that the last step could write over the
QSmallByteArray's size byte. The size never exceeds the maximum (it is a
QSmallByteArray invariant). Q_UNREACHABLE on the out-of-range branch asserts
in debug builds and tells the compiler in release builds, as the comment
intends. (Q_PRESUME does not help: GCC loses the assumption made inside
its lambda.)

Usage: qt_hmac_bound.py <src/corelib/tools/qcryptographichash.cpp>
"""

import sys
from pathlib import Path

path = sys.argv[1]
text = Path(path).read_text()
old = (
    "    // some hints for the optimizer:\n"
    "    Q_ASSERT(block.size() >= minHashBlockSize());\n"
    "    Q_ASSERT(block.size() <= maxHashBlockSize());\n"
    "    Q_ASSERT(block.size() % gcdHashBlockSize() == 0);\n"
)
new = old + ("    if (block.size() > maxHashBlockSize())\n        Q_UNREACHABLE();\n")
if new not in text:
    if text.count(old) != 1:
        sys.exit(f"{path}: unexpected source for xored()")
    Path(path).write_text(text.replace(old, new))

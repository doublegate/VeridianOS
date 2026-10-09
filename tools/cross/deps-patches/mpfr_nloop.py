"""MPFR 4.2.2: drop the unused Ziv loop counters in exp2m1 and exp10m1.

mpfr_exp2m1 and mpfr_exp10m1 count their Ziv loop iterations in `nloop`
and never read it, which GCC 16 reports (-Wunused-but-set-variable). The
loops become `for (;;)` and the variables go. Idempotent.

Usage: mpfr_nloop.py <MPFR source directory>
"""

import sys
from pathlib import Path

root = Path(sys.argv[1])
for name in ("src/exp2m1.c", "src/exp10m1.c"):
    path = root / name
    text = path.read_text()
    edits = [
        ("  int inexact, nloop;\n", "  int inexact;\n"),
        ("  for (nloop = 0;; nloop++)\n", "  for (;;)\n"),
    ]
    if "nloop" not in text:
        continue
    for old, new in edits:
        if text.count(old) != 1:
            sys.exit(f"{path}: expected one '{old.strip()}', found {text.count(old)}")
        text = text.replace(old, new)
    if "nloop" in text:
        sys.exit(f"{path}: nloop is still used")
    path.write_text(text)

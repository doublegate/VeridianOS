#!/usr/bin/env python3
"""Initialize the zstd v0.1 decoder's out-parameters at their declarations.

ZSTDv01_getcBlockSize() and ZSTD_decodeSeqHeaders() fill these on every
path that does not return an error, and their callers return on an error,
but GCC cannot see that across the calls and reports -Wmaybe-uninitialized
for each (zstd 1.5.7, lib/legacy/zstd_v01.c). The initial values are never
read; behaviour is unchanged.

Usage: zstd_legacy_v01_init.py <zstd_v01.c>
"""
import sys

FIXES = (
    ("    blockProperties_t litbp;\n",
     "    blockProperties_t litbp = { bt_compressed, 0 };\n"),
    ("    size_t errorCode, dumpsLength;\n    const BYTE* litPtr = litStart;\n",
     "    size_t errorCode, dumpsLength = 0;\n    const BYTE* litPtr = litStart;\n"),
    ("    int nbSeq;\n    const BYTE* dumps;\n",
     "    int nbSeq = 0;\n    const BYTE* dumps = NULL;\n"),
    ("    size_t errorCode=0;\n    blockProperties_t blockProperties;\n",
     "    size_t errorCode=0;\n    blockProperties_t blockProperties = { bt_compressed, 0 };\n"),
)

path = sys.argv[1]
src = open(path).read()
for old, new in FIXES:
    if new in src and old not in src:
        continue
    if src.count(old) != 1:
        sys.exit(f"{path}: unexpected source for {old.strip()!r}")
    src = src.replace(old, new)
open(path, "w").write(src)

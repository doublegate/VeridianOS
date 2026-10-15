#!/usr/bin/env python3
"""Make a -p1 unified patch from exact replacements in a source tree.

Usage: mkpatch.py SRCROOT OUT.patch HEADER_FILE EDITS.json
EDITS.json: [{"file": "rel/path", "old": "...", "new": "..."}, ...]
Each old text must occur exactly once. The header file's text (the
patch's explanation) goes first. The patch is checked with patch
--dry-run against SRCROOT.
"""

import difflib
import json
import subprocess
import sys
from pathlib import Path

root, out, header, edits = (
    Path(sys.argv[1]),
    Path(sys.argv[2]),
    Path(sys.argv[3]),
    sys.argv[4],
)
edits = json.loads(Path(edits).read_text())
by_file = {}
for e in edits:
    by_file.setdefault(e["file"], []).append(e)
chunks = [header.read_text().rstrip("\n") + "\n\n"]
for rel, es in by_file.items():
    old_text = (root / rel).read_text()
    new_text = old_text
    for e in es:
        n = new_text.count(e["old"])
        if n != 1:
            sys.exit(f"{rel}: expected one match, found {n}: {e['old'][:60]!r}")
        new_text = new_text.replace(e["old"], e["new"])
    diff = difflib.unified_diff(
        old_text.splitlines(keepends=True),
        new_text.splitlines(keepends=True),
        fromfile=f"a/{rel}",
        tofile=f"b/{rel}",
    )
    chunks.append("".join(diff))
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text("".join(chunks))
r = subprocess.run(
    ["patch", "-d", str(root), "-p1", "--forward", "--dry-run"],
    stdin=out.open(),
    capture_output=True,
    check=False,
    text=True,
)
print(r.stdout, r.stderr)
sys.exit(r.returncode)

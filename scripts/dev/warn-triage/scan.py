#!/usr/bin/env python3
"""Inventory warnings in the VeridianOS KDE cross-build log (read-only).

Usage: scan.py BUILD.log OUT.json
Classifies every warning line (compiler, linker, libtool, configure, meson,
CMake, moc/automoc, wayland-scanner, bison/flex, kdoctools, python) by
phase and package and writes the records to OUT.json; prints the counts.
Then: analyze.py OUT.json
"""

import collections
import json
import re
import sys

LOG = sys.argv[1]
OUT = sys.argv[2]

phase_re = re.compile(r"^ Phase: (\S+)")
hdr_re = re.compile(
    r"^\[build-[\w-]+\] (?:Building|Rebuilding|Cross-compiling|Installing) (.+?)(?:\.\.\.)?$"
)
ansi = re.compile(r"\x1b\[[0-9;]*[mK]")
flag_re = re.compile(r"\[(-W[^\]]+)\]\s*$")
# compiler diagnostic: path:line[:col]: warning: msg
cc_re = re.compile(
    r"^(?P<file>[^\s:][^:]*?):(?P<line>\d+)(?:[:.](?P<col>\d+)(?:-\d+)?)?: warning: (?P<msg>.*)$"
)
cc_nofile_re = re.compile(
    r"^(?P<tool>cc1(?:plus)?|<command-line>|<built-in>|gcc|g\+\+|[\w./+-]*-g(?:cc|\+\+)): warning: (?P<msg>.*)$"
)
ld_re = re.compile(
    r"(?:^|/|\s)(?:[\w.-]*-)?ld(?:\.lld|\.bfd|\.gold)?: warning: (?P<msg>.*)$"
)
cm_re = re.compile(
    r"^CMake (Warning|Deprecation Warning)( \(dev\))?( at ([^\s:]+):(\d+))?"
)

with open(LOG, encoding="utf-8", errors="replace") as fh:
    lines = fh.read().split("\n")

phase = "pre"
pkg = "pre"
recs = []  # warning records
cmake = []
for i, raw in enumerate(lines):
    s = ansi.sub("", raw).rstrip("\r")
    m = phase_re.match(s)
    if m:
        phase = m.group(1)
        pkg = phase + ":(phase-start)"
        continue
    m = hdr_re.match(s)
    if m and not s.startswith("[build-kwin] Applying"):
        pkg = m.group(1).strip()
        continue
    m = cm_re.match(s)
    if m:
        # collect block until a line that is not indented after first content
        blk = [s]
        j = i + 1
        seen = False
        while j < len(lines) and j < i + 60:
            t = ansi.sub("", lines[j]).rstrip()
            if t == "":
                if seen and (j + 1 >= len(lines) or not lines[j + 1].startswith(" ")):
                    break
                j += 1
                continue
            if not t.startswith(" ") and seen:
                break
            blk.append(t)
            seen = True
            j += 1
        cmake.append(
            {
                "phase": phase,
                "pkg": pkg,
                "line": i + 1,
                "kind": m.group(1) + (m.group(2) or ""),
                "where": (m.group(4) or "") + (":" + m.group(5) if m.group(5) else ""),
                "text": "\n".join(blk),
            }
        )
        continue
    t = s.lstrip()
    kind = None
    d = {"phase": phase, "pkg": pkg, "line": i + 1, "text": t}
    if (
        "warning:" in t
        or "Warning:" in t
        or "WARNING" in t
        or t.startswith(("Warn:", "I/O warning"))
        or "SyntaxWarning" in t
        or "AutoMoc warning" in t
    ):
        if ld_re.search(t):
            kind = "linker"
            d["msg"] = ld_re.search(t).group("msg")
        elif t.startswith(("libtool: warning", "libtool:   warning")):
            kind = "libtool"
        elif t.startswith("configure: WARNING") or re.match(
            r"^configure:\d+: WARNING", t
        ):
            kind = "configure"
        elif (
            re.match(r"^(WARNING|DEPRECATION|NOTICE):", t)
            or re.match(r"^[\w./-]*meson\.build:\d+:\d+: WARNING", t)
            or ("meson.build" in t and "WARNING" in t)
        ):
            kind = "meson"
        elif cc_re.match(t):
            mm = cc_re.match(t)
            f = mm.group("file")
            msg = mm.group("msg")
            if "[-Wyacc]" in msg or f.endswith((".y", ".l")):
                kind = "bison/flex"
            elif f.endswith(".xml") or "since version" in msg:
                kind = "wayland-scanner"
            else:
                kind = "compiler"
            d.update(file=f, ln=mm.group("line"), col=mm.group("col") or "", msg=msg)
            fm = flag_re.search(msg)
            d["flag"] = fm.group(1) if fm else "(none)"
        elif cc_nofile_re.match(t):
            mm = cc_nofile_re.match(t)
            kind = "compiler"
            d.update(file=mm.group("tool"), ln="", col="", msg=mm.group("msg"))
            fm = flag_re.search(mm.group("msg"))
            d["flag"] = fm.group(1) if fm else "(none)"
        elif "warning:" in t and re.match(
            r"^(egrep|fgrep|grep|sed|make(\[\d+\])?|ar|ranlib|objcopy|strip|install|ln|cp)\b",
            t,
        ):
            kind = "other-tool"
        elif "AutoMoc warning" in t or re.match(r"^Warning: .*\.h:\d+", t):
            kind = "moc/automoc"
        elif t.startswith(("Warn:", "I/O warning")):
            kind = "kdoctools/meinproc"
        elif "SyntaxWarning" in t:
            kind = "python"
        elif (
            "warning:" in t
            or t.startswith(("-- Warning", "Warning:"))
            or "WARNING:" in t
        ):
            kind = "other"
    if kind:
        d["kind"] = kind
        recs.append(d)

with open(OUT, "w") as fh:
    json.dump({"recs": recs, "cmake": cmake}, fh)
c = collections.Counter(r["kind"] for r in recs)
print(c, "cmake", len(cmake))

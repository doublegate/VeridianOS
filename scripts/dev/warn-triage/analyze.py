#!/usr/bin/env python3
"""Group the compiler-like warnings of a scan.py inventory by phase,
package, flag and origin (sysroot header, toolchain header, package).

Usage: analyze.py RECS.json   (written by scan.py LOG RECS.json)
"""

import collections
import json
import os
import re
import sys

with open(sys.argv[1]) as fh:
    d = json.load(fh)
R = [r for r in d["recs"] if r["kind"] in ("compiler", "bison/flex", "wayland-scanner")]


def origin(f):
    m = re.search(r"musl-sysroot/usr/include/([^/]+)(?:/([^/]+))?", f)
    if m:
        return "sysroot:" + m.group(1)
    m = re.search(r"musl-toolchain/", f)
    if m:
        return "toolchain-hdr"
    m = re.search(r"cross-build/[^/]+/([^/]+)/", f)
    if m:
        return m.group(1)
    m = re.match(r"^(?:\.\./)+([^/]+)/", f)
    if m:
        return m.group(1)
    return "."


def tail(f):
    f = os.path.normpath(f)
    return "/".join(f.split("/")[-3:])


for r in R:
    r["origin"] = origin(r["file"])
    r["key"] = (tail(r["file"]), r["ln"], r["col"], r["msg"])
print("raw compiler-ish", len(R))
ph = collections.OrderedDict()
for r in R:
    ph.setdefault(r["phase"], [0, set()])
    ph[r["phase"]][0] += 1
    ph[r["phase"]][1].add(r["key"])
for p, (n, s) in ph.items():
    print("PHASE", p, n, len(s))
pk = collections.defaultdict(lambda: [0, set(), collections.Counter()])
for r in R:
    x = pk[(r["phase"], r["pkg"])]
    x[0] += 1
    x[1].add(r["key"])
for (p, k), (n, s, _) in sorted(pk.items(), key=lambda kv: -len(kv[1][1])):
    fl = collections.Counter(
        next(rr["flag"] for rr in R if rr["key"] == kk) for kk in s
    )
    print(
        "PKG",
        p,
        "|",
        k,
        "|",
        n,
        len(s),
        "|",
        ", ".join(f"{a}:{b}" for a, b in fl.most_common(6)),
    )
print("=========== GROUPS")
G = collections.defaultdict(dict)
for r in R:
    G[(r["phase"], r["pkg"], r["flag"])].setdefault(r["key"], r)
for (p, k, f), m in sorted(G.items(), key=lambda kv: -len(kv[1]))[:45]:
    rs = list(m.values())
    oc = collections.Counter(x["origin"] for x in rs)
    files = collections.Counter(tail(x["file"]) for x in rs)
    msgs = collections.Counter(
        re.sub(r"'[^']*'|‘[^’]*’", "'X'", x["msg"])[:90] for x in rs
    )
    print(f"\n## {len(rs)} | {p} | {k} | {f} | origins={dict(oc.most_common(4))}")
    print("  files:", files.most_common(4))
    print("  msgs:", msgs.most_common(3))
    print("  REP:", rs[0]["text"][:400])

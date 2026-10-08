#!/usr/bin/env python3
"""Fix two kinds of meson lint in upstream build files.

version-checks SRC MIN
    Every `if meson.version().version_compare('>= V')` in SRC's meson.build
    files with V not above MIN (the project's minimum meson version) is
    always true, and meson warns ("Conditional on version '>= V' always
    evaluates to true"). The if-branch stays, one indentation level less;
    an else-branch (for older meson) goes.

copy-config FILE...
    A configure_file() given an empty configuration_data() only copies its
    input, and meson warns ("Got an empty configuration_data() object and
    found no substitutions"). Such a call becomes `copy: true`, which is
    what meson suggests and does the same.

Every change must find what it expects; a file that no longer matches
stops the build.
"""
import os
import re
import sys

CHECK = re.compile(r"^(\s*)if meson\.version\(\)\.version_compare\('>=\s*([0-9.]+)'\)\s*$")


def version(text):
    return tuple(int(x) for x in text.split("."))


def block_end(lines, start):
    """Indices of the matching else (or None) and endif for the if at start."""
    depth, else_at = 0, None
    for i in range(start + 1, len(lines)):
        word = lines[i].strip().split("(")[0].split(" ")[0]
        if word == "if":
            depth += 1
        elif word == "endif":
            if depth == 0:
                return else_at, i
            depth -= 1
        elif word in ("else", "elif") and depth == 0:
            if word == "elif":
                sys.exit(f"elif after a version check at line {start + 1}")
            else_at = i
    sys.exit(f"unterminated if at line {start + 1}")


def dedent(line, indent):
    if line.strip() == "":
        return line
    stripped = line.lstrip("\t ")
    return indent + stripped if len(line) - len(stripped) > len(indent) else line


def version_checks(src, minimum):
    changed = 0
    for root, _, files in os.walk(src):
        for name in files:
            if name not in ("meson.build",):
                continue
            path = os.path.join(root, name)
            lines = open(path).read().splitlines(keepends=True)
            i, out, touched = 0, [], False
            while i < len(lines):
                m = CHECK.match(lines[i])
                if not m or version(m.group(2)) > minimum:
                    out.append(lines[i])
                    i += 1
                    continue
                else_at, end = block_end(lines, i)
                body = lines[i + 1:else_at if else_at is not None else end]
                # Re-indent the kept branch to the if's own indentation
                # (nested lines keep their relative depth).
                base = min((len(l) - len(l.lstrip("\t ")) for l in body if l.strip()), default=0)
                cut = base - len(m.group(1))
                out.extend(l[cut:] if l.strip() else l for l in body)
                i, touched = end + 1, True
                changed += 1
            if touched:
                open(path, "w").write("".join(out))
    return changed


def copy_config(paths):
    for path in paths:
        text = open(path).read()
        old, new = "    configuration: configuration_data(),\n", "    copy: true,\n"
        if new in text and old not in text:
            continue
        if text.count(old) != 1:
            sys.exit(f"{path}: expected one empty configuration_data()")
        open(path, "w").write(text.replace(old, new))


if __name__ == "__main__":
    if sys.argv[1] == "version-checks":
        src, minimum = sys.argv[2], version(sys.argv[3])
        version_checks(src, minimum)
        remaining = []
        for root, _, files in os.walk(src):
            for name in files:
                if name == "meson.build":
                    for n, line in enumerate(open(os.path.join(root, name)), 1):
                        m = CHECK.match(line)
                        if m and version(m.group(2)) <= minimum:
                            remaining.append(f"{root}/{name}:{n}")
        if remaining:
            sys.exit("unhandled version checks: " + ", ".join(remaining))
    elif sys.argv[1] == "copy-config":
        copy_config(sys.argv[2:])
    else:
        sys.exit(f"unknown command {sys.argv[1]}")

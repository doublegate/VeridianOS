#!/usr/bin/env python3
"""Run a project's own build-time tools through the cross-compiling emulator.

Programs are built shared (ADR 0010), so a tool a project builds and then
runs during its build is a VeridianOS program that the host can run only
through the musl loader with the sysroot's libraries: the toolchain sets
CMAKE_CROSSCOMPILING_EMULATOR to that runner. CMake prepends the emulator
when a custom command's COMMAND is the tool's target name, but not when it
is `$<TARGET_FILE:tool>`. The host's own /lib/ld-musl then ran the tool with
the host's libraries (glibc builds of Qt), and it failed to relocate.

This rewrites `COMMAND $<TARGET_FILE:<target>>` to `COMMAND <target>` for
the named targets, which must be executables the project itself adds
(CMake then also adds the dependency on them, as the expression did).

Usage: cmake_run_built_tools.py <CMakeLists.txt> <target>...
"""
import sys

path, targets = sys.argv[1], sys.argv[2:]
if not targets:
    sys.exit("usage: cmake_run_built_tools.py <CMakeLists.txt> <target>...")
text = open(path).read()
for target in targets:
    old = f"COMMAND $<TARGET_FILE:{target}>"
    new = f"COMMAND {target} "
    if old not in text and new not in text:
        sys.exit(f"{path}: no command runs {target}")
    text = text.replace(old + " ", new)
    if old in text:
        sys.exit(f"{path}: {target} runs in an unexpected form")
open(path, "w").write(text)

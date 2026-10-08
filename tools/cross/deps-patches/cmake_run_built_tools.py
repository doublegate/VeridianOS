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

Usage: python3 -I cmake_run_built_tools.py <CMakeLists.txt> <target>...
"""

import re
import sys
from pathlib import Path

path, targets = Path(sys.argv[1]), sys.argv[2:]
if not targets:
    sys.exit("usage: cmake_run_built_tools.py <CMakeLists.txt> <target>...")
text = path.read_text()
for target in targets:
    expr = re.compile(r"COMMAND \$<TARGET_FILE:" + re.escape(target) + r">(?=\s)")
    done = re.compile(r"COMMAND " + re.escape(target) + r"(?=\s)")
    if not expr.search(text) and not done.search(text):
        sys.exit(f"{path}: no command runs {target}")
    text = expr.sub("COMMAND " + target.replace("\\", "\\\\"), text)
path.write_text(text)

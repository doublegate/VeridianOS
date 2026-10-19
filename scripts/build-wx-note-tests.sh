#!/usr/bin/env bash
# Build the programs that test tools/veridian-elfctl (ADR 0017) into a
# rootfs bin directory that already holds wx_needed_test:
#   wx_plain_test        wx_needed_test without the note (-DWX_NO_NOTE),
#                        as a binary built for Linux: W^X applies;
#   wx_patched_test      wx_plain_test with the note added by the tool;
#   wx_patched_dyn_test  the same, dynamically linked (PT_INTERP, PT_PHDR);
#   wx_cleared_test      wx_needed_test with the bit cleared in place.
# musl_runtime_test runs them.
#
# Usage: scripts/build-wx-note-tests.sh MUSL_CC BIN_DIR
set -euo pipefail

cc="$1"
bin="$2"
root="$(cd "$(dirname "$0")/.." && pwd)"
src="${root}/userland/tests/wx_needed_test.c"
tool_dir="${root}/tools/veridian-elfctl"
tool="${tool_dir}/target/release/veridian-elfctl"

(cd "${tool_dir}" && cargo build --release --quiet)
[[ -x "${tool}" ]] || { echo "veridian-elfctl not built at ${tool}" >&2; exit 1; }

"${cc}" -static-pie -O2 -Wall -Wextra -DWX_NO_NOTE -o "${bin}/wx_plain_test" "${src}"
"${cc}" -O2 -Wall -Wextra -DWX_NO_NOTE -o "${bin}/wx_plain_dyn_test" "${src}"
"${tool}" set wxneeded "${bin}/wx_plain_test" -o "${bin}/wx_patched_test" >/dev/null
"${tool}" set wxneeded "${bin}/wx_plain_dyn_test" -o "${bin}/wx_patched_dyn_test" >/dev/null
"${tool}" clear wxneeded "${bin}/wx_needed_test" -o "${bin}/wx_cleared_test" >/dev/null
rm -f "${bin}/wx_plain_dyn_test"
# The tool's own reading of what it wrote.
"${tool}" show "${bin}/wx_patched_test" | grep -q 'wxneeded: yes'
"${tool}" show "${bin}/wx_patched_dyn_test" | grep -q 'wxneeded: yes'
"${tool}" show "${bin}/wx_cleared_test" | grep -q 'wxneeded: no'

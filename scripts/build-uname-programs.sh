#!/usr/bin/env bash
# Install what Linux uname mode (ADR 0021) needs into a root filesystem
# tree, with the musl compiler MUSL_CC:
#   /bin/linux-uname                 runs a command in Linux mode
#   /usr/share/config.site and       name the build system for autoconf
#   /usr/local/share/config.site     configure scripts (uname -s says
#                                    VeridianOS, which config.guess does
#                                    not know)
#   /etc/profile                     exports CONFIG_SITE for login shells
# With --tests, also the programs musl_runtime_test runs to check the mode
# (each prints the system name uname reports):
#   uname_plain_test     no note: the caller's mode;
#   uname_linux_test     <veridian/linux-uname.h>: Linux mode at exec;
#   uname_gnu_tag_test   a GNU ABI tag naming Linux, as glibc programs carry;
#   uname_patched_test   uname_plain_test with the note added by
#                        tools/veridian-elfctl.
#
# Usage: scripts/build-uname-programs.sh MUSL_CC ROOT_DIR [--tests]
set -euo pipefail

cc="$1"
dest="$2"
tests="${3:-}"
root="$(cd "$(dirname "$0")/.." && pwd)"
include="${root}/userland/libc/include"
files="${root}/tools/cross/rootfs"

mkdir -p "${dest}/bin" "${dest}/etc" "${dest}/usr/share" "${dest}/usr/local/share"
"${cc}" -static-pie -O2 -Wall -Wextra -idirafter "${include}" \
    -o "${dest}/bin/linux-uname" "${root}/userland/programs/linux-uname/linux-uname.c"
install -m 644 "${files}/usr/share/config.site" "${dest}/usr/share/config.site"
install -m 644 "${files}/usr/share/config.site" "${dest}/usr/local/share/config.site"
install -m 644 "${files}/etc/profile" "${dest}/etc/profile"

[[ "${tests}" == "--tests" ]] || exit 0

src="${root}/userland/tests/uname_print_test.c"
bin="${dest}/bin"
tool_dir="${root}/tools/veridian-elfctl"
tool="${tool_dir}/target/release/veridian-elfctl"
(cd "${tool_dir}" && cargo build --release --quiet)
[[ -x "${tool}" ]] || { echo "veridian-elfctl not built at ${tool}" >&2; exit 1; }

"${cc}" -static-pie -O2 -Wall -Wextra -o "${bin}/uname_plain_test" "${src}"
"${cc}" -static-pie -O2 -Wall -Wextra -idirafter "${include}" -DUNAME_LINUX_NOTE \
    -o "${bin}/uname_linux_test" "${src}"
"${cc}" -static-pie -O2 -Wall -Wextra -DUNAME_GNU_TAG -o "${bin}/uname_gnu_tag_test" "${src}"
"${tool}" set linux-uname "${bin}/uname_plain_test" -o "${bin}/uname_patched_test" >/dev/null
# The tool's own reading of what was built and written.
"${tool}" show "${bin}/uname_linux_test" | grep -q 'linux-uname: yes'
"${tool}" show "${bin}/uname_patched_test" | grep -q 'linux-uname: yes'
"${tool}" show "${bin}/uname_plain_test" | grep -q 'linux-uname: no'

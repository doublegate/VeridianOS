#!/usr/bin/env bash
# musl and the GCC cross toolchain for AArch64 or RISC-V
#
# Usage: tools/cross/build-musl-arch.sh aarch64|riscv64
#
# There is no cross compiler for the architecture to build musl with the
# first time, so the sequence bootstraps one (tools/cross/veridian-paths.sh
# gives each architecture its own sysroot, toolchain and build trees):
#
#   0. build-linux-headers.sh: the architecture's Linux UAPI headers, which
#      build-musl.sh installs into the sysroot (LINUX_HEADERS_DIR);
#   1. build-musl.sh: clang builds musl for the target (no compiler runtime);
#   2. build-musl-toolchain.sh: binutils, the compiler and a static libgcc
#      (stage 1: nothing dynamic links against clang's libc.so);
#   3. build-musl.sh: musl again, with that compiler and its libgcc;
#   4. build-musl-toolchain.sh: the whole toolchain (C, C++, shared
#      runtimes) against that musl, and its checks (programs run under
#      qemu-user).
#
# Each step skips what is already done, so the script can be re-run.

set -euo pipefail

arch="${1:-}"
case "${arch}" in
    aarch64 | riscv64) ;;
    *) echo "usage: $0 aarch64|riscv64" >&2; exit 2 ;;
esac
command -v "qemu-${arch}" >/dev/null || { echo "qemu-${arch} (qemu-user) is needed for the checks" >&2; exit 1; }

dir="$(cd "$(dirname "$0")" && pwd)"
# The architecture's own locations, never ones inherited for another.
unset VERIDIAN_SYSROOT VERIDIAN_TOOLCHAIN VERIDIAN_TARGET VERIDIAN_CROSS_BUILD VERIDIAN_CC VERIDIAN_CXX
export VERIDIAN_ARCH="${arch}"

"${dir}/build-linux-headers.sh"
# shellcheck source=veridian-paths.sh
cross_build="$(source "${dir}/veridian-paths.sh" && echo "${VERIDIAN_CROSS_BUILD}")"
export LINUX_HEADERS_DIR="${cross_build}/linux-headers/include"
"${dir}/build-musl.sh"
"${dir}/build-musl-toolchain.sh"
"${dir}/build-musl.sh"
"${dir}/build-musl-toolchain.sh"

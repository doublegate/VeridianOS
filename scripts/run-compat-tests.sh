#!/usr/bin/env bash
# Run the conformance suites (LTP syscalls, the Open POSIX Test Suite, musl
# libc-test) on the compat root filesystem and compare the results with
# tests/compat/expected-<arch>.tsv.
#
# The image comes from scripts/build-compat-rootfs.sh <arch>; the kernel from
# ./build-kernel.sh <arch> dev. tools/compat/compat.py does the work: it
# boots QEMU (in the background, killing it when done; never `timeout`),
# starts the in-guest runner (userland/tests/compat/run.sh), and when a test
# takes the guest down (panic, hang, a killed runner) records it and boots
# again from the next test.
#
# Usage: scripts/run-compat-tests.sh [x86_64|aarch64|riscv64] [compat.py run options]
#   --suites ltp-syscalls,open-posix,libc-test   (default: all three)
#   --update     accept these results as the expected list
#   --strict     fail on any difference, not only on regressions
#   --timeout N  seconds per test (60 under KVM, 180 under TCG)
#   --smp N
#   --jobs N     guests at once; --smoke only the tests expected to pass;
#   --resume     only the tests results.tsv has no result for
# Output: target/compat/<arch>/results.tsv, times.tsv, serial-NNN.log.
# Exit:   0 with no regressions (a test expected to PASS that does not).
#
# A full run takes about an hour on x86_64 (KVM, --jobs 3) and about four on
# AArch64 and RISC-V (TCG, --jobs 7); tests/compat/README.md has the procedure.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
arch="${1:-x86_64}"
[[ $# -gt 0 ]] && shift
case "$arch" in
x86_64 | aarch64 | riscv64) ;;
*)
    echo "usage: $0 [x86_64|aarch64|riscv64] [options]" >&2
    exit 2
    ;;
esac
exec python3 -I "$root/tools/compat/compat.py" run --arch "$arch" "$@"

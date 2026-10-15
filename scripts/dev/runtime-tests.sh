#!/usr/bin/env bash
# Rebuild what the x86_64 user-space runtime suite runs, then run it:
# the dev kernel, the rootfs staging tree (which compiles audit_runtime_test
# and, with the musl toolchain, musl_runtime_test), the BlockFS image, and
# scripts/run-rootfs-tests.sh with the musl checks required.
#
# Usage: scripts/dev/runtime-tests.sh [prompt-wait-seconds]   (default 900)
set -u -o pipefail
root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)" || exit 2
cd "$root" || exit 2
log_dir="${LOG_DIR:-target/dev-logs}"
mkdir -p "$log_dir"
./build-kernel.sh x86_64 dev >"$log_dir/runtime-kernel.log" 2>&1 ||
    { echo "kernel build failed ($log_dir/runtime-kernel.log)"; exit 1; }
scripts/build-busybox-rootfs.sh rootfs >"$log_dir/runtime-rootfs.log" 2>&1 ||
    { echo "rootfs staging failed ($log_dir/runtime-rootfs.log)"; exit 1; }
scripts/build-busybox-rootfs.sh blockfs >"$log_dir/runtime-blockfs.log" 2>&1 ||
    { echo "BlockFS image failed ($log_dir/runtime-blockfs.log)"; exit 1; }
REQUIRE_MUSL=1 scripts/run-rootfs-tests.sh "${1:-900}"

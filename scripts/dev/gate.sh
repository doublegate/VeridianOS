#!/usr/bin/env bash
# Kernel quality gate, the one to pass before every commit:
#   fmt check, clippy (-D warnings) on the three bare-metal targets and the
#   host library, and the host unit tests. With --boot also build every
#   architecture and boot each in QEMU (scripts/boot-test.sh); with --rootfs
#   also run the x86_64 user-space runtime suite (scripts/run-rootfs-tests.sh,
#   musl checks required). The boots (and with --rootfs the runtime suite)
#   then run again on four CPUs, since the `smp` feature is on by default;
#   --no-smp skips that second round (--smp is accepted and is the default).
#   Exits non-zero if any step fails; each step's output is kept in
#   ${LOG_DIR:-target/dev-logs}.
#
# Usage: scripts/dev/gate.sh [--boot] [--rootfs] [--no-smp]
set -u
root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)" || exit 2
cd "$root" || exit 2
boot=0 rootfs=0 smp=1
for arg in "$@"; do
    case $arg in
        --boot) boot=1 ;;
        --rootfs) rootfs=1 ;;
        --smp) smp=1 ;;
        --no-smp) smp=0 ;;
        *) echo "usage: $0 [--boot] [--rootfs] [--no-smp]" >&2; exit 2 ;;
    esac
done
log_dir="${LOG_DIR:-target/dev-logs}"
mkdir -p "$log_dir"
bs=("-Zbuild-std=core,compiler_builtins,alloc" "-Zbuild-std-features=compiler-builtins-mem")
rc=0
# step NAME COMMAND...: run, log to $log_dir/NAME.log, report rc.
step() {
    local name=$1
    shift
    "$@" >"$log_dir/$name.log" 2>&1
    local r=$?
    echo "=== rc=$r: $name"
    if [ $r -ne 0 ]; then
        rc=1
        tail -30 "$log_dir/$name.log"
    fi
}
step fmt cargo fmt --all --check
step clippy-x86_64 cargo clippy --target targets/x86_64-veridian.json -p veridian-kernel "${bs[@]}" -- -D warnings
step clippy-aarch64 cargo clippy --target aarch64-unknown-none-softfloat -p veridian-kernel "${bs[@]}" -- -D warnings
step clippy-riscv64 cargo clippy --target riscv64imac-unknown-none-elf -p veridian-kernel "${bs[@]}" -- -D warnings
step clippy-host cargo clippy --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu -- -D warnings
step host-tests cargo test --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu
grep -a 'test result' "$log_dir/host-tests.log"
if [ $boot -eq 1 ] || [ $rootfs -eq 1 ]; then
    step build ./build-kernel.sh all dev
fi
if [ $boot -eq 1 ]; then
    for arch in x86_64 aarch64 riscv64; do
        step "boot-$arch" scripts/boot-test.sh "$arch"
        tail -1 "$log_dir/boot-$arch.log"
    done
fi
if [ $rootfs -eq 1 ]; then
    # Stage first: it compiles the runtime test programs, so a change to
    # their sources is tested (blockfs alone packs the old binaries).
    step rootfs-stage scripts/build-busybox-rootfs.sh rootfs
    step rootfs-image scripts/build-busybox-rootfs.sh blockfs
    REQUIRE_MUSL=1 step rootfs-tests scripts/run-rootfs-tests.sh 900
    tail -1 "$log_dir/rootfs-tests.log"
fi
if [ $smp -eq 1 ] && { [ $boot -eq 1 ] || [ $rootfs -eq 1 ]; }; then
    # The same kernel (the `smp` feature is on by default) on four CPUs.
    if [ $boot -eq 1 ]; then
        for arch in x86_64 aarch64 riscv64; do
            SMP=4 EXPECT_CPUS=4 step "boot-smp-$arch" scripts/boot-test.sh "$arch"
            tail -1 "$log_dir/boot-smp-$arch.log"
        done
    fi
    if [ $rootfs -eq 1 ]; then
        SMP=4 REQUIRE_MUSL=1 step rootfs-tests-smp scripts/run-rootfs-tests.sh 900
        tail -1 "$log_dir/rootfs-tests-smp.log"
    fi
fi
echo "=== GATE rc=$rc"
exit $rc

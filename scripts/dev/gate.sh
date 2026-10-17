#!/usr/bin/env bash
# Kernel quality gate, the one to pass before every commit:
#   fmt check, clippy (-D warnings) on the three bare-metal targets and the
#   host library (also with every feature, as CI's Quick Checks), the
#   inline-assembly flags lint (scripts/dev/check-asm-flags.py), the rest of
#   CI's Quick Checks (syscall numbers, libc host tests, vsh), and the host
#   unit tests. With --boot also build every
#   architecture and boot each in QEMU (scripts/boot-test.sh); with --rootfs
#   also run the user-space runtime suites (scripts/run-rootfs-tests.sh,
#   musl checks required): x86_64's, then AArch64's and RISC-V's musl roots
#   (scripts/build-musl-rootfs.sh). The boots (and with --rootfs the runtime suite)
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
# Inline assembly that claims preserves_flags must not set the flags: a
# block that did made release AArch64 boot tests report passes as failures.
step asm-flags python3 "$root/scripts/dev/check-asm-flags.py"
# The rest of CI's Quick Checks: generated syscall numbers in sync with the
# generator (a hand edit of numbers.rs failed CI once), the native libc
# host tests, and vsh's formatting and build.
step syscall-numbers python3 "$root/tools/compat/gen_syscalls.py" --check
step libc-host-tests bash -c 'for t in "$0"/userland/libc/tests/test-*.sh; do bash "$t" || exit 1; done' "$root"
step vsh-fmt bash -c 'cd "$0/userland/vsh" && cargo fmt -- --check' "$root"
step vsh-build bash -c 'cd "$0/userland/vsh" && cargo build --target x86_64-unknown-none -Zbuild-std=core,compiler_builtins,alloc' "$root"
step clippy-x86_64 cargo clippy --target targets/x86_64-veridian.json -p veridian-kernel "${bs[@]}" -- -D warnings
step clippy-aarch64 cargo clippy --target aarch64-unknown-none-softfloat -p veridian-kernel "${bs[@]}" -- -D warnings
step clippy-riscv64 cargo clippy --target riscv64imac-unknown-none-elf -p veridian-kernel "${bs[@]}" -- -D warnings
step clippy-host cargo clippy --lib --features alloc -p veridian-kernel --target x86_64-unknown-linux-gnu -- -D warnings
# CI's Quick Checks step: the host build with every feature (e.g. `trace`).
step clippy-all-features cargo clippy --lib --all-features -- -D warnings
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
    # The debug RISC-V and AArch64 kernels again on a slow clock (16 ns per
    # instruction): a timer tick that outlasts its period must not livelock
    # the boot, as one did on CI's slower runners. KVM rules this out on
    # x86_64.
    QEMU_EXTRA="-icount shift=4" step boot-riscv64-slow scripts/boot-test.sh riscv64 300
    tail -1 "$log_dir/boot-riscv64-slow.log"
    QEMU_EXTRA="-icount shift=4" step boot-aarch64-slow scripts/boot-test.sh aarch64 300
    tail -1 "$log_dir/boot-aarch64-slow.log"
    # The AArch64 arm64 Image (HA-03), which QEMU boots through the Linux
    # protocol: it must also get the device tree in x0 (HA-01).
    AARCH64_IMAGE=1 step boot-aarch64-image scripts/boot-test.sh aarch64
    tail -1 "$log_dir/boot-aarch64-image.log"
    step boot-aarch64-image-dtb grep -q "Device tree passed in x0" "$root/target/boot-logs/boot-aarch64.log"
    # The same Image loaded 4 MiB higher: the MMU-on code measures where it
    # runs, so the kernel must not depend on its default load address.
    AARCH64_LOAD_ADDR=0x40600000 step boot-aarch64-relocated scripts/boot-test.sh aarch64
    tail -1 "$log_dir/boot-aarch64-relocated.log"
    # A CPU with FEAT_BTI (QEMU's `max`): the BTI boot tests take their
    # enforcing branch, which must really have run.
    AARCH64_CPU=max step boot-aarch64-max scripts/boot-test.sh aarch64
    tail -1 "$log_dir/boot-aarch64-max.log"
    step boot-aarch64-max-bti grep -q "guarded pages for user programs: enforced" "$root/target/boot-logs/boot-aarch64.log"
    # Release kernels too, as CI boots them: optimisation exposes what a
    # debug build hides (an asm block's false preserves_flags made release
    # AArch64 report passing tests as failed).
    step build-release ./build-kernel.sh all release
    for arch in x86_64 aarch64 riscv64; do
        PROFILE=release step "boot-release-$arch" scripts/boot-test.sh "$arch"
        tail -1 "$log_dir/boot-release-$arch.log"
    done
fi
if [ $rootfs -eq 1 ]; then
    # Stage first: it compiles the runtime test programs, so a change to
    # their sources is tested (blockfs alone packs the old binaries).
    step rootfs-stage scripts/build-busybox-rootfs.sh rootfs
    step rootfs-image scripts/build-busybox-rootfs.sh blockfs
    REQUIRE_MUSL=1 step rootfs-tests scripts/run-rootfs-tests.sh 900
    tail -1 "$log_dir/rootfs-tests.log"
    # AArch64 and RISC-V: the musl BusyBox root (built from source, so the
    # test programs are current) under TCG.
    for arch in aarch64 riscv64; do
        step "rootfs-stage-$arch" scripts/build-musl-rootfs.sh "$arch"
        ARCH=$arch REQUIRE_MUSL=1 step "rootfs-tests-$arch" scripts/run-rootfs-tests.sh
        tail -1 "$log_dir/rootfs-tests-$arch.log"
    done
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
        # The musl roots the rootfs stage built, on four emulated CPUs.
        for arch in aarch64 riscv64; do
            ARCH=$arch SMP=4 REQUIRE_MUSL=1 step "rootfs-tests-smp-$arch" scripts/run-rootfs-tests.sh
            tail -1 "$log_dir/rootfs-tests-smp-$arch.log"
        done
    fi
fi
echo "=== GATE rc=$rc"
exit $rc

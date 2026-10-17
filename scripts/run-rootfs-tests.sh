#!/usr/bin/env bash
# Run the user-space runtime suite: boot the dev kernel with the BusyBox
# BlockFS rootfs, let the BusyBox self-test run (BUSYBOX_ALL_PASS), then run
# /bin/audit_runtime_test at the shell prompt and require every check to
# pass ("AUDIT-RUNTIME: N/N").
#
# ARCH=x86_64 (default) boots the UEFI image under KVM with the native-libc
# root (scripts/build-busybox-rootfs.sh), whose self-test the kernel runs at
# boot. ARCH=aarch64 or riscv64 boots the kernel ELF under TCG with the musl
# root (scripts/build-musl-rootfs.sh <arch>), and runs the self-test from the
# shell prompt.
#
# These exercise exec, fork/exit, fds, stat, permissions, sockets and the
# user-copy paths, none of which the in-kernel boot tests reach.
#
# Prerequisites:
#   ./build-kernel.sh x86_64 dev
#   ./scripts/build-busybox-rootfs.sh all   (builds audit_runtime_test too)
#
# QEMU runs in the background and is killed when done (never `timeout`).
#
# Usage: scripts/run-rootfs-tests.sh [prompt-wait-seconds]
# Env:   ARCH (x86_64, aarch64, riscv64), REQUIRE_MUSL=1 (musl_runtime_test
#        must be present),
#        LOG_DIR (default target/boot-logs), ROOTFS (default
#        target/rootfs-blockfs.img), TEST_WAIT (seconds for the audit suite,
#        120), MUSL_WAIT (seconds for the musl suite, 600; it runs for
#        several minutes, and a result ends the wait at once), SMP (CPU
#        count, default 1)
set -u

root="$(cd "$(dirname "$0")/.." && pwd)"
prompt_wait="${1:-240}"
test_wait="${TEST_WAIT:-120}"
musl_wait="${MUSL_WAIT:-600}"
arch="${ARCH:-x86_64}"
log_dir="${LOG_DIR:-$root/target/boot-logs}"
mkdir -p "$log_dir"
suffix=""
[[ $arch == x86_64 ]] || suffix="-$arch"
log="$log_dir/rootfs-tests$suffix.log"
fifo="$log_dir/rootfs-serial$suffix.fifo"

case "$arch" in
x86_64)
    rootfs="${ROOTFS:-$root/target/rootfs-blockfs.img}"
    img="$root/target/x86_64-veridian/debug/veridian-uefi.img"
    ovmf=""
    for f in /usr/share/edk2/x64/OVMF.4m.fd /usr/share/OVMF/OVMF_CODE_4M.fd \
        /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2/ovmf/OVMF_CODE.fd /usr/share/qemu/OVMF.fd; do
        [[ -r $f ]] && { ovmf="$f"; break; }
    done
    [[ -n $ovmf ]] || { echo "FAIL: no OVMF firmware found"; exit 2; }
    accel=(-cpu qemu64)
    [[ -w /dev/kvm ]] && accel=(-enable-kvm -cpu host)
    # shellcheck disable=SC2054 # the commas are in QEMU option values
    qemu=(qemu-system-x86_64 "${accel[@]}"
        -drive "if=pflash,format=raw,readonly=on,file=$ovmf"
        -drive "id=disk0,if=none,format=raw,file=$img,snapshot=on"
        -device ide-hd,drive=disk0
        -drive "file=$rootfs,if=none,id=vd0,format=raw,snapshot=on"
        -device virtio-blk-pci,drive=vd0)
    # The gs_base check exercises argv (it is x86_64's).
    argv_test=gs_base
    ;;
aarch64 | riscv64)
    rootfs="${ROOTFS:-$root/target/rootfs-musl-$arch.img}"
    if [[ $arch == aarch64 ]]; then
        img="$root/target/aarch64-unknown-none-softfloat/debug/veridian-kernel"
        qemu=(qemu-system-aarch64 -M virt -cpu cortex-a72)
    else
        img="$root/target/riscv64imac-unknown-none-elf/debug/veridian-kernel"
        qemu=(qemu-system-riscv64 -M virt -bios default)
    fi
    # shellcheck disable=SC2054 # the commas are in QEMU option values
    qemu+=(-kernel "$img" -device ramfb
        -drive "file=$rootfs,if=none,id=vd0,format=raw,snapshot=on"
        -device virtio-blk-device,drive=vd0)
    argv_test=sched
    # Emulated (TCG): slower than KVM.
    prompt_wait="${1:-600}"
    test_wait="${TEST_WAIT:-900}"
    musl_wait="${MUSL_WAIT:-2400}"
    ;;
*)
    echo "FAIL: unknown ARCH $arch"
    exit 2
    ;;
esac
for f in "$img" "$rootfs"; do
    [[ -r $f ]] || { echo "FAIL: $f not built"; exit 2; }
done

rm -f "$fifo"
mkfifo "$fifo"
"${qemu[@]}" -smp "${SMP:-1}" -serial stdio -display none -m 2048M <"$fifo" >"$log" 2>&1 &
pid=$!
exec 3>"$fifo" # keep the write end open for the whole session

finish() {
    exec 3>&-
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    rm -f "$fifo"
}

wait_for() { # pattern seconds
    local waited=0
    until grep -aqE "$1" "$log" 2>/dev/null; do
        ((waited >= $2)) && return 1
        kill -0 "$pid" 2>/dev/null || return 1
        sleep 1
        waited=$((waited + 1))
    done
}

if ! wait_for 'root@veridian' "$prompt_wait"; then
    finish
    echo "FAIL: no shell prompt within ${prompt_wait}s (log: $log)"
    tail -20 "$log"
    exit 1
fi
sleep 2
# Only x86_64's kernel runs the BusyBox self-test at boot.
if [[ $arch != x86_64 ]]; then
    printf '/bin/ash /usr/src/busybox_test.sh\r' >&3
    wait_for 'BUSYBOX_ALL_PASS|BUSYBOX_SOME_FAIL' "$test_wait"
    sleep 1
fi
printf '/bin/audit_runtime_test\r' >&3
wait_for 'AUDIT-RUNTIME: [0-9]+/[0-9]+' "$test_wait"
sleep 1
# musl-built checks: run if the image has them (REQUIRE_MUSL=1 makes their
# absence a failure). The prompt is the kernel shell (vsh), which has no
# conditionals, so a missing binary just never prints a result.
printf '/bin/musl_runtime_test\r' >&3
wait_for 'MUSL-RUNTIME: [0-9]+/[0-9]+|vsh: command not found' "$musl_wait"
sleep 1
# A program started with arguments gets its own name as argv[0] and the
# arguments after it (the kernel shell used to pass the arguments alone).
if grep -aqE 'MUSL-RUNTIME: [0-9]+/[0-9]+' "$log"; then
    printf '/bin/musl_runtime_test --only %s\r' "$argv_test" >&3
    wait_for "MUSL-ONLY $argv_test: " "$test_wait"
    sleep 1
fi
finish

status=PASS
busybox="$(grep -aoE 'BUSYBOX_ALL_PASS|BUSYBOX_[A-Z_]*FAIL[A-Z_]*' "$log" | tail -1)"
[[ $busybox == BUSYBOX_ALL_PASS ]] || status=FAIL
audit="$(grep -aoE 'AUDIT-RUNTIME: [0-9]+/[0-9]+' "$log" | tail -1)"
if [[ $audit =~ ([0-9]+)/([0-9]+) ]]; then
    [[ ${BASH_REMATCH[1]} == "${BASH_REMATCH[2]}" ]] || status=FAIL
else
    status=FAIL
fi
musl="$(grep -aoE 'MUSL-RUNTIME: [0-9]+/[0-9]+' "$log" | tail -1)"
if [[ $musl =~ ([0-9]+)/([0-9]+) ]]; then
    [[ ${BASH_REMATCH[1]} == "${BASH_REMATCH[2]}" ]] || status=FAIL
else
    musl="MUSL-RUNTIME: absent"
    [[ ${REQUIRE_MUSL:-0} == 1 ]] && status=FAIL
fi
if [[ $musl != "MUSL-RUNTIME: absent" ]]; then
    only="$(grep -aoE "MUSL-ONLY $argv_test: [0-9]+/[0-9]+" "$log" | tail -1)"
    if [[ $only =~ ([0-9]+)/([0-9]+)$ ]] && [[ ${BASH_REMATCH[1]} == "${BASH_REMATCH[2]}" ]]; then
        musl="$musl; ARGV: ok"
    else
        musl="$musl; ARGV: ${only:-no result for /bin/musl_runtime_test --only $argv_test}"
        status=FAIL
    fi
fi
grep -aqE 'KERNEL PANIC|panicked at' "$log" && status=FAIL

echo "$status rootfs: ${busybox:-no BusyBox result}; ${audit:-no AUDIT-RUNTIME result}; ${musl:-no MUSL-RUNTIME result}"
if [[ $status != PASS ]]; then
    grep -aE 'FAIL|KERNEL PANIC|panicked at' "$log" | head -30
    echo "log: $log"
    exit 1
fi

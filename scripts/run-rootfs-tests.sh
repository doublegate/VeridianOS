#!/usr/bin/env bash
# Run the user-space runtime suite on x86_64: boot the dev kernel with the
# BusyBox BlockFS rootfs, let the boot-time BusyBox self-test run
# (BUSYBOX_ALL_PASS), then run /bin/audit_runtime_test at the shell prompt
# and require every check to pass ("AUDIT-RUNTIME: N/N").
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
# Env:   REQUIRE_MUSL=1 (musl_runtime_test must be present),
#        LOG_DIR (default target/boot-logs), ROOTFS (default
#        target/rootfs-blockfs.img), TEST_WAIT (seconds for the suite, 120),
#        SMP (CPU count, default 1)
set -u

root="$(cd "$(dirname "$0")/.." && pwd)"
prompt_wait="${1:-240}"
test_wait="${TEST_WAIT:-120}"
log_dir="${LOG_DIR:-$root/target/boot-logs}"
rootfs="${ROOTFS:-$root/target/rootfs-blockfs.img}"
img="$root/target/x86_64-veridian/debug/veridian-uefi.img"
mkdir -p "$log_dir"
log="$log_dir/rootfs-tests.log"
fifo="$log_dir/rootfs-serial.fifo"

ovmf=""
for f in /usr/share/edk2/x64/OVMF.4m.fd /usr/share/OVMF/OVMF_CODE_4M.fd \
    /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2/ovmf/OVMF_CODE.fd /usr/share/qemu/OVMF.fd; do
    [[ -r $f ]] && { ovmf="$f"; break; }
done
[[ -n $ovmf ]] || { echo "FAIL: no OVMF firmware found"; exit 2; }
for f in "$img" "$rootfs"; do
    [[ -r $f ]] || { echo "FAIL: $f not built"; exit 2; }
done

accel=(-cpu qemu64)
[[ -w /dev/kvm ]] && accel=(-enable-kvm -cpu host)

rm -f "$fifo"
mkfifo "$fifo"
qemu-system-x86_64 "${accel[@]}" -smp "${SMP:-1}" \
    -drive "if=pflash,format=raw,readonly=on,file=$ovmf" \
    -drive "id=disk0,if=none,format=raw,file=$img,snapshot=on" \
    -device ide-hd,drive=disk0 \
    -drive "file=$rootfs,if=none,id=vd0,format=raw,snapshot=on" \
    -device virtio-blk-pci,drive=vd0 \
    -serial stdio -display none -m 2048M <"$fifo" >"$log" 2>&1 &
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
printf '/bin/audit_runtime_test\r' >&3
wait_for 'AUDIT-RUNTIME: [0-9]+/[0-9]+' "$test_wait"
sleep 1
# musl-built checks: run if the image has them (REQUIRE_MUSL=1 makes their
# absence a failure). The prompt is the kernel shell (vsh), which has no
# conditionals, so a missing binary just never prints a result.
printf '/bin/musl_runtime_test\r' >&3
wait_for 'MUSL-RUNTIME: [0-9]+/[0-9]+|vsh: command not found' 60
sleep 1
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
grep -aqE 'KERNEL PANIC|panicked at' "$log" && status=FAIL

echo "$status rootfs: ${busybox:-no BusyBox result}; ${audit:-no AUDIT-RUNTIME result}; ${musl:-no MUSL-RUNTIME result}"
if [[ $status != PASS ]]; then
    grep -aE 'FAIL|KERNEL PANIC|panicked at' "$log" | head -30
    echo "log: $log"
    exit 1
fi

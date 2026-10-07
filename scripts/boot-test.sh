#!/usr/bin/env bash
# Boot one architecture's dev kernel in QEMU and check the in-kernel tests.
#
# Passes when the serial log shows BOOTOK, the boot-test tally is complete
# ("Results: N/N passed"), and nothing panicked during a settle period after
# BOOTOK (later boot stages run after the marker).
#
# QEMU runs in the background and is killed when done; it is never wrapped
# in `timeout` (QEMU 10 misbehaves under it).
#
# Usage: scripts/boot-test.sh <x86_64|aarch64|riscv64> [max-seconds]
# Env:   SETTLE (default 20), LOG_DIR (default target/boot-logs),
#        QEMU_EXTRA (extra QEMU arguments), SMP (CPU count, default 1)
set -u

arch="${1:?usage: boot-test.sh <x86_64|aarch64|riscv64> [max-seconds]}"
max="${2:-120}"
settle="${SETTLE:-20}"
root="$(cd "$(dirname "$0")/.." && pwd)"
log_dir="${LOG_DIR:-$root/target/boot-logs}"
mkdir -p "$log_dir"
log="$log_dir/boot-$arch.log"
smp="${SMP:-1}"
read -r -a extra <<<"${QEMU_EXTRA:-}"

find_ovmf() {
    local f
    for f in /usr/share/edk2/x64/OVMF.4m.fd /usr/share/OVMF/OVMF_CODE_4M.fd \
        /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2/ovmf/OVMF_CODE.fd \
        /usr/share/qemu/OVMF.fd; do
        [[ -r $f ]] && { echo "$f"; return 0; }
    done
    return 1
}

case "$arch" in
x86_64)
    ovmf="$(find_ovmf)" || { echo "FAIL x86_64: no OVMF firmware found"; exit 2; }
    img="$root/target/x86_64-veridian/debug/veridian-uefi.img"
    accel=(-cpu qemu64)
    [[ -w /dev/kvm ]] && accel=(-enable-kvm -cpu host)
    # shellcheck disable=SC2054  # commas belong to QEMU option values
    cmd=(qemu-system-x86_64 "${accel[@]}" -smp "$smp"
        -drive "if=pflash,format=raw,readonly=on,file=$ovmf"
        -drive "id=disk0,if=none,format=raw,file=$img,snapshot=on"
        -device ide-hd,drive=disk0 -m 2048M)
    ;;
aarch64)
    img="$root/target/aarch64-unknown-none/debug/veridian-kernel"
    cmd=(qemu-system-aarch64 -M virt -cpu cortex-a72 -smp "$smp" -m 256M -kernel "$img")
    ;;
riscv64)
    img="$root/target/riscv64gc-unknown-none-elf/debug/veridian-kernel"
    cmd=(qemu-system-riscv64 -M virt -smp "$smp" -m 256M -bios default -kernel "$img")
    ;;
*)
    echo "unknown architecture: $arch"
    exit 2
    ;;
esac
[[ -r $img ]] || { echo "FAIL $arch: $img not built (./build-kernel.sh $arch dev)"; exit 2; }

"${cmd[@]}" -serial stdio -display none "${extra[@]}" </dev/null >"$log" 2>&1 &
pid=$!
waited=0
while ((waited < max)); do
    sleep 1
    waited=$((waited + 1))
    kill -0 "$pid" 2>/dev/null || break
    if grep -aq BOOTOK "$log"; then
        sleep "$settle"
        break
    fi
done
kill "$pid" 2>/dev/null
wait "$pid" 2>/dev/null

status=PASS
grep -aq BOOTOK "$log" || status=FAIL
tally="$(grep -aoE 'Results: [0-9]+/[0-9]+ passed' "$log" | tail -1)"
if [[ $tally =~ Results:\ ([0-9]+)/([0-9]+) ]]; then
    [[ ${BASH_REMATCH[1]} == "${BASH_REMATCH[2]}" ]] || status=FAIL
else
    status=FAIL
fi
grep -aqE 'KERNEL PANIC|panicked at|BOOTFAIL' "$log" && status=FAIL

echo "$status $arch: BOOTOK=$(grep -ac BOOTOK "$log") after ${waited}s; ${tally:-no test tally}"
if [[ $status != PASS ]]; then
    grep -aE -A3 'KERNEL PANIC|panicked at|BOOTFAIL|\[FAIL' "$log" | head -20
    echo "log: $log"
    exit 1
fi

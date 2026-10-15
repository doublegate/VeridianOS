#!/usr/bin/env bash
# Run the x86_64 runtime suite under QEMU with an HMP monitor socket and
# leave QEMU running afterwards, so a stall can be inspected. The serial log
# is $DIR/serial.log, the monitor $DIR/mon.sock, QEMU's pid $DIR/qemu.pid.
#
#   socat - UNIX-CONNECT:$DIR/mon.sock     then e.g.: info registers
#   (map RIP with: addr2line -fe target/x86_64-veridian/debug/veridian-kernel <rip>)
#
# Stop QEMU yourself when done (`kill "$(cat $DIR/qemu.pid)"`; or
# `pkill -9 -f qemu-system` run on its own -- it exits 1 when nothing matches).
#
# Usage: scripts/dev/rootfs-monitor.sh   Env: DIR (default target/dev-logs/monitor),
#        OVMF (firmware, auto-detected), MEM (default 2048M), SMP (default 1)
set -u
root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)" || exit 2
cd "$root" || exit 2
dir="${DIR:-target/dev-logs/monitor}"
mkdir -p "$dir"
log="$dir/serial.log" fifo="$dir/serial.fifo" mon="$dir/mon.sock"
ovmf="${OVMF:-}"
for f in /usr/share/edk2/x64/OVMF.4m.fd /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd; do
    [ -z "$ovmf" ] && [ -r "$f" ] && ovmf="$f"
done
[ -n "$ovmf" ] || { echo "no OVMF firmware found (set OVMF=)"; exit 2; }
rm -f "$fifo" "$mon"
mkfifo "$fifo"
qemu-system-x86_64 -enable-kvm -cpu host -smp "${SMP:-1}" \
    -drive "if=pflash,format=raw,readonly=on,file=$ovmf" \
    -drive "id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img,snapshot=on" \
    -device ide-hd,drive=disk0 \
    -drive "file=target/rootfs-blockfs.img,if=none,id=vd0,format=raw,snapshot=on" \
    -device virtio-blk-pci,drive=vd0 \
    -monitor "unix:$mon,server,nowait" \
    -serial stdio -display none -m "${MEM:-2048M}" <"$fifo" >"$log" 2>&1 &
pid=$!
echo "$pid" >"$dir/qemu.pid"
exec 3>"$fifo"
wait_for() { # pattern seconds
    local w=0
    until grep -aqE "$1" "$log"; do
        [ "$w" -ge "$2" ] && return 1
        kill -0 "$pid" 2>/dev/null || return 1
        sleep 1
        w=$((w + 1))
    done
}
wait_for 'root@veridian' 300 || echo "no shell prompt"
sleep 2
printf '/bin/audit_runtime_test\r' >&3
wait_for 'AUDIT-RUNTIME: [0-9]+/[0-9]+' 180
sleep 1
printf '/bin/musl_runtime_test\r' >&3
if wait_for 'MUSL-RUNTIME: [0-9]+/[0-9]+' 600; then echo "suite finished"; else echo "suite did not finish: inspect $mon"; fi
grep -aoE 'AUDIT-RUNTIME: [0-9]+/[0-9]+|MUSL-RUNTIME: [0-9]+/[0-9]+' "$log"
# Keep the serial input open while QEMU lives.
wait "$pid"

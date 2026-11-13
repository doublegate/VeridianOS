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
#        count, default 1), RISCV_CPU (QEMU -cpu for RISC-V, default QEMU's;
#        `rv64,sv48=off` leaves Sv39 as the only paging mode, ADR 0018),
#        ONLY ("name ..." of musl_runtime_test checks: run just those, each
#        REPEAT times, default 1, and skip the rest of the suite)
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
        [[ -n ${RISCV_CPU:-} ]] && qemu+=(-cpu "$RISCV_CPU")
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
# A monitor socket, for the CPU dump of a stalled run (dump_cpus).
mon="$log_dir/qemu-monitor$suffix.sock"
rm -f "$mon"
"${qemu[@]}" -smp "${SMP:-1}" -serial stdio -display none -m 2048M \
    -monitor "unix:$mon,server,nowait" <"$fifo" >"$log" 2>&1 &
pid=$!
exec 3>"$fifo" # keep the write end open for the whole session

finish() {
    exec 3>&-
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    rm -f "$fifo" "$mon"
}

# A run that stopped making progress: every CPU's registers, twice a
# second apart (a spinning CPU moves little, a halted one not at all),
# with each RIP/PC resolved against the kernel ELF, into
# stall-cpus<suffix>.txt beside the log (CI uploads the directory). For a
# hang seen only in CI (KNOWN-LIMITATIONS, the four-CPU pid_ns hang).
dump_cpus() {
    [[ -S $mon ]] || return 0
    local elf="$img"
    [[ $arch == x86_64 ]] && elf="$root/target/x86_64-veridian/debug/veridian-kernel"
    python3 - "$mon" "$elf" "$log_dir/stall-cpus$suffix.txt" <<'PY' || true
import re, socket, subprocess, sys, time
mon, elf, out = sys.argv[1:4]
ansi = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
def ask(*cmds):
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(5)
    s.connect(mon)
    for cmd in cmds:
        s.sendall((cmd + "\n").encode())
        time.sleep(0.3)
    time.sleep(1)
    data = b""
    try:
        while True:
            chunk = s.recv(65536)
            if not chunk:
                break
            data += chunk
    except socket.timeout:
        pass
    s.close()
    # The monitor echoes each typed character with cursor moves: keep
    # only its answers.
    text = ansi.sub("", data.decode(errors="replace"))
    return "\n".join(l for l in text.splitlines()
                     if not l.startswith(("(qemu)", "QEMU ")))
def where(addrs):
    if not addrs:
        return []
    try:
        res = subprocess.run(["addr2line", "-Cfipe", elf] + ["0x" + a for a in addrs],
                             capture_output=True, text=True, timeout=20).stdout
        return res.strip().splitlines()
    except (OSError, subprocess.SubprocessError):
        return ["?"]
# A kernel text address: the upper half (x86_64 0xffffffff8..., AArch64
# and RISC-V kernels linked high as well).
text_addr = re.compile(r"0x(ffffffff[0-9a-f]{8})\b")
with open(out, "w") as f:
    for round_ in (1, 2):
        regs = ask("info registers -a")
        f.write(f"=== sample {round_}\n{regs}\n")
        # x86_64 and AArch64 print "RIP=..." / "PC=...", RISC-V " pc  ..."
        # and "x2/sp ..." with no "=" (PR #42 review).
        for pc in re.findall(r"\b(?:RIP|PC|pc)(?:=|\s+)([0-9a-fA-F]{8,16})\b", regs):
            f.write(f"{pc}: {' | '.join(where([pc]))}\n")
        # A rough backtrace per CPU: the kernel text addresses among the
        # first 64 words of its stack, most recent first.
        for n, block in enumerate(re.split(r"\nCPU#\d+", "\n" + regs)[1:]):
            m = re.search(r"(?:\bRSP=|\bSP=|\bx2/sp\s+)([0-9a-fA-F]{16})\b", block)
            if not m:
                continue
            words = ask(f"cpu {n}", f"x /64gx 0x{m.group(1)}")
            seen = []
            for line in words.splitlines():
                # "addr: 0x... 0x...": only the values, not the address column.
                for a in text_addr.findall(line.split(":", 1)[-1]):
                    if a not in seen:
                        seen.append(a)
            f.write(f"--- CPU#{n} stack (sp {m.group(1)}): kernel code addresses\n")
            shown = 0
            for a in seen:
                w = where([a])
                # A data pointer into the kernel image resolves to nothing.
                if not w or w[0].startswith("??"):
                    continue
                f.write(f"  {a}: {' | '.join(w)}\n")
                shown += 1
                if shown == 24:
                    break
        time.sleep(1)
PY
    echo "stalled: CPU state in $log_dir/stall-cpus$suffix.txt"
}

count_in_log() { # pattern: matching lines in the log so far (0 before it exists)
    local n
    n=$(grep -acE "$1" "$log" 2>/dev/null)
    echo "${n:-0}"
}

wait_for() { # pattern seconds [matches already seen, default 0]
    local waited=0
    until (($(count_in_log "$1") > ${3:-0})); do
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
# ONLY="name ...": run just those musl_runtime_test checks (REPEAT times
# each, default 1) and stop; for a change confined to them, instead of the
# whole suite.
if [[ -n ${ONLY:-} ]]; then
    status=PASS
    for t in $ONLY; do
        # The same name may come twice: wait for a result line past the
        # ones already in the log (PR #42 review).
        seen=$(count_in_log "MUSL-ONLY $t: ")
        printf '/bin/musl_runtime_test --only %s %s\r' "$t" "${REPEAT:-1}" >&3
        wait_for "MUSL-ONLY $t: " "$musl_wait" "$seen" || dump_cpus
        sleep 1
        r="$(grep -aoE "MUSL-ONLY $t: [0-9]+/[0-9]+" "$log" | tail -1)"
        if [[ $r =~ ([0-9]+)/([0-9]+)$ ]] && [[ ${BASH_REMATCH[1]} == "${BASH_REMATCH[2]}" ]]; then
            echo "PASS $r"
        else
            echo "FAIL ${r:-MUSL-ONLY $t: no result}"
            status=FAIL
        fi
    done
    finish
    grep -aqE 'KERNEL PANIC|panicked at' "$log" && status=FAIL
    if [[ $status != PASS ]]; then
        grep -aE '^FAIL|KERNEL PANIC|panicked at' "$log" | head -30
        echo "log: $log"
        exit 1
    fi
    exit 0
fi
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
wait_for 'MUSL-RUNTIME: [0-9]+/[0-9]+|vsh: command not found' "$musl_wait" || dump_cpus
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
# On several CPUs a kernel message can land inside the marker's line
# ("BUSYBO[PROCESS] ...\nX_ALL_PASS"): busybox_test.sh's summary line just
# before it says the same, so a whole run with no failure counts too.
summary="$(grep -aoE '^=== Results: [0-9]+/[0-9]+ passed, 0 failed ===' "$log" | tail -1)"
if [[ -z $busybox && $summary =~ ([0-9]+)/([0-9]+) && ${BASH_REMATCH[1]} == "${BASH_REMATCH[2]}" ]]; then
    busybox=BUSYBOX_ALL_PASS
fi
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

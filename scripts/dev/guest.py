#!/usr/bin/env python3
"""Boot the AArch64 or RISC-V musl root filesystem and run commands at its shell.

Usage: scripts/dev/guest.py ARCH LOG CMD...

Boots the dev kernel of ARCH (aarch64 or riscv64) with target/rootfs-musl-ARCH.img
(built by scripts/build-musl-rootfs.sh; opened with snapshot=on, so it is not
changed), waits for the root prompt, then types each CMD and waits for the next
prompt. The whole serial output goes to LOG. Prints "ok" when every command
returned to the prompt, otherwise "timeout/exit", and exits 0 or 1 to match.

A command has finished when a root prompt, in any working directory, appears in
the output after the shell has echoed the whole command. A command that itself
prints a root prompt ends its wait early.

Environment: SMP (CPU count, default 1), RISCV_CPU (QEMU -cpu for RISC-V),
PROMPT_TIMEOUT (seconds to wait for each prompt, default 600).
"""

import os
import re
import subprocess
import sys
import time
from pathlib import Path

# The kernel shell's root prompt: "root@veridian:" + working directory + "#".
PROMPT = re.compile(rb"root@veridian:[^\s#]*#")
ROOT = Path(__file__).resolve().parents[2]


def qemu_command(arch: str) -> list[str]:
    if arch == "aarch64":
        cmd = [
            "qemu-system-aarch64",
            "-M",
            "virt",
            "-cpu",
            "cortex-a72",
            "-kernel",
            str(ROOT / "target/aarch64-unknown-none-softfloat/debug/veridian-kernel"),
        ]
    elif arch == "riscv64":
        cmd = [
            "qemu-system-riscv64",
            "-M",
            "virt",
            "-bios",
            "default",
            "-kernel",
            str(ROOT / "target/riscv64imac-unknown-none-elf/debug/veridian-kernel"),
        ]
        if os.environ.get("RISCV_CPU"):
            cmd += ["-cpu", os.environ["RISCV_CPU"]]
    else:
        sys.exit(f"unknown architecture {arch!r} (aarch64 or riscv64)")
    kernel = Path(cmd[cmd.index("-kernel") + 1])
    if not kernel.exists():
        sys.exit(f"{kernel} not found; build it with ./build-kernel.sh {arch} dev")
    image = ROOT / f"target/rootfs-musl-{arch}.img"
    if not image.exists():
        sys.exit(
            f"{image} not found; build it with scripts/build-musl-rootfs.sh {arch}"
        )
    return cmd + [
        "-device",
        "ramfb",
        "-drive",
        f"file={image},if=none,id=vd0,format=raw,snapshot=on",
        "-device",
        "virtio-blk-device,drive=vd0",
        "-smp",
        os.environ.get("SMP", "1"),
        "-serial",
        "stdio",
        "-display",
        "none",
        "-m",
        "2048M",
    ]


def main() -> int:
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    arch, log, cmds = sys.argv[1], sys.argv[2], sys.argv[3:]
    timeout = int(os.environ.get("PROMPT_TIMEOUT", "600"))
    # Checked before LOG is truncated.
    command = qemu_command(arch)
    Path(log).parent.mkdir(parents=True, exist_ok=True)
    ok = False
    with open(log, "wb") as out, open(log, "rb") as reader:
        proc = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=out,
            stderr=subprocess.STDOUT,
        )
        data = bytearray()

        def wait(start: int, echo: bytes) -> int | None:
            """The end of the first prompt written after `start` and after
            the whole of `echo`, or None on timeout or exit."""
            began = time.time()
            while time.time() - began < timeout:
                data.extend(reader.read())
                at = start
                if echo:
                    seen = data.find(echo, start)
                    at = seen + len(echo) if seen >= 0 else -1
                match = PROMPT.search(data, at) if at >= 0 else None
                if match:
                    return match.end()
                if proc.poll() is not None:
                    return None
                time.sleep(1)
            return None

        try:
            end = wait(0, b"")
            for cmd in cmds:
                if end is None:
                    break
                assert proc.stdin is not None
                try:
                    proc.stdin.write(cmd.encode() + b"\n")
                    proc.stdin.flush()
                except BrokenPipeError:
                    end = None
                    break
                end = wait(end, cmd.encode())
            ok = end is not None
        finally:
            proc.kill()
            proc.wait()
    print("ok" if ok else "timeout/exit")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

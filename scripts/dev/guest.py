#!/usr/bin/env python3
"""Boot the AArch64 or RISC-V musl root filesystem and run commands at its shell.

Usage: scripts/dev/guest.py ARCH LOG CMD...

Boots the dev kernel of ARCH (aarch64 or riscv64) with target/rootfs-musl-ARCH.img
(built by scripts/build-musl-rootfs.sh; opened with snapshot=on, so it is not
changed), waits for the root prompt, then types each CMD and waits for the next
prompt. The whole serial output goes to LOG. Prints "ok" when every command
returned to the prompt, otherwise "timeout/exit", and exits 0 or 1 to match.

Environment: SMP (CPU count, default 1), RISCV_CPU (QEMU -cpu for RISC-V),
PROMPT_TIMEOUT (seconds to wait for each prompt, default 600).
"""

import os
import subprocess
import sys
import time
from pathlib import Path

PROMPT = b"root@veridian:/#"
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
    with open(log, "wb") as out:
        proc = subprocess.Popen(
            qemu_command(arch),
            stdin=subprocess.PIPE,
            stdout=out,
            stderr=subprocess.STDOUT,
        )

        def prompts() -> int:
            with open(log, "rb") as f:
                return f.read().count(PROMPT)

        def wait(count: int) -> bool:
            start = time.time()
            while time.time() - start < timeout:
                if prompts() >= count:
                    return True
                if proc.poll() is not None:
                    return False
                time.sleep(1)
            return False

        expected = 1
        ok = wait(expected)
        for cmd in cmds:
            if not ok:
                break
            assert proc.stdin is not None
            proc.stdin.write(cmd.encode() + b"\n")
            proc.stdin.flush()
            expected += 1
            ok = wait(expected)
        proc.kill()
        proc.wait()
    print("ok" if ok else "timeout/exit")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

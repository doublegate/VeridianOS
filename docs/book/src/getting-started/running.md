# Running in QEMU

VeridianOS can be run in QEMU on all three supported architectures. QEMU 10.2+ is recommended.

## x86_64 (UEFI boot)

x86_64 uses UEFI boot via the bootloader crate. It **cannot** use the `-kernel` flag directly.

```bash
# Build first
./build-kernel.sh x86_64 dev

# Run (serial only, ALWAYS use -enable-kvm on x86_64 hosts)
qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -serial stdio -display none -m 256M
```

### With BlockFS rootfs (KDE binaries)

Requires 2GB RAM for the full rootfs with KDE Plasma 6 binaries:

```bash
qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -drive file=target/rootfs-blockfs.img,if=none,id=vd0,format=raw \
    -device virtio-blk-pci,drive=vd0 \
    -serial stdio -display none -m 2G
```

## AArch64 (direct kernel boot)

```bash
./build-kernel.sh aarch64 dev
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \
    -kernel target/aarch64-unknown-none/debug/veridian-kernel \
    -serial stdio -display none
```

## RISC-V 64 (OpenSBI + kernel)

```bash
./build-kernel.sh riscv64 dev
qemu-system-riscv64 -M virt -m 256M -bios default \
    -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel \
    -serial stdio -display none
```

## Quick Reference

| Arch | Boot | Firmware | Image | KVM |
|------|------|----------|-------|-----|
| x86_64 | UEFI disk | OVMF.4m.fd | `target/x86_64-veridian/debug/veridian-uefi.img` | Required |
| AArch64 | Direct `-kernel` | None | `target/aarch64-unknown-none/debug/veridian-kernel` | N/A |
| RISC-V | `-kernel` + `-bios default` | OpenSBI | `target/riscv64gc-unknown-none-elf/debug/veridian-kernel` | N/A |

## Expected Output

All 3 architectures boot to Stage 6 BOOTOK with 29/29 tests passing. x86_64 shows Ring 3 user-space entry and a `root@veridian:/#` shell prompt.

## Debugging with GDB

Add `-s -S` to any QEMU command to enable GDB debugging (server on port 1234, start paused):

```bash
# In another terminal:
gdb-multiarch target/x86_64-veridian/debug/veridian-kernel
(gdb) target remote :1234
(gdb) continue
```

See [docs/GDB-DEBUGGING.md](https://github.com/doublegate/VeridianOS/blob/main/docs/GDB-DEBUGGING.md) for detailed debugging instructions.

## QEMU Pitfalls

- **Do NOT** use `timeout` to wrap QEMU -- causes "drive exists" errors
- **Do NOT** use `-kernel` for x86_64 -- fails with "PVH ELF Note" error
- **Do NOT** use `-bios` instead of `-drive if=pflash` -- different semantics
- **ALWAYS** use `-enable-kvm` for x86_64 (TCG is ~100x slower)
- **ALWAYS** kill any existing QEMU before re-running

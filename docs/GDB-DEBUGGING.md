# GDB Debugging Guide for VeridianOS

This guide explains how to debug the VeridianOS kernel using GDB with QEMU.

## Quick Start

We provide architecture-specific debug scripts that handle all the setup:

```bash
# For x86_64
./scripts/debug-x86_64.sh

# For AArch64
./scripts/debug-aarch64.sh

# For RISC-V 64
./scripts/debug-riscv64.sh
```

These scripts will:
1. Build the dev kernel with `./build-kernel.sh <arch> dev` (x86_64 also gets its UEFI disk image)
2. Start QEMU paused with a GDB server on port 1234, the guest's serial output going to
   `target/debug-<arch>-serial.log`
3. Launch GDB with the architecture's script from `scripts/gdb/`, which connects, loads the kernel
   symbols and sets the boot and panic breakpoints
4. Stop QEMU when you exit GDB

GDB reads its commands from standard input, so a session can also be scripted:
`./scripts/debug-aarch64.sh < commands.gdb`. `just debug-<arch>` runs the same scripts.

## Manual Debugging

### 1. Build with Debug Symbols

Dev builds carry the symbols:

```bash
./build-kernel.sh x86_64 dev     # target/x86_64-veridian/debug/veridian-kernel (+ veridian-uefi.img)
./build-kernel.sh aarch64 dev    # target/aarch64-unknown-none-softfloat/debug/veridian-kernel
./build-kernel.sh riscv64 dev    # target/riscv64imac-unknown-none-elf/debug/veridian-kernel
```

### 2. Start QEMU with GDB Server

Add `-s` (GDB server on port 1234) and `-S` (pause at start) to the usual command lines:

```bash
# x86_64 (UEFI boot; -kernel does not work for x86_64). The OVMF path below is Arch's; it varies
# by distribution (Debian/Ubuntu: /usr/share/OVMF/OVMF_CODE_4M.fd; Fedora:
# /usr/share/edk2/ovmf/OVMF_CODE.fd);
# without a writable /dev/kvm, replace -enable-kvm with -cpu qemu64.
qemu-system-x86_64 -enable-kvm -m 256M \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -serial stdio -display none \
    -s -S

# AArch64
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \
    -kernel target/aarch64-unknown-none-softfloat/debug/veridian-kernel \
    -serial stdio -display none \
    -s -S

# RISC-V
qemu-system-riscv64 -M virt -m 256M -bios default \
    -kernel target/riscv64imac-unknown-none-elf/debug/veridian-kernel \
    -serial stdio -display none \
    -s -S
```

### 3. Connect GDB

In another terminal, from the repository root (the scripts use relative paths):

```bash
# For x86_64
gdb -x scripts/gdb/x86_64.gdb

# For AArch64 (use gdb-multiarch if your gdb is single-architecture)
gdb-multiarch -x scripts/gdb/aarch64.gdb

# For RISC-V (use gdb-multiarch if your gdb is single-architecture)
gdb-multiarch -x scripts/gdb/riscv64.gdb
```

## Common GDB Commands

### Basic Control
- `continue` (or `c`) - Start/continue execution
- `break <symbol>` - Set breakpoint at symbol
- `break *<address>` - Set breakpoint at address
- `next` (or `n`) - Step over
- `step` (or `s`) - Step into
- `finish` - Run until current function returns
- `backtrace` (or `bt`) - Show call stack

### Examining State
- `info registers` - Show all registers
- `info registers <reg>` - Show specific register
- `x/<n><f> <addr>` - Examine memory
  - `n` = count (e.g., 16)
  - `f` = format (x=hex, i=instruction, s=string)
  - Examples:
    - `x/16xw 0x1000` - 16 words in hex at 0x1000
    - `x/10i $pc` - 10 instructions at PC

### Custom Commands

Our GDB scripts provide custom commands for kernel debugging:

#### Common Commands (all architectures)
- `kernel-symbols <target-dir> [release]` - Load kernel symbols from
  `target/<target-dir>/debug/veridian-kernel`, or the release build with `release`
- `break-panic` - Set breakpoint on `core::panicking::panic_fmt` (every panic goes through it)
- `hbreak-panic` - The same with a hardware breakpoint (x86_64 under UEFI)
- `break-main` - Set breakpoint on `veridian_kernel::bootstrap::kernel_init`
- `break-boot-x86_64`, `break-boot-aarch64`, `break-boot-riscv64` - Boot entry breakpoints
- `examine-stack` - Display stack contents at `$sp`
- `examine-uart-aarch64`, `examine-uart-riscv64` - Display UART registers (x86_64's COM1 is an
  I/O port, which `x` cannot read)
- `kernel-state` - Display overall kernel state

GDB cannot compare string arguments on a bare-metal target (a string literal needs `malloc` in the
inferior), so the architecture-specific helpers are separate commands rather than one command that
takes the architecture name.

#### x86_64 Specific
- `dump-gdt` - Display Global Descriptor Table
- `dump-idt` - Display Interrupt Descriptor Table
- `dump-cr` - Display control registers
- `dump-vga` - Display VGA text buffer
- `walk-page-table <addr>` - Walk x86_64 page tables

#### AArch64 Specific
- `dump-regs` - Display all general purpose registers
- `dump-system-regs` - Display system registers
- `dump-uart-pl011` - Display PL011 UART registers
- `examine-boot-area` - Examine boot memory area
- `walk-page-table-aa64 <addr>` - Walk AArch64 page tables

#### RISC-V Specific
- `dump-regs` - Display all general purpose registers
- `dump-csr` - Display Control and Status Registers
- `dump-uart-8250` - Display 8250 UART registers
- `examine-opensbi` - Examine OpenSBI region
- `walk-page-table-sv48 <addr>` - Walk RISC-V Sv48 page tables (QEMU `virt` default)
- `walk-page-table-sv39 <addr>` - Walk RISC-V Sv39 page tables (Sv39-only harts)
- `analyze-trap` - Analyze trap cause

### Aliases

Short aliases are available for common commands:
- `ks` = kernel-symbols
- `bp` = break-panic
- `bm` = break-main
- `bb` = break-boot (prints the per-architecture commands)
- `es` = examine-stack
- `eu` = examine-uart (prints the per-architecture commands)
- `kst` = kernel-state

Architecture-specific aliases vary by platform.

## Debugging Tips

### 1. Early Boot Issues
The scripts set the boot breakpoints already; by hand:
```gdb
# In GDB
break-boot-x86_64   # or break-boot-aarch64, break-boot-riscv64
continue
```

- **x86_64** boots through UEFI (under KVM when `/dev/kvm` is writable): the bootloader loads the
  kernel after GDB has attached and overwrites any software breakpoint set before then, so use
  `hbreak` (there are four hardware breakpoints; the script uses two).
- **AArch64 and RISC-V** start at their physical load address, before the MMU maps the kernel at
  its linked (high) address, so a breakpoint on `_start` does not fire. On RISC-V,
  `break-boot-riscv64` breaks at the physical entry (0x80200000, where OpenSBI jumps) and then at
  `_start_rust`. On AArch64 not even the physical entry (0x40200000) stops under QEMU's gdbstub,
  so the first stop is `_start_rust`.

### 2. Panic Debugging
To catch panics (the scripts set this already):
```gdb
break-panic      # AArch64, RISC-V
hbreak-panic     # x86_64: a software breakpoint set before UEFI loads the kernel is overwritten
continue
```

### 3. Memory Inspection
To examine specific memory regions:
```gdb
# Stack
examine-stack

# UART (AArch64, RISC-V)
examine-uart-aarch64

# Custom address
x/32xb 0x1000
```

### 4. AArch64 Specific Tips
```gdb
# Check the PL011 UART registers
x/16xb 0x09000000
```

### 5. Source-Level Debugging
GDB can show source code if symbols are loaded:
```gdb
# List source at current location
list

# List specific function
list veridian_kernel::bootstrap::kernel_init

# Set breakpoint by line
break main.rs:42
```

## Troubleshooting

### GDB Can't Connect
- Ensure QEMU is running with `-s -S` flags
- Check that port 1234 is not in use
- Try `target remote :1234` manually in GDB

### No Symbols
- Ensure you built with debug profile (not release)
- Check that `kernel-symbols` command succeeded
- Verify path to kernel binary is correct

### Wrong Architecture
- Use `gdb-multiarch` for cross-architecture debugging
- Ensure correct GDB script is loaded for your target
- Check `set architecture` command in GDB

### Breakpoints Not Hit
- Verify address with `info breakpoints`
- Check that code is actually executed
- For early boot, use hardware breakpoints: `hbreak *0x1000`

## Advanced Usage

### Real Hardware
None of the architectures has run on real hardware yet (see `docs/KNOWN-LIMITATIONS.md`). For
x86_64 the kernel contains a GDB remote-protocol stub on COM2 (`kernel/src/debug/gdb_stub.rs`);
it is not enabled in a default build.

### Debugging with VS Code
VS Code can be configured to use GDB. Create `.vscode/launch.json`:
```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "name": "Debug Kernel (x86_64)",
            "type": "cppdbg",
            "request": "launch",
            "program": "${workspaceFolder}/target/x86_64-veridian/debug/veridian-kernel",
            "miDebuggerServerAddress": "localhost:1234",
            "miDebuggerPath": "gdb",
            "setupCommands": [
                {
                    "text": "source ${workspaceFolder}/scripts/gdb/x86_64.gdb"
                }
            ]
        }
    ]
}
```

## References

- [GDB Documentation](https://sourceware.org/gdb/current/onlinedocs/gdb/)
- [QEMU GDB Usage](https://qemu-project.gitlab.io/qemu/system/gdb.html)
- [OSDev GDB Guide](https://wiki.osdev.org/Kernel_Debugging#Use_GDB_with_QEMU)
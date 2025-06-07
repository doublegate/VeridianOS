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
1. Build the kernel with debug symbols
2. Start QEMU with GDB server enabled (paused)
3. Launch GDB with the appropriate configuration
4. Clean up when you exit GDB

## Manual Debugging

### 1. Build with Debug Symbols

Always build with debug symbols for debugging:

```bash
# x86_64
cargo build --target targets/x86_64-veridian.json -p veridian-kernel \
    -Zbuild-std=core,compiler_builtins,alloc \
    -Zbuild-std-features=compiler-builtins-mem

# AArch64
cargo build --target targets/aarch64-veridian.json -p veridian-kernel \
    -Zbuild-std=core,compiler_builtins,alloc \
    -Zbuild-std-features=compiler-builtins-mem

# RISC-V
cargo build --target targets/riscv64gc-veridian.json -p veridian-kernel \
    -Zbuild-std=core,compiler_builtins,alloc \
    -Zbuild-std-features=compiler-builtins-mem
```

### 2. Start QEMU with GDB Server

Start QEMU with `-s` (GDB server on port 1234) and `-S` (pause at start):

```bash
# x86_64
qemu-system-x86_64 \
    -drive format=raw,file=target/x86_64-veridian/debug/bootimage-veridian-kernel.bin \
    -serial stdio \
    -display none \
    -s -S

# AArch64
qemu-system-aarch64 \
    -M virt \
    -cpu cortex-a53 \
    -nographic \
    -kernel target/aarch64-veridian/debug/veridian-kernel \
    -serial mon:stdio \
    -s -S

# RISC-V
qemu-system-riscv64 \
    -M virt \
    -nographic \
    -kernel target/riscv64gc-veridian/debug/veridian-kernel \
    -serial mon:stdio \
    -s -S
```

### 3. Connect GDB

In another terminal, start GDB:

```bash
# For x86_64
gdb -x scripts/gdb/x86_64.gdb

# For AArch64 (use gdb-multiarch if available)
gdb-multiarch -x scripts/gdb/aarch64.gdb

# For RISC-V (use gdb-multiarch if available)
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
- `kernel-symbols <arch>` - Load kernel symbols
- `break-panic` - Set breakpoint on panic handler
- `break-main` - Set breakpoint on kernel_main
- `break-boot <arch>` - Set architecture-specific boot breakpoints
- `examine-stack` - Display stack contents
- `examine-uart <arch>` - Display UART registers
- `kernel-state` - Display overall kernel state

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
- `walk-page-table-sv39 <addr>` - Walk RISC-V Sv39 page tables
- `analyze-trap` - Analyze trap cause

### Aliases

Short aliases are available for common commands:
- `ks` = kernel-symbols
- `bp` = break-panic
- `bm` = break-main
- `bb` = break-boot
- `es` = examine-stack
- `eu` = examine-uart
- `kst` = kernel-state

Architecture-specific aliases vary by platform.

## Debugging Tips

### 1. Early Boot Issues
For early boot debugging, set breakpoints at the entry point:
```gdb
# In GDB
break-boot x86_64  # or aarch64, riscv64
continue
```

### 2. Panic Debugging
To catch panics:
```gdb
break-panic
continue
```

### 3. Memory Inspection
To examine specific memory regions:
```gdb
# Stack
examine-stack

# UART (architecture specific)
examine-uart x86_64

# Custom address
x/32xb 0x1000
```

### 4. AArch64 Specific Tips
For AArch64, the kernel loads at 0x40080000:
```gdb
# Examine kernel load address
x/16i 0x40080000

# Check UART output buffer
x/16xb 0x09000000
```

### 5. Source-Level Debugging
GDB can show source code if symbols are loaded:
```gdb
# List source at current location
list

# List specific function
list kernel_main

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

### Remote Debugging
To debug on actual hardware (when supported):
```bash
# On target machine
gdbserver :1234 /path/to/kernel

# On development machine
gdb
target remote <target-ip>:1234
```

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
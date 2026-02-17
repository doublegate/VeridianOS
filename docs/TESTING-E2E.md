# VeridianOS End-to-End Testing Guide

This document describes how to cross-compile, deploy, and run C programs on VeridianOS for end-to-end validation of the userland toolchain.

## Overview

The E2E test infrastructure validates the complete path from C source code to running user-space process:

1. Cross-compilation using the VeridianOS GCC toolchain
2. Linking against the VeridianOS libc and CRT
3. Loading and executing the resulting ELF binary on the kernel
4. Verifying output via serial console

Two test programs are provided:

| Test | Description | Dependencies |
|------|-------------|--------------|
| `minimal` | Raw syscall test -- no libc, no CRT | Cross-compiler only |
| `hello` | Full libc test -- stdio, stdlib, string, unistd | Cross-compiler + libc.a + crt0.o |

## Prerequisites

### 1. Cross-Compiler Toolchain

The VeridianOS cross-compiler must be built and installed at `/opt/veridian/toolchain/` (or set `VERIDIAN_TOOLCHAIN_PREFIX`). The expected binaries:

```
/opt/veridian/toolchain/bin/x86_64-veridian-gcc
/opt/veridian/toolchain/bin/aarch64-veridian-gcc
/opt/veridian/toolchain/bin/riscv64-veridian-gcc
```

See `docs/CROSS-COMPILATION.md` for build instructions.

### 2. libc (for the `hello` test)

Build and install the VeridianOS libc into the sysroot:

```bash
cd userland/libc
make ARCH=x86_64 install     # or aarch64, riscv64
```

This produces `toolchain/sysroot/lib/<arch>/libc.a` and installs headers.

### 3. CRT startup files (for the `hello` test)

The `crt0.o` startup object must be assembled and installed:

```bash
# x86_64 example:
x86_64-veridian-gcc -c -o crt0.o toolchain/sysroot/crt/x86_64/crt0.S
mkdir -p toolchain/sysroot/lib/x86_64
cp crt0.o toolchain/sysroot/lib/x86_64/
```

### 4. Kernel (for QEMU execution)

```bash
./build-kernel.sh x86_64 dev    # or aarch64, riscv64
```

## Cross-Compiling Test Programs

### Using the script (recommended)

```bash
# Compile for x86_64 (default)
./scripts/cross-compile-test.sh

# Compile for AArch64
./scripts/cross-compile-test.sh --arch aarch64

# Compile with a custom toolchain path
./scripts/cross-compile-test.sh --toolchain /usr/local/veridian

# Compile and show QEMU run instructions
./scripts/cross-compile-test.sh --run
```

### Using the Makefile

```bash
cd userland/tests

# Build all tests for x86_64
make

# Build for a specific architecture
make ARCH=aarch64

# Build only the minimal test
make minimal

# Show ELF info
make info

# Clean
make clean
```

### Manual compilation

For the minimal test (no libc):

```bash
x86_64-veridian-gcc \
    -nostdlib -nostdinc -ffreestanding -static \
    -mno-red-zone -mcmodel=small \
    -Wall -Wextra -O2 -g \
    -o minimal userland/tests/minimal.c
```

For the hello test (with libc):

```bash
x86_64-veridian-gcc \
    -std=c11 -nostdinc -ffreestanding -static \
    -isystem userland/libc/include \
    -isystem toolchain/sysroot/include \
    -fno-stack-protector -fno-builtin \
    -mno-red-zone -mcmodel=small \
    -Wall -Wextra -O2 -g \
    -nostdlib -Ltoolchain/sysroot/lib/x86_64 \
    -o hello \
    toolchain/sysroot/lib/x86_64/crt0.o userland/tests/hello.c \
    -lc
```

## Test Programs

### `minimal.c` -- Raw Syscall Test

This is the simplest possible test: a freestanding binary that writes a message to stdout using raw `syscall`/`svc`/`ecall` instructions and then exits.

- **No libc required** -- uses inline assembly for syscalls
- **No CRT required** -- provides its own `_start` entry point
- **Multi-architecture** -- includes x86_64, AArch64, and RISC-V 64 syscall wrappers

Syscall numbers used (from `kernel/src/syscall/mod.rs`):

| Syscall | Number | Arguments |
|---------|--------|-----------|
| `SYS_FILE_WRITE` | 53 | fd, buf_ptr, count |
| `SYS_PROCESS_EXIT` | 11 | status |

**Expected output:**
```
MINIMAL_TEST_PASS
```

### `hello.c` -- Full libc Test

A comprehensive test that exercises the VeridianOS libc:

1. `write()` -- raw file descriptor write via syscall wrapper
2. `printf()` -- formatted output (tests vfprintf, fwrite, stdio buffering)
3. `argv`/`argc` -- command-line argument access (tests CRT stack setup)
4. `getpid()` -- process identity syscall
5. `strcpy()`/`strcat()` -- string operations
6. `malloc()`/`free()` -- heap allocation (tests sbrk/brk syscall)

**Expected output:**
```
Hello from VeridianOS userland!
argc = 1
argv[0] = /bin/hello
pid = <N>
VeridianOS works!
heap allocation OK
E2E_TEST_PASS
```

## Running on VeridianOS

### Current Status

The kernel currently boots to an interactive shell (vsh) on all three architectures. User-mode transitions work (Ring 3 via SYSCALL/SYSRET on x86_64). However, loading external ELF binaries from a filesystem requires:

1. A block device driver (virtio-blk) accessible from user space
2. A filesystem on the block device containing the test binary
3. The shell's `exec` built-in to load and run the binary

### Future: Automated QEMU Testing

Once the kernel can load ELF binaries from a disk image, the automated flow will be:

```bash
# 1. Build kernel
./build-kernel.sh x86_64 dev

# 2. Cross-compile tests
./scripts/cross-compile-test.sh --arch x86_64

# 3. Create a disk image with the test binary
dd if=/dev/zero of=/tmp/VeridianOS/test-disk.img bs=1M count=4
mkfs.ext2 /tmp/VeridianOS/test-disk.img
# ... mount, copy binaries, unmount ...

# 4. Boot QEMU with the test disk
qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -drive id=disk1,if=none,format=raw,file=/tmp/VeridianOS/test-disk.img \
    -device virtio-blk-pci,drive=disk1 \
    -serial stdio -display none -m 256M \
    </dev/null > /tmp/VeridianOS/e2e.log 2>&1 &

# 5. Check output
grep "E2E_TEST_PASS" /tmp/VeridianOS/e2e.log
grep "MINIMAL_TEST_PASS" /tmp/VeridianOS/e2e.log
```

### Manual Testing via Embedded Binary

As an interim approach, test binaries can be embedded into the kernel image as static byte arrays and loaded directly by the init system. This avoids the filesystem dependency but requires a kernel rebuild for each test change.

## Troubleshooting

### "cross-compiler not found"

The VeridianOS GCC cross-compiler is not installed at the expected path. Either:
- Build it from source (see `docs/CROSS-COMPILATION.md`)
- Set `--toolchain /path/to/toolchain` to point to your installation

### "libc.a not found"

Build and install the libc:
```bash
cd userland/libc
make ARCH=x86_64 install
```

### "crt0.o not found"

Assemble the CRT startup file:
```bash
x86_64-veridian-gcc -c \
    -o toolchain/sysroot/lib/x86_64/crt0.o \
    toolchain/sysroot/crt/x86_64/crt0.S
```

### "undefined reference to `main`" (minimal test)

The minimal test provides `_start` directly and must be compiled with `-nostdlib`. If you accidentally link with CRT and the CRT's `_start` calls `main()`, you need to either:
- Add `-nostdlib` to skip the CRT, or
- Rename `_start` to `main` in minimal.c

### Linker errors about missing symbols (stdio, string, etc.)

Ensure libc.a is on the linker search path (`-L`) and listed after the source file (`-lc` at the end of the command). The linker resolves symbols left-to-right.

### "relocation truncated to fit: R_X86_64_32S"

For x86_64 user-space programs, use `-mcmodel=small` (the default). The kernel uses the `kernel` code model, but user-space code uses the standard `small` code model. If you see this error, you may be accidentally using kernel linker flags.

### Wrong syscall numbers

Verify that the syscall numbers in your test match `kernel/src/syscall/mod.rs` and `toolchain/sysroot/include/veridian/syscall.h`. Key numbers:

| Syscall | Number |
|---------|--------|
| `SYS_FILE_WRITE` | 53 |
| `SYS_PROCESS_EXIT` | 11 |
| `SYS_PROCESS_GETPID` | 15 |
| `SYS_MEMORY_BRK` | 23 |

## File Locations

| File | Purpose |
|------|---------|
| `userland/tests/minimal.c` | Raw syscall test (no libc) |
| `userland/tests/hello.c` | Full libc test |
| `userland/tests/Makefile` | Build system for test programs |
| `scripts/cross-compile-test.sh` | Cross-compilation automation script |
| `toolchain/sysroot/crt/<arch>/crt0.S` | C runtime entry point per architecture |
| `userland/libc/` | VeridianOS libc source and Makefile |
| `toolchain/sysroot/include/veridian/syscall.h` | Syscall number definitions and inline wrappers |

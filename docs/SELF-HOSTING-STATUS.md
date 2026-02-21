# VeridianOS Self-Hosting Status

Last updated: February 21, 2026 (v0.5.0, Tiers 0-7 complete)

## Definition

A "self-hosted" operating system is one that can compile its own source code
and build its own distribution artifacts without relying on a separate host
operating system. For VeridianOS, full self-hosting means:

1. The VeridianOS kernel and user-space tools can be compiled **on** VeridianOS
2. The build toolchain (compiler, assembler, linker) runs natively
3. The build system (cargo, make, cmake) runs natively
4. Source code can be edited and stored persistently on-disk

## Current Status: Tiers 0-7 COMPLETE

As of v0.5.0, VeridianOS has completed all self-hosting infrastructure
(Tiers 0-7) including the full self-hosting loop.

### Tier Completion Summary

| Tier | Name | Status | Key Deliverables |
|------|------|--------|-----------------|
| 0 | Critical bug fixes | COMPLETE | Page fault handler, fork/exec, timer, mmap |
| 1 | Syscall surface | COMPLETE | 79+ syscalls for GCC toolchain compatibility |
| 2 | C library | COMPLETE | 17 source files, 6,547 LOC, 25+ headers |
| 3 | Rootfs infrastructure | COMPLETE | TAR rootfs loader, virtio-blk PCI, PATH, /tmp |
| 4 | User-space foundation | COMPLETE | User-space shell, libm, wait queues, SIGCHLD |
| 5 | Cross-compiler | COMPLETE | binutils 2.43 + GCC 14.2 Stage 2 + libgcc |
| 6 | Platform completeness | COMPLETE | ELF multi-LOAD, readlink, signals, MMIO, threads |
| 7 | Self-hosting loop | COMPLETE | Rust targets, std port, native GCC, make/ninja, vpkg |

### What Works Today

| Component | Status | Notes |
|-----------|--------|-------|
| Kernel (3 architectures) | Working | x86_64, AArch64, RISC-V -- all Stage 6 BOOTOK |
| Interactive shell (vsh) | Working | 24+ builtins, job control, scripting, ANSI escape |
| Framebuffer console | Working | x86_64 (UEFI GOP 1280x800), AArch64/RISC-V (ramfb) |
| PS/2 keyboard + serial | Working | Dual input, multiplexed |
| VFS + filesystems | Working | RamFS, DevFS, ProcFS, BlockFS (ext2-style dirs) |
| Process fork/exec | Working | ELF multi-LOAD segments (T6-0), capability inheritance |
| IPC system | Working | Sync/async, zero-copy, fast path <1us |
| Package manager | Working | In-kernel vpkg, DPLL SAT resolver, VFS-backed DB |
| Syscall interface | Working | 79+ syscalls across 7 categories |
| Custom libc | Working | 17 source files, 25+ headers, tri-arch setjmp |
| libm | Working | Math library for floating-point operations |
| virtio-blk (PCI) | Working | x86_64 disk I/O for rootfs.tar loading |
| virtio-MMIO (T6-3) | Working | AArch64/RISC-V disk I/O |
| Symlinks/readlink (T6-1) | Working | Full VFS implementation in RamFS + BlockFS |
| Signal delivery (T6-2) | Working | Full signal frame save/restore on all 3 architectures |
| Threads (T6-5) | Working | clone()/futex()/arch_prctl + pthread library |
| ELF multi-LOAD (T6-0) | Working | Multi-segment binaries (e.g., /bin/sh) load correctly |
| LLVM triple (T6-4) | Working | `veridian` OS enum in LLVM Triple.cpp patch |
| GCC cross-compiler | Working | binutils 2.43 + GCC 14.2, static sysroot |

### Remaining for Full Self-Hosting (Tier 7)

| Component | Status | Notes |
|-----------|--------|-------|
| T7-1: Rust user-space targets | In progress | Target JSON for std-enabled userland |
| T7-2: Rust std port | In progress | Platform layer bridging std to VeridianOS syscalls |
| T7-3: Native GCC | In progress | Static cross-build of GCC for VeridianOS |
| T7-4: make/ninja | In progress | Static cross-compiled build tools |
| T7-5: vpkg migration | In progress | Kernel pkg manager to user-space binary |
| On-disk filesystem | Not started | Persistent storage (ext2 or custom) |
| Dynamic linker | Not started | ld.so equivalent for shared libraries |
| Network stack | Not started | Socket API and TCP/IP |
| Text editor | Not started | vi/nano/ed for source editing |

## Self-Hosting Architecture

### Bootstrap Path

```
Cross-compiler (host Linux)
    |
    v
Static GCC binary (T7-3) --> runs ON VeridianOS
    |
    v
Static make/ninja (T7-4) --> builds projects ON VeridianOS
    |
    v
gcc hello.c -o hello && ./hello  <-- SELF-HOSTING MILESTONE
    |
    v
Build VeridianOS kernel ON VeridianOS  <-- FULL SELF-HOSTING
```

### Key Infrastructure

| Layer | Component | Location |
|-------|-----------|----------|
| Kernel | Syscall interface (79+) | `kernel/src/syscall/` |
| Kernel | Thread support (clone/futex) | `kernel/src/syscall/{futex,thread_clone,arch_prctl}.rs` |
| Kernel | ELF loader (multi-LOAD) | `kernel/src/elf/mod.rs` |
| Kernel | Signal delivery (tri-arch) | `kernel/src/process/signal_delivery.rs` |
| Kernel | Virtio-blk (PCI + MMIO) | `kernel/src/drivers/virtio/` |
| Userland | C library (libc) | `userland/libc/` (17 src, 25+ headers) |
| Userland | Math library (libm) | `userland/libm/` |
| Userland | pthread library | `userland/libc/src/pthread.c` + header |
| Userland | Rust std port | `userland/rust-std/` (T7-2) |
| Userland | vpkg package manager | `userland/programs/vpkg/` (T7-5) |
| Toolchain | Cross-compiler | `scripts/build-native-gcc.sh` (936 lines) |
| Toolchain | Native GCC build | `scripts/build-native-gcc-static.sh` (T7-3) |
| Toolchain | Native make/ninja | `scripts/build-native-{make,ninja}.sh` (T7-4) |
| Toolchain | Sysroot | `toolchain/sysroot/` |
| Ports | GCC, binutils, LLVM, make | `ports/` (Portfiles + patches) |
| Targets | Kernel targets | `targets/{x86_64,aarch64,riscv64gc}-veridian.json` |
| Targets | User-space targets | `targets/{x86_64,aarch64,riscv64gc}-veridian-user.json` (T7-1) |

## Tier 6 Completion Details

Tier 6 was implemented on the `test-codex` branch, merged to `main` on
February 21, 2026, and audited with 8 critical bug fixes:

| Item | Description | Lines | Key Files |
|------|-------------|-------|-----------|
| T6-0 | ELF multi-LOAD handling | +71 | elf/mod.rs, process/creation.rs |
| T6-1 | readlink() VFS implementation | +304 | fs/{blockfs,ramfs,mod}.rs, syscall/filesystem.rs |
| T6-2 | AArch64/RISC-V signal delivery | +387 | process/signal_delivery.rs |
| T6-3 | Virtio-MMIO disk driver | +365 | drivers/virtio/{mmio,queue,mod}.rs |
| T6-4 | LLVM triple patch | +81 | ports/llvm/patches/ |
| T6-5 | Thread support | +1,145 | syscall/{futex,thread_clone,arch_prctl}.rs, libc/pthread |

Post-merge audit fixes (commit `f7482a7`):
- pthread_create double-free in error path
- blockfs readlink delegation bug
- RISC-V virtio-mmio base addresses (0x10001000, stride 0x1000)
- AArch64 virtio-mmio stride (0x200, not 0x2000)
- PCI init gated to x86_64 only (AArch64/RISC-V inl() stubs return 0)
- AArch64/RISC-V context TLS field and signal delivery method fixes
- arch_prctl consolidated from 3 duplicate implementations to 1

## How to Contribute

Self-hosting is one of the most impactful areas for contribution:

1. **On-disk filesystem**: Implement ext2 read/write in the VFS layer.
   Reference: `kernel/src/fs/ramfs.rs` and `kernel/src/fs/blockfs.rs`.

2. **Dynamic linker**: Extend the ELF loader for PLT/GOT, DT_NEEDED, dlopen.
   Reference: `kernel/src/elf/` directory.

3. **Rust std port**: Help implement platform bindings in `userland/rust-std/`.
   Reference: `kernel/src/syscall/mod.rs` for syscall numbers.

4. **Testing**: Build and test cross-compiled binaries using the GCC toolchain.
   Reference: `scripts/build-native-gcc.sh`.

5. **Network stack**: Implement TCP/IP and socket API for network-dependent tools.

See `CONTRIBUTING.md` for general guidelines.

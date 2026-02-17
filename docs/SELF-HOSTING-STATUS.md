# VeridianOS Self-Hosting Status

Last updated: February 2026 (v0.4.8)

## Definition

A "self-hosted" operating system is one that can compile its own source code
and build its own distribution artifacts without relying on a separate host
operating system. For VeridianOS, full self-hosting means:

1. The VeridianOS kernel and user-space tools can be compiled **on** VeridianOS
2. The build toolchain (compiler, assembler, linker) runs natively
3. The build system (cargo, make, cmake) runs natively
4. Source code can be edited and stored persistently on-disk

## Current Status: Cross-Compilation Only

As of v0.4.8, VeridianOS is strictly a cross-compiled project. All kernel and
user-space development happens on a Linux host.

### What Works Today

| Component              | Status | Notes                                         |
|------------------------|--------|-----------------------------------------------|
| Kernel (3 architectures) | Working | Builds on Linux via `cargo build` with custom targets |
| Kernel shell (vsh)     | Working | Interactive shell with 24+ builtins, runs in kernel space |
| Boot to shell prompt   | Working | All 3 architectures reach `root@veridian:/#`  |
| Framebuffer console    | Working | x86_64 (UEFI GOP), AArch64/RISC-V (ramfb)    |
| PS/2 keyboard input    | Working | Polling-based, x86_64 only                    |
| Serial I/O             | Working | All 3 architectures, UART-based               |
| In-memory filesystem   | Working | RamFS, DevFS, ProcFS -- volatile only         |
| Process fork/exec      | Working | Capability inheritance on fork, ELF loading on exec |
| IPC system             | Working | Sync/async channels, fast path under 1 microsecond |
| Package manager        | Working | In-kernel, VFS-backed database, DPLL resolver |
| Syscall interface      | Working | 55 syscalls implemented across 7 categories   |

### What Cannot Run on VeridianOS Yet

| Component              | Status    | Notes                                      |
|------------------------|-----------|--------------------------------------------|
| Any C/C++ compiler     | Not available | No libc, no ELF dynamic linker          |
| Rust compiler (rustc)  | Not available | Requires libc, LLVM, and file I/O       |
| Text editor            | Not available | Shell builtins only; no vi/nano/ed      |
| Persistent file storage| Not available | No disk driver, no on-disk filesystem   |
| Network access         | Not available | No network stack or socket API          |
| Dynamic linking        | Not available | No ld.so or equivalent                  |
| User-space programs    | Limited   | Shell runs in kernel space; user-mode stubs exist |

## Blockers for Self-Hosting

The following components must be implemented before VeridianOS can approach
self-hosting. They are listed in approximate dependency order.

### 1. Persistent Filesystem

**Priority: Critical**

Without persistent storage, source code and build artifacts cannot survive a
reboot. This requires:

- Block device driver (virtio-blk for QEMU, or AHCI/NVMe for real hardware)
- On-disk filesystem implementation (ext2 as a starting point, or a custom
  journaling filesystem)
- VFS integration to mount block-backed filesystems alongside RamFS

**Estimated effort:** Major -- requires disk I/O path through the entire stack
from hardware interrupts to VFS operations.

### 2. C Library (libc)

**Priority: Critical**

Nearly all Unix software depends on a C library. VeridianOS needs a libc port
that implements the syscall wrappers and standard library functions.

Candidate libraries:

| Library | Pros                            | Cons                              |
|---------|---------------------------------|-----------------------------------|
| musl    | Small, static-linking friendly, clean codebase | Needs full POSIX syscall surface |
| newlib  | Designed for embedded/OS bringup, minimal syscall needs | Less complete POSIX support |
| custom  | Tailored to VeridianOS capabilities | Enormous effort, compatibility risk |

Recommended approach: Start with a newlib port (minimal syscall surface), then
migrate to musl once the syscall interface matures.

**Estimated effort:** Medium to large -- syscall stubs exist, but the shim
layer between libc expectations and VeridianOS capabilities needs careful
design.

### 3. Dynamic Linker

**Priority: High**

A dynamic linker (`ld.so` equivalent) is needed for:

- Shared library support (`libc.so`, `libm.so`, etc.)
- Reducing binary size (static linking duplicates library code)
- Runtime symbol resolution

The ELF loader in the kernel already handles basic relocation processing for
AArch64 and RISC-V. Extending this to full dynamic linking requires:

- PLT/GOT resolution
- `DT_NEEDED` dependency loading
- `dlopen`/`dlsym` API

**Estimated effort:** Large -- dynamic linking is complex and must be correct
for all three architectures.

### 4. Compiler Backend

**Priority: High (after libc)**

Self-hosting requires a compiler that runs on VeridianOS. Options:

| Compiler | Approach                                 | Difficulty |
|----------|------------------------------------------|------------|
| GCC      | Port via Portfile (stage 1 C-only exists) | High -- GCC requires a working libc and POSIX environment |
| LLVM/Clang | Port via Portfile (cmake-based build)  | High -- similar POSIX requirements, but LLVM is the project's preferred backend |
| TCC      | Tiny C Compiler, minimal dependencies    | Medium -- good bootstrap candidate, limited optimization |
| mrustc   | Bootstrap Rust compiler written in C++   | Very high -- experimental, but enables Rust self-hosting |

Recommended bootstrap path:

1. Port TCC (minimal C compiler) as the first native compiler
2. Use TCC to build a minimal GCC or LLVM
3. Use the native GCC/LLVM to build rustc

**Estimated effort:** Very large -- compiler porting is one of the hardest
self-hosting milestones.

### 5. Build System Tooling

**Priority: Medium (after compiler)**

Building the kernel and user space requires:

- `make` or `ninja` (build orchestration)
- `cargo` (Rust package manager and build tool)
- Shell scripting support (the kernel shell `vsh` partially covers this)
- `git` or equivalent (source control, optional for initial self-hosting)

### 6. User-Space Shell Migration

**Priority: Medium**

The current shell (`vsh`) runs in kernel space. For a proper self-hosted
system, the shell must run as a user-space process with:

- Proper SYSCALL/SYSRET transitions (stubs exist for all 3 architectures)
- File descriptor inheritance across fork/exec
- Per-process environment variables and working directory

## Realistic Timeline and Milestones

Self-hosting is a long-term goal. The following milestones represent the
approximate order of work:

| Milestone                        | Phase | Dependencies                | Status       |
|----------------------------------|-------|-----------------------------|--------------|
| Block device driver (virtio-blk) | 5     | Memory-mapped I/O           | Not started  |
| On-disk filesystem (ext2)        | 5     | Block device driver         | Not started  |
| Newlib port (minimal libc)       | 5     | Syscall wrappers, VFS       | Not started  |
| User-space shell                 | 5-6   | fork/exec, fd inheritance   | Stubs exist  |
| TCC port (bootstrap C compiler)  | 6     | libc, persistent filesystem | Not started  |
| GNU Make port                    | 6     | libc, persistent filesystem | Portfile exists |
| GCC/LLVM native build            | 6     | TCC or cross-built stage 1  | Portfiles exist |
| Cargo/rustc native build         | 6+    | LLVM, libc, filesystem      | Not started  |
| Full self-hosting                | 7+    | All of the above            | Not started  |

Conservative estimate: Self-hosting is a Phase 6-7 goal, likely 12-18 months
of focused development from the current state.

## How to Contribute

Self-hosting is one of the most impactful areas for contribution. If you are
interested in helping:

1. **Block device driver**: Implement a virtio-blk driver as a user-space
   process following the existing driver framework in `kernel/src/drivers/`.

2. **Filesystem implementation**: Implement ext2 read/write support in the VFS
   layer. The RamFS implementation in `kernel/src/fs/` serves as a reference.

3. **Libc port**: Start with newlib's `libgloss` syscall stubs targeting
   VeridianOS's syscall numbers (see `kernel/src/syscall/mod.rs`).

4. **Testing**: Run the existing Portfile build steps for binutils and GCC
   using the cross-compiler toolchain and report issues.

5. **Documentation**: Improve the porting guides and syscall reference as you
   discover gaps.

See `CONTRIBUTING.md` in the repository root for general contribution
guidelines. Join the project's issue tracker on GitHub to coordinate with
other contributors.

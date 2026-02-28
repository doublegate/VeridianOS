<!-- markdownlint-disable MD033 -->

# VeridianOS

<div align="center">

<img src="images/VeridianOS_Logo-Only.png" alt="VeridianOS Logo" width="60%" />

## A research microkernel operating system built with Rust

[![CI Status](https://github.com/doublegate/VeridianOS/workflows/CI/badge.svg)](https://github.com/doublegate/VeridianOS/actions)
[![Coverage](https://codecov.io/gh/doublegate/VeridianOS/branch/main/graph/badge.svg)](https://codecov.io/gh/doublegate/VeridianOS)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE-MIT)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE-APACHE)
[![Discord](https://img.shields.io/discord/123456789?label=Discord&logo=discord)](https://discord.gg/24KbHS4C)

</div>

## Overview

**VeridianOS** is a research operating system written in Rust, focused on **correctness, isolation, and explicit architectural invariants**. It is intended as **executable documentation of high-assurance systems design**, not as a production OS or a general-purpose hobby kernel.

The project explores how capability-oriented design, strong isolation boundaries, and disciplined use of unsafe code can be combined to produce systems that are _auditable, teachable, and resilient to failure_. VeridianOS features a capability-based security model, zero-copy IPC, and multi-architecture support with an emphasis on reliability and deterministic behavior.

### Key Features

- 🛡️ **Capability-based security** — Unforgeable tokens for all resource access
- 🚀 **Microkernel architecture** — Minimal kernel with services in user space
- 🦀 **Written in Rust** — Memory safety without garbage collection
- ⚡ **High performance** — Lock-free algorithms, zero-copy IPC
- 🔧 **Multi-architecture** — x86_64, AArch64, and RISC-V support
- 🔒 **Security focused** — Mandatory access control, secure boot, hardware security
- 📦 **Modern package management** — Source and binary package support
- 🖥️ **Wayland compositor** — Modern display server with GPU acceleration

---

## Purpose

VeridianOS exists to explore and demonstrate:

- Capability-based system design with explicit authority boundaries
- Strong isolation between kernel, drivers, services, and userland
- Memory safety and ownership as architectural properties
- Deterministic, inspectable system behavior
- Long-horizon durability over short-term feature velocity

---

## Non-Goals

VeridianOS intentionally does **not** aim to be:

- A natively POSIX-based operating system (a POSIX compatibility layer is planned for future phases to support software porting, but native APIs remain capability-based)
- A Linux replacement or distribution
- A performance-first microbenchmark platform
- A feature-complete general-purpose OS

These exclusions are deliberate and protect architectural clarity. Where future compatibility layers are mentioned (e.g., POSIX, Wayland), they will be implemented as user-space libraries that translate to native capability-based interfaces, never as kernel-level compromises.

---

## Threat Model (Bounded)

VeridianOS assumes a single-machine environment with a trusted toolchain. It focuses on software isolation failures, authority misuse, and memory safety violations. Physical attacks, malicious firmware, and advanced side-channel attacks are out of scope by design.

---

## Core Architectural Invariants

The system is defined by explicit invariants governing authority, isolation, memory ownership, and unsafe code usage. These are normative and binding.

See [Invariants](docs/invariants.md) for the authoritative list.

---

## Architecture

VeridianOS uses a microkernel architecture with the following key components:

```text
┌─────────────────────────────────────────────┐
│              User Applications              │
├─────────────────────────────────────────────┤
│    System Services (VFS, Network, etc.)     │
├─────────────────────────────────────────────┤
│    User-Space Drivers (Block, Network)      │
├─────────────────────────────────────────────┤
│    Microkernel (Memory, Scheduling, IPC)    │
└─────────────────────────────────────────────┘
```

---

## Repository Structure

```text
kernel/        Trusted computing base
drivers/       Hardware interaction behind explicit privilege boundaries
services/      Capability-mediated system services
userland/      User processes, libc, libm, Rust std port, vpkg, test programs
boot/          Bootloader and early initialization
targets/       Rust target JSON specs (kernel and user-space, all 3 architectures)
scripts/       Build infrastructure (cross-toolchain, sysroot, rootfs, native GCC/make/ninja)
toolchain/     CRT files, sysroot headers, CMake/Meson cross-compilation configs
ports/         Port definitions for external software (binutils, gcc, make, ninja, etc.)
docs/          Canonical specifications
experiments/   Non-normative exploratory work
```

---

## Project Status

**Latest Release**: v0.6.3 (February 27, 2026) | **Releases Published**: 42 (v0.1.0 through v0.6.3)

| Metric                 | Value                                           |
| ---------------------- | ----------------------------------------------- |
| Build                  | 0 errors, 0 warnings across all 3 architectures |
| Boot Tests             | 29/29 (all architectures, Stage 6 BOOTOK)       |
| Host-Target Unit Tests | 646/646 passing                                 |
| CI Pipeline            | 10/10 jobs passing (GitHub Actions + Codecov)   |

### Architecture Support

| Architecture | Build | Boot | Init Tests | Stage 6 | Stable Idle (30s) | Status                                       |
| ------------ | ----- | ---- | ---------- | ------- | ----------------- | -------------------------------------------- |
| x86_64       | ✅    | ✅   | 29/29      | ✅      | ✅ PASS           | **100% Functional** -- UEFI boot via OVMF    |
| AArch64      | ✅    | ✅   | 29/29      | ✅      | ✅ PASS           | **100% Functional** -- Direct kernel loading |
| RISC-V 64    | ✅    | ✅   | 29/29      | ✅      | ✅ PASS           | **100% Functional** -- OpenSBI boot          |

### Development Phases

| Phase | Description               | Status                 | Version | Date     |
| ----- | ------------------------- | ---------------------- | ------- | -------- |
| 0     | Foundation and Tooling    | **Complete**           | v0.1.0  | Jun 2025 |
| 1     | Microkernel Core          | **Complete**           | v0.2.1  | Jun 2025 |
| 2     | User Space Foundation     | **Complete**           | v0.3.2  | Feb 2026 |
| 3     | Security Hardening        | **Complete**           | v0.3.2  | Feb 2026 |
| 4     | Package Ecosystem         | **Complete**           | v0.4.0  | Feb 2026 |
| 5     | Performance Optimization  | **Complete (~90%)**    | v0.5.8  | Feb 2026 |
| 5.5   | Infrastructure Bridge     | **COMPLETE (100%)** | v0.5.13 | Feb 2026 |
| 6     | Advanced Features and GUI | **~100% (desktop complete)** | v0.6.3  | Feb 2026 |
| 7     | Production Readiness     | **Planned**            | --      | --       |

For detailed release notes, see [Release History](docs/RELEASE-HISTORY.md).

### What Is Built

Phases 0 through 4 are complete. The kernel provides:

- **IPC** -- Synchronous/asynchronous channels with zero-copy fast path (<1us)
- **Memory Management** -- Hybrid bitmap+buddy allocator, NUMA-aware, 4-level page tables
- **Process Management** -- Full lifecycle with context switching on all architectures
- **Scheduler** -- CFS with SMP support, load balancing, CPU affinity
- **Capability System** -- 64-bit unforgeable tokens, two-level O(1) lookup, revocation, interrupt capabilities
- **Interrupt Controllers** -- x86_64 APIC (Local + I/O), AArch64 GICv2, RISC-V PLIC with unified IRQ abstraction
- **VFS** -- ramfs, devfs, procfs, blockfs with POSIX-style file operations
- **Security** -- MAC, secure boot, TPM 2.0, ASLR, W^X, Spectre barriers, KPTI, post-quantum crypto
- **Package Manager** -- DPLL SAT resolver, ports system, reproducible builds, Ed25519 signing
- **Interactive Shell (vsh)** -- Bash/Fish-parity serial console shell with 24+ builtins, pipes, redirections, variable expansion, globbing, tab completion, job control, scripting (if/for/while/case), functions, aliases
- **Framebuffer Display** -- 1280x800 text console via UEFI GOP framebuffer (x86_64) and ramfb (AArch64/RISC-V), ANSI color support, PS/2 keyboard input via controller polling, glyph cache, pixel ring buffer, write-combining (PAT) on x86_64
- **Userland Bridge** -- Ring 0 to Ring 3 transitions with SYSCALL/SYSRET on x86_64, 35+ system calls (including clone, futex, arch_prctl, readlink, pipe2)
- **Complete C Library** -- 19 source files, full stdio/stdlib/string/unistd, architecture-specific setjmp/longjmp, 50+ syscall wrappers, 25+ POSIX-compatible headers (network, system, POSIX, C standard), math library (ldexp, frexp, log, exp, sqrt, pow, fabs, floor, ceil, modf)
- **Cross-Compilation Toolchain** -- binutils 2.43 + GCC 14.2 Stage 2 cross-compiler, sysroot with headers and CRT files, CMake/Meson toolchain files; static native GCC toolchain (gcc, cc1, as, ld, ar) via Canadian cross-compilation for on-target self-hosting
- **Coreutils** -- 6 progressively complex POSIX utilities cross-compiled and verified on VeridianOS: echo, cat, wc, ls, sort, and pipeline_test (capstone fork/exec/pipe/waitpid exercise)
- **BusyBox 1.36.1** -- 95 applets cross-compiled with ash shell support; EPIPE/BrokenPipe handling for multi-pipe commands (`yes | head -n 1`), float printf (`%f/%g/%e`) for `seq`, ash interactive mode (isatty/ENOTTY, sysconf, exec family, fnmatch/glob, tcgetpgrp), process lifecycle hardening for 213+ sequential execs (zombie reaping, MAX_PROCESSES=1024, fd leak detection), ARG_MAX enforcement (128KB), strftime (28 format specifiers), popen/pclose
- **POSIX Regex Engine** -- 1291-line BRE/ERE regex library (`regex.h`/`regcomp`/`regexec`/`regfree`) with recursive backtracking NFA, supports `. * + ? ^ $ [...] | () {m,n}`, 12 POSIX character classes ([:alpha:], [:digit:], etc.), enables grep/sed/awk/find BusyBox applets
- **Native Compilation** -- 208/208 BusyBox source files compiled and linked by GCC 14.2 running natively on VeridianOS; POSIX-compliant partial munmap (front trim, back trim, hole punch) for GCC ggc garbage collector; consolidated brk() heap mapping (O(1) per extension); 512MB kernel heap; 768MB per-process heap limit; 8MB user-space stack growth
- **Persistent Storage** -- BlockFS filesystem with on-disk superblock, bitmap, inode table serialization; auto-detected at boot via magic number probe; sync/fsync support; `mkfs-blockfs` host tool for image creation
- **Virtio-blk Driver** -- Block I/O with TAR rootfs loader for cross-compiled user-space binaries; virtio-MMIO transport on AArch64/RISC-V, PCI on x86_64
- **Thread Support** -- clone() with CLONE_VM/CLONE_FS/CLONE_THREAD/CLONE_SETTLS, futex (WAIT/WAKE/REQUEUE/BITSET), POSIX pthread library (create/join/detach/mutex/cond/TLS)
- **Signal Delivery** -- Full signal frames and trampolines on all three architectures (x86_64, AArch64, RISC-V) with sigreturn context restoration
- **Symlink Support** -- Full readlink() implementation across BlockFS and RamFS with VFS-level dispatch

### Self-Hosting Roadmap

The self-hosting effort follows a tiered plan to build VeridianOS toward compiling its own software natively:

| Tier | Description                                                              | Status                                |
| ---- | ------------------------------------------------------------------------ | ------------------------------------- |
| 0    | Kernel infrastructure (syscalls, ELF loader, virtio-blk)                 | **Complete**                          |
| 1    | C standard library (stdio, stdlib, string, unistd, math)                 | **Complete**                          |
| 2    | Cross-compilation toolchain (binutils 2.43 + GCC 14.2)                   | **Complete**                          |
| 3    | User-space execution (`/bin/minimal` verified, process lifecycle)        | **Complete**                          |
| 4    | Sysroot and CRT files (crt0.S, crti.S, crtn.S, all 3 architectures)      | **Complete**                          |
| 5    | Cross-compiled programs running on VeridianOS                            | **Complete**                          |
| 6    | Thread support, signal delivery, virtio-MMIO, multi-LOAD ELF, native GCC | **Complete** (merged from test-codex) |
| 7    | Full self-hosting (Rust std port, native GCC, make/ninja, vpkg)          | **Complete** (v0.5.0)                 |

Tier 6 was developed on the test-codex branch and merged to main with a comprehensive audit pass fixing 8 critical bugs. Tier 7 provides the complete self-hosting toolchain: T7-1 (Rust user-space target specs), T7-2 (Rust std platform port), T7-3 (static native GCC via Canadian cross-compilation), T7-4 (GNU Make + Ninja), and T7-5 (vpkg package manager). The native GCC toolchain (T7-3) uses CONFIG_SITE-based autoconf caching to solve endianness detection in Canadian cross builds (`build=linux, host=veridian, target=veridian`), producing statically-linked gcc, cc1, as, ld, ar, and related tools totaling ~91 MB.

### Recent Kernel Updates (Tier 6 Self-Hosting)

- Futex/threads: wait/wake/requeue validation, futex bitset filtering, CLONE_FS per-thread cwd/umask sharing, TLS-preserving clone/pthread trampoline, child-cleartid wake.
- Virtio: AArch64/RISC-V virtio-mmio transport (replaces PCI-only); probing fails fast on feature negotiation errors; PCI gated to x86_64 only.
- Filesystem/exec: BlockFS symlink/readlink works; ELF loader handles multi-LOAD binaries while retaining stack mappings; per-thread FS state wired through syscalls.
- Signals: Full signal frame construction and sigreturn on AArch64 (x0-x30, NEON q0-q31) and RISC-V (x1-x31, f0-f31) with architecture-specific trampolines.
- Tooling: LLVM triple patched for `-veridian`; rustup targets installed for x86_64/aarch64/riscv64; `arch_prctl` TLS wired on all arches.

### What Comes Next

- **Phase 6 Core** -- Wayland compositor with software rendering, graphical desktop with gradient background and windowed applications, PS/2 mouse driver, unified input event system, TCP/IP network stack with VirtIO-Net/Ethernet/ARP/TCP/DHCP, 19 new syscalls, `startgui` shell command boots to desktop (v0.6.1)
- **Phase 6 Remaining** -- GPU acceleration, real Wayland client applications, multimedia, virtualization, cloud-native features, POSIX compatibility layer

### Technical Notes

**AArch64 FP/NEON fix**: LLVM emits NEON/SIMD instructions (`movi v0.2d`, `str q0`) for buffer zeroing on buffers >= 16 bytes. Without CPACR_EL1.FPEN enabled, these instructions trap silently. Fixed by enabling FP/NEON in `boot.S` before entering Rust code.

**UnsafeBumpAllocator on AArch64**: AArch64 uses the same lock-free bump allocator as RISC-V, with a simple load-store allocation path (no CAS) and direct atomic initialization with DSB SY/ISB memory barriers.

**bare_lock::RwLock**: UnsafeCell-based single-threaded RwLock replacement for AArch64 bare metal, used in VFS filesystem modules to avoid `spin::RwLock` CAS spinlock hangs without proper exclusive monitor configuration.

**AArch64 LLVM workaround**: AArch64 bypasses a critical LLVM loop-compilation bug by routing `print!`/`println!` through `DirectUartWriter`, which uses `uart_write_bytes_asm()` -- a pure assembly loop that LLVM cannot miscompile. The `kprintln!` macro provides an alternative path using `direct_print_str()` for literal-only output. See [README - LLVM Bug](kernel/src/arch/aarch64/README_LLVM_BUG.md) for details.

### Maturity

VeridianOS is an active research system. Phases 0 through 5.5 are architecturally stable; Phase 6 (advanced features and GUI) is in active development with a functional graphical desktop.

Historical status is recorded in:

- [`RELEASE-HISTORY.md`](docs/RELEASE-HISTORY.md) -- Detailed per-release notes
- [`PROJECT-STATUS.md`](docs/status/PROJECT-STATUS.md)
- [`PHASE2-STATUS-SUMMARY.md`](docs/status/PHASE2-STATUS-SUMMARY.md)
- [`BOOTLOADER-UPGRADE-STATUS.md`](docs/status/BOOTLOADER-UPGRADE-STATUS.md)

Normative truth lives in this README and `docs/`.

---

## Quick Start

### Prerequisites

- Rust nightly-2025-11-15 or later
- QEMU 9.0+ (10.2+ recommended; for testing)
- 8GB RAM (16GB recommended)
- 20GB free disk space

### Building and Running

```bash
# Clone the repository
git clone https://github.com/doublegate/VeridianOS.git
cd VeridianOS

# Install dependencies (Ubuntu/Debian)
./scripts/install-deps.sh

# Build all architectures
./build-kernel.sh all dev      # Development build
./build-kernel.sh all release  # Release build

# Build a specific architecture
./build-kernel.sh x86_64 dev
./build-kernel.sh aarch64 release
./build-kernel.sh riscv64 dev

# Run in QEMU
just run

# Or build manually (x86_64 requires custom target)
cargo build --target targets/x86_64-veridian.json \
    -p veridian-kernel \
    -Zbuild-std=core,compiler_builtins,alloc

# Run in QEMU (x86_64 - UEFI boot, requires OVMF)
# build-kernel.sh creates the UEFI disk image automatically
qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -serial stdio -display none -m 256M

# Run in QEMU (AArch64)
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \
    -kernel target/aarch64-unknown-none/debug/veridian-kernel \
    -serial stdio -display none

# Run in QEMU (RISC-V)
qemu-system-riscv64 -M virt -m 256M -bios default \
    -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel \
    -serial stdio -display none
```

#### Persistent Storage (BlockFS)

```bash
# Build the cross-compiled BusyBox rootfs (first time only)
./scripts/build-busybox-rootfs.sh all

# Create a 256MB persistent BlockFS image populated from rootfs
./scripts/build-busybox-rootfs.sh blockfs

# Boot with persistent storage
./scripts/run-veridian.sh --blockfs

# Or manually:
qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -drive file=target/rootfs-blockfs.img,if=none,id=vd0,format=raw \
    -device virtio-blk-pci,drive=vd0 \
    -serial stdio -display none -m 2048M
```

For detailed build instructions, see [BUILD-INSTRUCTIONS.md](docs/BUILD-INSTRUCTIONS.md).

---

## Supported Platforms

### Architectures

- **x86_64** — Full support (UEFI boot via bootloader 0.11.15)
- **AArch64** — Full support (direct QEMU `-kernel` loading)
- **RISC-V (RV64GC)** — Full support (direct QEMU `-kernel` loading via OpenSBI)

### Minimum Requirements

- 64-bit CPU with MMU
- 256MB RAM (1.5GB for persistent storage / native compilation)
- 1GB storage

### Recommended Requirements

- Multi-core CPU with virtualization support
- 4GB+ RAM
- NVMe storage

---

## Documentation

- 📖 [Architecture Overview](docs/ARCHITECTURE-OVERVIEW.md) — System design and architecture
- 🛠️ [Development Guide](docs/DEVELOPMENT-GUIDE.md) — Getting started with development
- 📚 [API Reference](docs/API-REFERENCE.md) — System call and library APIs
- 🧪 [Testing Strategy](docs/TESTING-STRATEGY.md) — Testing approach and guidelines
- 🔍 [Troubleshooting](docs/TROUBLESHOOTING.md) — Common issues and solutions

### Implementation Guides

- 🗺️ [Implementation Roadmap](docs/IMPLEMENTATION-ROADMAP.md) — Detailed development plan
- 🔄 [Software Porting Guide](docs/SOFTWARE-PORTING-GUIDE.md) — Porting Linux software to VeridianOS
- 🔧 [Compiler Toolchain Guide](docs/COMPILER-TOOLCHAIN-GUIDE.md) — Native compiler integration strategy
- 💾 [Persistent Storage Guide](docs/PERSISTENT-STORAGE.md) — BlockFS filesystem and disk image management
- 🚀 [Future Development Insights](docs/FUTURE-DEVELOPMENT-INSIGHTS.md) — Analysis and recommendations

### Development Phases

The project follows a phased development approach:

1. [Phase 0: Foundation](docs/00-PHASE-0-FOUNDATION.md) — Build system and tooling
2. [Phase 1: Microkernel Core](docs/01-PHASE-1-MICROKERNEL-CORE.md) — Core kernel functionality
3. [Phase 2: User Space Foundation](docs/02-PHASE-2-USER-SPACE-FOUNDATION.md) — Essential services
4. [Phase 3: Security Hardening](docs/03-PHASE-3-SECURITY-HARDENING.md) — Security features
5. [Phase 4: Package Ecosystem](docs/04-PHASE-4-PACKAGE-ECOSYSTEM.md) — Package management
6. [Phase 5: Performance Optimization](docs/05-PHASE-5-PERFORMANCE-OPTIMIZATION.md) — Performance tuning
7. [Phase 6: Advanced Features](docs/06-PHASE-6-ADVANCED-FEATURES.md) — GUI and advanced features

See [PROJECT-STATUS.md](docs/PROJECT-STATUS.md) for detailed status information and [Master TODO](to-dos/MASTER_TODO.md) for task tracking.

---

## How to Read the Code

1. [Invariants](docs/invariants.md) — Architectural invariants (start here)
2. [Architecture](docs/architecture.md) — System architecture

Helpful diagrams:

- [Mermaid - Architecture Capability Flow](docs/diagrams/architecture-capability-flow.mmd)
- [Mermaid - Kernal Entry Points](docs/diagrams/kernel-entry-points.mmd)

1. [Kernel Entry Points](docs/kernel-entry-points.md) — Kernel entry points
2. [Capability Flow](docs/capability-flow.md) — Capability flow into services and drivers

---

## Unsafe Code Policy

Unsafe Rust is permitted only to enforce higher-level invariants and is strictly controlled.

See [Unsafe Policy](docs/unsafe-policy.md).

---

## Performance Targets

VeridianOS is not a performance-first system, but targets reasonable latency for a research microkernel:

**Phase 1 targets** (achieved):

- IPC Latency: < 5μs
- Context Switch: < 10μs
- Microkernel Size: < 15,000 lines of code

**Phase 5 targets** (planned):

- IPC Latency: < 1μs
- Memory Allocation: < 1μs
- System Call Overhead: < 100ns
- Support for 1000+ concurrent processes

Design properties that support these targets include lock-free data structures in critical paths, zero-copy IPC, NUMA-aware memory allocation, and sub-microsecond system call paths.

---

## Security

Security is a fundamental design principle:

- **Capability-based access control** — Fine-grained, unforgeable permissions
- **Secure boot** — Full chain of trust verification
- **Memory safety** — Rust's ownership guarantees plus runtime checks
- **Mandatory access control** — SELinux-style policies
- **Hardware security** — TPM, HSM, and TEE integration

---

## Technical Roadmap

### Completed (2025-2026)

- [x] **Phase 0**: Foundation and Tooling (v0.1.0, Jun 2025)
- [x] **Phase 1**: Microkernel Core (v0.2.1, Jun 2025)
- [x] **Phase 2**: User Space Foundation (v0.3.2, Feb 2026)
- [x] **Phase 3**: Security Hardening (v0.3.2, Feb 2026)
- [x] **Phase 4**: Package Ecosystem and Self-Hosting (v0.4.0, Feb 2026)
- [x] **Self-Hosting Tiers 0-5**: Complete libc, cross-toolchain, user-space execution (v0.4.9, Feb 2026)
- [x] **Self-Hosting Tier 6**: Thread support, signal delivery, virtio-MMIO, multi-LOAD ELF, LLVM triple, native GCC infrastructure (merged from test-codex, Feb 2026)
- [x] **Self-Hosting Tier 7**: Full self-hosting toolchain -- Rust user-space targets, std port, static native GCC 14.2 via Canadian cross-compilation, GNU Make + Ninja, vpkg package manager (v0.5.0, Feb 2026)
- [x] **Coreutils + Toolchain Validation**: 6 progressive POSIX coreutils (echo, cat, wc, ls, sort, pipeline_test) cross-compiled and verified on-target, pipe fd corruption fix, tri-arch clippy clean (v0.5.1, Feb 2026)
- [x] **BusyBox Integration**: BusyBox 1.36.1 cross-compiled with 95 applets and ash shell, EPIPE handling, float printf, pipe improvements, Phase C native compilation infrastructure (384MB heap, sbrk hardening), POSIX BRE/ERE regex engine, CI target fix (v0.5.2, Feb 2026)
- [x] **Phase 5 Sprint 1**: Scheduler context switch wiring, IPC blocking/wake + fast path, TODO(phase5) resolution across 56 items in 31 files, user-space /sbin/init process, dead_code audit reduction, native binary execution verification (v0.5.6, Feb 2026)
- [x] **Phase 5.5 Infrastructure Bridge**: ACPI table parser, APIC timer 1000Hz preemptive scheduling, IPI/SMP, PCI/PCIe completion, DMA/IOMMU, POSIX shared memory, Unix domain sockets, lock-free RCU/hazard pointers, NVMe driver, VirtIO-Net, hardware PMU, 2MB huge pages, dynamic linker (v0.5.9-v0.5.13, Feb 2026)
- [x] **Pre-Phase 6 Tech Debt Remediation**: 12 new syscalls (shm_open/unlink/truncate, socket create/bind/listen/connect/accept/send/recv/close/socketpair), PMU bootstrap wiring, RCU scheduler integration, NVMe PCI enumeration, IOMMU DMAR detection, dynamic linker segment copy fix, stale documentation correction (v0.6.0, Feb 2026)

- [x] **Phase 6 Core (Waves 1-5)**: Graphical desktop with Wayland compositor (wire protocol, SHM buffers, surface compositing, XDG shell), PS/2 mouse driver, unified input events, TCP/IP network stack (VirtIO-Net, Ethernet, ARP, TCP state machine, DHCP client), 19 new syscalls (230-255), `startgui` desktop command, 5 network shell commands (v0.6.1, Feb 2026)
- [x] **Phase 6 Completion**: Documentation sync (all Phase 6 references updated from ~5% to ~40%), AF_INET socket creation wired to net::socket, VirtIO-Net/E1000 device registry integration, UDP recv_from wired to socket buffer layer, all 43 TODO(phase6) markers resolved (4 wired + 39 reclassified to Phase 7), Phase 7 TODO roadmap generated (15 categories, ~93 items) (v0.6.2, Feb 2026)

### Upcoming

- [ ] **Phase 7**: GPU acceleration, advanced Wayland (DMA-BUF, XWayland, client library), multimedia (audio server, video), virtualization (KVM, containers), cloud-native, POSIX compatibility layer

See [Release History](docs/RELEASE-HISTORY.md) for detailed per-release notes.

---

## Contributing

Contributions are welcome. Please see the [Contributing Guide](CONTRIBUTING.md) for details on the code of conduct, development workflow, coding standards, and pull request process.

---

## Community

- [Discord Server](https://discord.gg/24KbHS4C) — Real-time chat
- [Issue Tracker](https://github.com/doublegate/VeridianOS/issues) — Bug reports and feature requests

---

## License

VeridianOS is dual-licensed under:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

You may choose either license for your use.

---

## Acknowledgments

VeridianOS builds upon ideas from many excellent operating systems:

- **seL4** — Formal verification and capability systems
- **Redox OS** — Rust OS development practices
- **Fuchsia** — Component-based architecture
- **FreeBSD** — Driver framework inspiration
- **Linux** — Hardware support reference

---

<div align="center">

![Alt](https://repobeats.axiom.co/api/embed/1292141e5c9e3241d1afa584338f1dfdb278a269.svg "Repobeats analytics image")

<img src="images/VeridianOS_Full-Logo.png" alt="VeridianOS Full Banner" width="60%" />

**Building the future of operating systems, one commit at a time.**

</div>
<!-- markdownlint-enable MD033 -->

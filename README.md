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

The project explores how capability-oriented design, strong isolation boundaries, and disciplined use of unsafe code can be combined to produce systems that are *auditable, teachable, and resilient to failure*. VeridianOS features a capability-based security model, zero-copy IPC, and multi-architecture support with an emphasis on reliability and deterministic behavior.

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

- A POSIX-compatible operating system
- A Linux replacement or distribution
- A performance-first microbenchmark platform
- A feature-complete general-purpose OS

These exclusions are deliberate and protect architectural clarity.

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

```
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

```
kernel/        Trusted computing base
drivers/       Hardware interaction behind explicit privilege boundaries
services/      Capability-mediated system services
userland/      Intentionally constrained user processes
boot/          Bootloader and early initialization
docs/          Canonical specifications
experiments/   Non-normative exploratory work
```

---

## Project Status

**Last Updated**: February 14, 2026 (v0.3.2)

### Current Architecture Support

| Architecture | Build | Boot | Init Tests | Stage 6 | Stable Idle (30s) | Status |
|--------------|-------|------|-----------|---------|-------------------|--------|
| x86_64       | ✅    | ✅   | 22/22     | ✅      | ✅ PASS           | **100% Functional** — UEFI boot via OVMF |
| AArch64      | ✅    | ✅   | 22/22     | ✅      | ✅ PASS           | **100% Functional** — Direct kernel loading |
| RISC-V 64    | ✅    | ✅   | 22/22     | ✅      | ✅ PASS           | **100% Functional** — OpenSBI boot |

### Phase 0: Foundation & Tooling — Complete (v0.1.0)

Released June 7, 2025.

### Phase 1: Microkernel Core — Complete (v0.2.1)

Started June 8, 2025. Completed June 12, 2025. Maintenance release v0.2.1 on June 17, 2025.

Core subsystems implemented:

- **IPC System** — Synchronous/asynchronous channels, registry, performance tracking, rate limiting, capability integration
- **Memory Management** — Frame allocator, virtual memory, page tables, bootloader integration, VAS cleanup
- **Process Management** — PCB, threads, context switching, synchronization primitives, syscalls
- **Scheduler** — CFS, SMP support, load balancing, CPU hotplug, task management
- **Capability System** — Tokens, rights, space management, inheritance, revocation, per-CPU cache
- **Test Framework** — `no_std` test framework with benchmarks, IPC/scheduler/process tests

### Technical Debt Remediation (v0.2.4)

Released February 13, 2026. Comprehensive codebase quality improvement:

- **550 `// SAFETY:` comments** added across 122 files (0.9% to 84.5% coverage)
- **180 new unit tests** across 7 modules (70 to 250 total)
- **5 god objects split** into focused submodules (0 files >1000 LOC remaining)
- **201 TODO/FIXME/HACK** comments triaged with phase tags
- **204 files** with module-level documentation (up from ~60)
- **39 files** cleaned of `#[allow(dead_code)]` with proper feature gating
- **161 files changed** total

### Technical Debt Remediation (v0.3.1)

Released February 14, 2026. Comprehensive 5-sprint remediation covering safety, soundness, and architecture:

- **Critical Safety** — Fixed OnceLock::set() use-after-free soundness bug, fixed process_compat memory leak, added `#[must_use]` to KernelError
- **Static Mut Elimination** — Converted 48 of 55 `static mut` declarations to safe patterns (OnceLock, Mutex, Atomics); 7 retained with documented SAFETY justifications (pre-heap boot, per-CPU data)
- **Panic-Free Syscalls** — Removed 8 production panic paths from syscall/VFS handlers via error propagation
- **Error Type Migration** — Converted 150+ functions across 18 files from `&'static str` errors to typed `KernelError` (legacy ratio reduced from ~65% to ~37%)
- **Architecture Abstractions** — PlatformTimer trait with 3 arch implementations, memory barrier abstractions (memory_fence, data_sync_barrier, instruction_sync_barrier)
- **Dead Code Cleanup** — Removed 25 incorrect `#[allow(dead_code)]` annotations plus 1 dead function

### Phase 2 & Phase 3 Completion (v0.3.2)

Released February 14, 2026. Comprehensive completion of both Phase 2 (User Space Foundation: 80% to 100%) and Phase 3 (Security Hardening: 65% to 100%) across 15 implementation sprints:

**Phase 2 Sprints (6):**

- **Clock/Timestamp Infrastructure** — `get_timestamp_secs()`/`get_timestamp_ms()` wrappers; RamFS/ProcFS/DevFS timestamp integration; VFS `list_mounts()`; init system and shell uptime using real timers
- **BlockFS Directory Operations** — ext2-style `DiskDirEntry` parsing; `readdir()`, `lookup_in_dir()`, `create_file()`, `create_directory()` with `.`/`..`, `unlink_from_dir()`, `truncate_inode()` block freeing
- **Signal Handling + Shell Input** — PTY signal delivery (SIGINT, SIGWINCH); architecture-conditional serial input (x86_64 port I/O, AArch64 UART MMIO, RISC-V SBI getchar); touch command implementation
- **ELF Relocation Processing** — `process_relocations()` with AArch64 (R_AARCH64_RELATIVE/GLOB_DAT/JUMP_SLOT/ABS64) and RISC-V (R_RISCV_RELATIVE/64/JUMP_SLOT) types; PIE binary support; dynamic linker bootstrap delegation
- **Driver Hot-Plug Event System** — `DeviceEvent` enum (Added/Removed/StateChanged); `DeviceEventListener` trait; publish-subscribe notification; auto-probe on device addition
- **Init System Hardening** — Service wait timeout with SIGKILL; exponential backoff restart (base_delay * 2^min(count,5)); architecture-specific reboot (x86_64 keyboard controller 0xFE, AArch64 PSCI, RISC-V SBI reset); timer-based sleep replacing spin loops

**Phase 3 Sprints (9):**

- **Cryptographic Algorithms** — ChaCha20-Poly1305 AEAD (RFC 8439); Ed25519 sign/verify (RFC 8032); X25519 key exchange (RFC 7748); ML-DSA/Dilithium sign/verify (FIPS 204); ML-KEM/Kyber encapsulate/decapsulate (FIPS 203); ChaCha20-based CSPRNG with hardware entropy seeding
- **Secure Boot Verification** — Kernel image SHA-256 hashing via linker symbols; Ed25519 signature verification; measured boot with measurement log; TPM PCR extension; certificate chain validation
- **TPM Integration** — MMIO-based TPM 2.0 communication; locality management; command marshaling (TPM2_Startup, PCR_Extend, PCR_Read, GetRandom); `seal_key()`/`unseal_key()` for TPM-backed storage
- **MAC Policy System** — Text-based policy language parser (`allow source target { perms };`); domain transitions; RBAC layer (users to roles to types); MLS support (sensitivity + categories + dominance); `SecurityLabel` struct replacing `&'static str` labels
- **Audit System Completion** — Event filtering by type; structured format (timestamp, PID, UID, action, target, result); VFS-backed persistent storage; binary serialization; wired into syscall dispatch, capability ops, MAC decisions; real-time alert hooks
- **Memory Protection Hardening** — ChaCha20 CSPRNG-based ASLR entropy; DEP/NX enforcement via page table NX bits; guard page integration with VMM; W^X enforcement; stack guard pages; Spectre v1 barriers (LFENCE/CSDB); KPTI (separate kernel/user page tables on x86_64)
- **Authentication Hardening** — Real timestamps for MFA; PBKDF2-HMAC-SHA256 password hashing; password complexity enforcement; password history (prevent reuse); account expiration
- **Capability System Phase 3** — ObjectRef::Endpoint in IPC integration; PRESERVE_EXEC filtering; default IPC/memory capabilities; process notification on revocation; permission checks; IPC broadcast for revocation
- **Syscall Security + Fuzzing** — MAC checks before capability checks in syscall handlers; audit logging in syscall entry/exit; argument validation (pointer bounds, size limits); `FuzzTarget` trait with mutation-based fuzzer; ELF/IPC/FS/capability fuzz targets; crash detection via panic handler hooks

### Phase 3: Security Hardening — Complete (v0.3.0, v0.3.2)

Initial release February 14, 2026 (v0.3.0). Fully completed February 14, 2026 (v0.3.2). Architecture leakage reduction and comprehensive security hardening:

- **Architecture Leakage Reduction** — `kprintln!`/`kprint!` macro family, `IpcRegisterSet` trait, heap/scheduler consolidation; `cfg(target_arch)` outside `arch/` reduced from 379 to 204 (46% reduction)
- **Test Expansion** — Kernel-mode init tests expanded from 12 to 22, all passing on all architectures
- **Capability System Hardening** — Root capability bootstrap, per-process resource quotas (256 cap limit), syscall enforcement (fork/exec/kill require Process cap)
- **MAC + Audit** — MAC convenience functions wired into VFS `open()`/`mkdir()`, audit events for capability and process lifecycle
- **Memory Hardening** — Speculation barriers (LFENCE/CSDB/FENCE.I) at syscall entry, guard pages in VMM, stack canary integration
- **Crypto Validation** — SHA-256 NIST FIPS 180-4 test vector validation

### Phase 2: User Space Foundation — Complete (v0.2.3, parity v0.2.5, completed v0.3.2)

Started August 15, 2025. Architecturally complete August 16, 2025. Runtime activation verified February 13, 2026. Full multi-architecture boot parity achieved February 13, 2026 (v0.2.5) with RISC-V post-BOOTOK crash fix, heap sizing corrections, and 30-second stability tests passing on all architectures.

Implementation achievements:

- **Virtual Filesystem (VFS) Layer** — Mount points, ramfs, devfs (`/dev`), procfs (`/proc`)
- **File Descriptors & Operations** — POSIX-style operations with full syscall suite (open, read, write, close, seek, mkdir, etc.)
- **Live System Information** — `/proc` with real process and memory stats
- **Device Abstraction** — `/dev/null`, `/dev/zero`, `/dev/random`, `/dev/console`
- **Process Server** — Complete process management with resource handling
- **ELF Loader** — Dynamic linking support for user-space applications
- **Thread Management** — Complete APIs with TLS and scheduling policies
- **Standard Library** — C-compatible foundation for user-space
- **Init System** — Service management with dependencies and runlevels
- **Shell Implementation** — 20+ built-in commands with environment management
- **Driver Suite** — PCI/USB bus drivers, network drivers (Ethernet + loopback with TCP/IP stack), storage drivers (ATA/IDE), console drivers (VGA + serial)
- **Runtime Init Tests** — 22 kernel-mode tests (6 VFS + 6 shell + 10 security/capability/crypto) verifying subsystem functionality at boot

### Technical Notes

**AArch64 FP/NEON fix**: Root cause of AArch64 VFS read hangs identified and resolved. LLVM emits NEON/SIMD instructions (`movi v0.2d`, `str q0`) for buffer zeroing on buffers >= 16 bytes. Without CPACR_EL1.FPEN enabled, these instructions trap silently. Fixed by enabling FP/NEON in `boot.S` before entering Rust code.

**UnsafeBumpAllocator on AArch64**: AArch64 now uses the same lock-free bump allocator as RISC-V, with a simple load-store allocation path (no CAS) and direct atomic initialization with DSB SY/ISB memory barriers.

**bare_lock::RwLock**: UnsafeCell-based single-threaded RwLock replacement for AArch64 bare metal, used in VFS filesystem modules to avoid `spin::RwLock` CAS spinlock hangs without proper exclusive monitor configuration.

**AArch64 LLVM workaround**: AArch64 uses an assembly-only approach to bypass a critical LLVM loop compilation bug. All `println!` and `boot_println!` macros are no-ops on AArch64; critical messages use direct UART character writes. See [README - LLVM Bug](kernel/src/arch/aarch64/README_LLVM_BUG.md) for details.

**DEEP-RECOMMENDATIONS**: All 9 of 9 recommendations complete — bootstrap circular dependency fix, AArch64 calling convention, atomic operations, capability overflow, user pointer validation, custom test framework, error type migration, RAII patterns, and Phase 2 readiness.

### Maturity

VeridianOS is an active research system. Core architectural concepts are stable; subsystems evolve deliberately.

Historical status is recorded in:

- [`PROJECT-STATUS.md`](docs/status/PROJECT-STATUS.md)
- [`PHASE2-STATUS-SUMMARY.md`](docs/status/PHASE2-STATUS-SUMMARY.md)
- [`BOOTLOADER-UPGRADE-STATUS.md`](docs/status/BOOTLOADER-UPGRADE-STATUS.md)

Normative truth lives in this README and `docs/`.

---

## Quick Start

### Prerequisites

- Rust nightly-2025-11-15 or later
- QEMU 8.0+ (for testing)
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

# Run in QEMU (x86_64 - requires UEFI disk image)
# First build the UEFI image:
./tools/build-bootimage.sh \
    target/x86_64-veridian/debug/veridian-kernel \
    target/x86_64-veridian/debug

# Then boot with OVMF firmware:
qemu-system-x86_64 \
    -bios /usr/share/edk2/x64/OVMF.4m.fd \
    -drive format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -serial stdio \
    -display none

# Run in QEMU (AArch64)
qemu-system-aarch64 \
    -M virt \
    -cpu cortex-a57 \
    -kernel target/aarch64-unknown-none/debug/veridian-kernel \
    -serial stdio \
    -display none

# Run in QEMU (RISC-V)
qemu-system-riscv64 \
    -M virt \
    -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel \
    -serial stdio \
    -display none
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
- 256MB RAM
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

3. [Kernel Entry Points](docs/kernel-entry-points.md) — Kernel entry points
4. [Capability Flow](docs/capability-flow.md) — Capability flow into services and drivers

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

### Near-term (2025)

- [x] Phase 0: Foundation — Complete (2025-06-07)
- [x] Phase 1: Microkernel Core — Complete (2025-06-12, v0.2.1)
- [x] Phase 2: User Space Foundation — Runtime verified (2025-08-16, v0.2.3)
- [x] Technical Debt Remediation — 9/10 issues resolved (2026-02-13, v0.2.4)
- [x] RISC-V Crash Fix & Architecture Parity — All 3 architectures stable (2026-02-13, v0.2.5)
- [x] Phase 3: Security Hardening — Architecture cleanup, capability hardening, MAC/audit, memory hardening (2026-02-14, v0.3.0)
- [x] Technical Debt Remediation — OnceLock soundness fix, 48 static mut eliminated, typed errors, panic-free syscalls (2026-02-14, v0.3.1)
- [x] Phase 2 & Phase 3 Completion — 15 implementation sprints, full crypto/secure boot/TPM/MAC/audit/ELF/BlockFS/signals (2026-02-14, v0.3.2)

### Mid-term (2026)

- [x] Phase 2: User Space Foundation — 100% Complete (2026-02-14, v0.3.2)
- [x] Phase 3: Security Hardening — 100% Complete (2026-02-14, v0.3.2)
- [x] Technical Debt Remediation — Complete (2026-02-14, v0.3.1)
- [ ] Phase 4: Package Ecosystem & Self-Hosting (5–6 months) — Ports system, LLVM toolchain priority

### Long-term (2027+)

- [ ] Phase 5: Performance Optimization (5–6 months) — Sub-microsecond IPC, lock-free kernel paths, DPDK networking
- [ ] Phase 6: Advanced Features (8–9 months) — Wayland compositor, desktop environment, cloud-native features

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

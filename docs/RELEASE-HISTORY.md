# VeridianOS Release History

Detailed release notes for all VeridianOS versions, in reverse-chronological order.

For current project status, see the [README](../README.md). For task tracking, see the [Master TODO](../to-dos/MASTER_TODO.md).

---

## Table of Contents

- [v0.4.1 -- Technical Debt Remediation](#v041----technical-debt-remediation)
- [v0.4.0 -- Phase 4 Milestone](#v040----phase-4-milestone)
- [v0.3.9 -- Phase 4 Completion + Userland Bridge](#v039----phase-4-completion--userland-bridge)
- [v0.3.8 -- Phase 4 Groups 3+4: Toolchain, Testing, Compliance, Ecosystem](#v038----phase-4-groups-34-toolchain-testing-compliance-ecosystem)
- [v0.3.7 -- Phase 4 Group 2: Ports Build, Reproducible Builds, Security](#v037----phase-4-group-2-ports-build-reproducible-builds-security)
- [v0.3.6 -- Phase 4 Group 1 + Build Fixes](#v036----phase-4-group-1--build-fixes)
- [v0.3.5 -- Critical Architecture Boot Fixes](#v035----critical-architecture-boot-fixes)
- [v0.3.4 -- Phase 1-3 Integration + Phase 4 Package Ecosystem](#v034----phase-1-3-integration--phase-4-package-ecosystem)
- [v0.3.3 -- Technical Debt Remediation](#v033----technical-debt-remediation)
- [v0.3.2 -- Phase 2 and Phase 3 Completion](#v032----phase-2-and-phase-3-completion)
- [v0.3.1 -- Technical Debt Remediation](#v031----technical-debt-remediation)
- [v0.3.0 -- Phase 3 Security Hardening](#v030----phase-3-security-hardening)
- [v0.2.5 -- RISC-V Crash Fix and Architecture Parity](#v025----risc-v-crash-fix-and-architecture-parity)
- [v0.2.4 -- Technical Debt Remediation](#v024----technical-debt-remediation)
- [v0.2.3 -- Phase 2 User Space Foundation](#v023----phase-2-user-space-foundation)
- [v0.2.1 -- Phase 1 Maintenance Release](#v021----phase-1-maintenance-release)
- [v0.2.0 -- Phase 1 Microkernel Core](#v020----phase-1-microkernel-core)
- [v0.1.0 -- Phase 0 Foundation and Tooling](#v010----phase-0-foundation-and-tooling)
- [DEEP-RECOMMENDATIONS](#deep-recommendations)

---

## v0.4.1 -- Technical Debt Remediation

**Released**: February 15, 2026

Cross-cutting remediation across 58 kernel source files:

- **Bootstrap Refactoring** -- `kernel_init_main()` refactored from 370-line monolith to 24-line dispatcher with 6 focused helpers; guarded `unwrap()` on `BOOT_ALLOCATOR` lock replaced with contextual `expect()`
- **Error Handling Audit** -- 22 `let _ =` patterns in security-critical subsystems upgraded to log warnings (auth RNG, SIGCHLD delivery, frame leaks, capability inheritance, network registration, database persistence)
- **Dead Code Consolidation** -- 157 per-item `#[allow(dead_code)]` in `pkg/` replaced with 11 module-level `#![allow(dead_code)]` directives
- **String Error Elimination** -- 7 remaining `Err("...")` in `arch/x86_64/usermode.rs` converted to typed `KernelError` variants
- **TODO Reclassification** -- 35 `TODO(phase4)` reclassified to `TODO(future)`, 12 untagged TODOs given phase markers

58 files changed (+407/-352 lines). All 3 architectures: Stage 6 BOOTOK, 27/27 tests, zero warnings.

---

## v0.4.0 -- Phase 4 Milestone

**Released**: February 15, 2026

Formal Phase 4 milestone with comprehensive syscall API documentation (19 wrappers fully documented with examples, errors, and arguments) and 5 new Phase 4 boot tests bringing the total to 27/27. Version bump to 0.4.0 marks Phase 4 as complete.

8 files changed (+1,294/-103 lines). All 3 architectures: Stage 6 BOOTOK, 27/27 tests, zero warnings.

---

## v0.3.9 -- Phase 4 Completion + Userland Bridge

**Released**: February 15, 2026

Completes Phase 4 (Package Ecosystem) to 100% and implements the Userland Bridge for Ring 0 to Ring 3 transitions.

**Userland Bridge (5 sprints):**

- **GDT User Segments** -- Ring 3 code/data segments (0x30/0x28), SYSCALL/SYSRET MSR configuration (EFER, LSTAR, STAR, SFMASK, KernelGsBase)
- **Embedded Init Binary** -- x86_64 machine code init process (57 bytes) using SYSCALL for sys_write + sys_exit, with ELF header generation
- **Ring 3 Entry via iretq** -- `enter_usermode()` pushing SS/RSP/RFLAGS/CS/RIP frame; page table walker with safe frame allocation (skips bootloader page table pages)
- **Syscall Backends** -- sys_write serial fallback for fd 1/2, sys_read serial input for fd 0, sys_exit process termination
- **Integration** -- Full Ring 0 -> Ring 3 -> SYSCALL -> Ring 0 path verified; init binary prints "VeridianOS init started" via serial

**Phase 4 Finalization:**

- SDK Generator, Plugin System, Async Runtime type definitions
- PHASE4_TODO.md updated to 100% complete

22 files changed, 5 new files. All 3 architectures: Stage 6 BOOTOK, 27/27 tests (5 new Phase 4 tests), zero warnings.

---

## v0.3.8 -- Phase 4 Groups 3+4: Toolchain, Testing, Compliance, Ecosystem

**Released**: February 15, 2026

Three parallel implementation sprints advancing Phase 4 to ~95%:

- **Toolchain Manager** -- Toolchain registry, cross-compiler config, linker script generation, CMake toolchain files
- **Testing + Compliance** -- Package test framework, security scanner (9 patterns), license detection and compatibility checking, dependency graph analysis with cycle detection
- **Statistics + Ecosystem** -- Package stats collector, update notifications, CVE advisory checking, core package ecosystem definitions (base-system, dev-tools, drivers, apps)

5 new files (+2,350 lines). All 3 architectures: Stage 6 BOOTOK, 22/22 tests, zero warnings.

---

## v0.3.7 -- Phase 4 Group 2: Ports Build, Reproducible Builds, Security

**Released**: February 15, 2026

Three parallel implementation sprints advancing Phase 4 to ~85%:

- **Ports Build Execution** -- Real SHA-256 checksum verification, `execute_command()` framework for build steps, VFS-first port collection scanning
- **Reproducible Builds** -- `BuildSnapshot`/`BuildManifest` types, environment normalization (zeroed timestamps, canonical paths), manifest comparison and serialization
- **Repository Security** -- Access control with Ed25519 upload verification, malware pattern scanning (10 default patterns), CVE vulnerability database

5 files changed (+1,385/-49 lines), 1 new file. All 3 architectures: Stage 6 BOOTOK, 22/22 tests, zero warnings.

---

## v0.3.6 -- Phase 4 Group 1 + Build Fixes

**Released**: February 15, 2026

Four parallel implementation sprints advancing Phase 4:

- **Repository Infrastructure** -- Repository index with Ed25519 verification, mirror manager with failover, multi-repo configuration
- **Package Removal** -- Config file preservation on remove/upgrade, orphan package detection and batch removal
- **Binary Delta Updates** -- Block-matching delta computation/application with SHA-256 verification for incremental downloads
- **Config File Tracking** -- FileType classification (Binary/Config/Documentation/Asset) with path-based inference
- **RISC-V Build Fix** -- Changed `jal` to `call` in boot.S (kernel grew past JAL's 1MB range)

7 files changed (+717/-392 lines), 1 new file. All 3 architectures: Stage 6 BOOTOK, 22/22 tests, zero warnings.

---

## v0.3.5 -- Critical Architecture Boot Fixes

**Released**: February 15, 2026

Resolves 3 architecture-specific boot issues:

- **x86_64 CSPRNG Double Fault** -- Added CPUID check for RDRAND support before use; prevents `#UD` -> double fault on CPU models without RDRAND (e.g., QEMU `qemu64`)
- **RISC-V Frame Allocator** -- Fixed memory start address from `0x88000000` (end of RAM) to `0x80E00000` (after kernel image); frame allocations now reference valid physical memory
- **RISC-V Stack Canary Guard** -- Restricted RNG usage during process creation to x86_64 only; prevents unhandled faults on RISC-V (no `stvec` trap handler during creation)
- **x86_64 Boot Stack Overflow** -- Increased boot stack from 64KB to 256KB; prevents silent overflow from `CapabilitySpace` array construction (~20KB) in debug builds

4 files changed (+67/-21 lines). All 3 architectures: Stage 6 BOOTOK, 22/22 tests, zero warnings.

---

## v0.3.4 -- Phase 1-3 Integration + Phase 4 Package Ecosystem

**Released**: February 15, 2026

Two-track release closing Phase 1-3 integration gaps and advancing Phase 4 to ~75% across 14 implementation sprints.

**Phase 1-3 Integration Gaps Closed (7 sprints):**

- **IPC-Scheduler Bridge** -- IPC sync_send/sync_receive now block via scheduler instead of returning ChannelFull/NoMessage; sync_reply wakes blocked senders; async channels wake endpoint waiters after enqueue
- **VMM-Page Table Integration** -- map_region/unmap_region write to real architecture page tables via PageMapper; VAS operations allocate/free physical frames via frame allocator
- **Capability Validation** -- IPC capability validation performs two-level check against process capability space; fast path process lookup uses real process table
- **FPU Context Switching** -- NEON Q0-Q31 save/restore on AArch64; F/D extension f0-f31 save/restore on RISC-V
- **Thread Memory** -- Thread creation allocates real stack frames with guard pages; TLS allocation uses real frame allocation with architecture-specific register setup (FS_BASE/TPIDR_EL0/tp)
- **Shared Memory** -- Regions allocate/free physical frames and flush TLB; transfer_ownership validates target process; unmap properly frees frames
- **Zero-Copy IPC** -- Uses real ProcessPageTable with VAS delegation; allocate_virtual_range uses VAS mmap instead of hardcoded address

**Phase 4 Package Ecosystem (7 sprints, ~75% complete):**

- **Transaction System** -- Package manager with atomic install/remove/upgrade operations and rollback support
- **DPLL SAT Resolver** -- Dependency resolver with version ranges, virtual packages, conflict detection, and backtracking
- **Ports Framework** -- 6 build types (Autotools, CMake, Meson, Cargo, Make, Custom); port collection management with 6 standard categories
- **SDK Types** -- ToolchainInfo, BuildTarget, SdkConfig; typed syscall API wrappers for 6 subsystems; package configuration with .pc file generation
- **Shell Commands** -- install, remove, update, upgrade, list, search, info, verify
- **Package Syscalls** -- SYS_PKG_INSTALL (90) through SYS_PKG_UPDATE (94)
- **Crypto Hardening** -- Real Ed25519 signature verification with trust policies for packages

**Phase 4 Prerequisites:**

- Page fault handler framework with demand paging and stack growth
- ELF dynamic linker support with auxiliary vector and PT_INTERP parsing
- Process waitpid infrastructure with WNOHANG and POSIX wstatus encoding
- Per-process working directory with path normalization

42 files changed (+7,581/-424 lines), 15 new files. AArch64 and RISC-V boot to Stage 6 BOOTOK with 22/22 tests passing.

---

## v0.3.3 -- Technical Debt Remediation

**Released**: February 14, 2026

Comprehensive technical debt remediation across 4 parallel work streams:

- **Soundness and Safety** -- Fixed RiscvScheduler soundness issue (UnsafeCell to spin::Mutex), deleted 353-line dead `security::crypto` module, fixed 5 clippy suppressions, deduplicated x86_64 I/O port functions
- **Error Type Migration** -- Eliminated all remaining `Err("...")` string literals (96 to 0) and `Result<T, &str>` signatures (91 to 1 justified); 11 primary files + ~33 cascade files converted to typed `KernelError`
- **Code Organization** -- Split 3 files exceeding 1,500 lines: `crypto/post_quantum.rs` into directory (kyber/dilithium/hybrid), `security/mac.rs` into directory (parser extracted), `elf/types.rs` extracted; created `arch/entropy.rs` abstraction
- **Comment and Annotation Cleanup** -- 55 `TODO(phase3)` items triaged to zero (9 eliminated, 1 removed as already implemented, 45 reclassified), 15 unnecessary `#[allow(unused_imports)]` removed, `process_compat::Process` renamed to `TaskProcessAdapter`
- **Net result**: 80 files changed, +1,024/-5,069 lines (net -4,045 lines), zero `Result<T, &str>` remaining, zero soundness bugs

---

## v0.3.2 -- Phase 2 and Phase 3 Completion

**Released**: February 14, 2026

Comprehensive completion of both Phase 2 (User Space Foundation: 80% to 100%) and Phase 3 (Security Hardening: 65% to 100%) across 15 implementation sprints.

**Phase 2 Sprints (6):**

- **Clock/Timestamp Infrastructure** -- `get_timestamp_secs()`/`get_timestamp_ms()` wrappers; RamFS/ProcFS/DevFS timestamp integration; VFS `list_mounts()`; init system and shell uptime using real timers
- **BlockFS Directory Operations** -- ext2-style `DiskDirEntry` parsing; `readdir()`, `lookup_in_dir()`, `create_file()`, `create_directory()` with `.`/`..`, `unlink_from_dir()`, `truncate_inode()` block freeing
- **Signal Handling + Shell Input** -- PTY signal delivery (SIGINT, SIGWINCH); architecture-conditional serial input (x86_64 port I/O, AArch64 UART MMIO, RISC-V SBI getchar); touch command implementation
- **ELF Relocation Processing** -- `process_relocations()` with AArch64 (R_AARCH64_RELATIVE/GLOB_DAT/JUMP_SLOT/ABS64) and RISC-V (R_RISCV_RELATIVE/64/JUMP_SLOT) types; PIE binary support; dynamic linker bootstrap delegation
- **Driver Hot-Plug Event System** -- `DeviceEvent` enum (Added/Removed/StateChanged); `DeviceEventListener` trait; publish-subscribe notification; auto-probe on device addition
- **Init System Hardening** -- Service wait timeout with SIGKILL; exponential backoff restart (base_delay * 2^min(count,5)); architecture-specific reboot (x86_64 keyboard controller 0xFE, AArch64 PSCI, RISC-V SBI reset); timer-based sleep replacing spin loops

**Phase 3 Sprints (9):**

- **Cryptographic Algorithms** -- ChaCha20-Poly1305 AEAD (RFC 8439); Ed25519 sign/verify (RFC 8032); X25519 key exchange (RFC 7748); ML-DSA/Dilithium sign/verify (FIPS 204); ML-KEM/Kyber encapsulate/decapsulate (FIPS 203); ChaCha20-based CSPRNG with hardware entropy seeding
- **Secure Boot Verification** -- Kernel image SHA-256 hashing via linker symbols; Ed25519 signature verification; measured boot with measurement log; TPM PCR extension; certificate chain validation
- **TPM Integration** -- MMIO-based TPM 2.0 communication; locality management; command marshaling (TPM2_Startup, PCR_Extend, PCR_Read, GetRandom); `seal_key()`/`unseal_key()` for TPM-backed storage
- **MAC Policy System** -- Text-based policy language parser (`allow source target { perms };`); domain transitions; RBAC layer (users to roles to types); MLS support (sensitivity + categories + dominance); `SecurityLabel` struct replacing `&'static str` labels
- **Audit System Completion** -- Event filtering by type; structured format (timestamp, PID, UID, action, target, result); VFS-backed persistent storage; binary serialization; wired into syscall dispatch, capability ops, MAC decisions; real-time alert hooks
- **Memory Protection Hardening** -- ChaCha20 CSPRNG-based ASLR entropy; DEP/NX enforcement via page table NX bits; guard page integration with VMM; W^X enforcement; stack guard pages; Spectre v1 barriers (LFENCE/CSDB); KPTI (separate kernel/user page tables on x86_64)
- **Authentication Hardening** -- Real timestamps for MFA; PBKDF2-HMAC-SHA256 password hashing; password complexity enforcement; password history (prevent reuse); account expiration
- **Capability System Phase 3** -- ObjectRef::Endpoint in IPC integration; PRESERVE_EXEC filtering; default IPC/memory capabilities; process notification on revocation; permission checks; IPC broadcast for revocation
- **Syscall Security + Fuzzing** -- MAC checks before capability checks in syscall handlers; audit logging in syscall entry/exit; argument validation (pointer bounds, size limits); `FuzzTarget` trait with mutation-based fuzzer; ELF/IPC/FS/capability fuzz targets; crash detection via panic handler hooks

---

## v0.3.1 -- Technical Debt Remediation

**Released**: February 14, 2026

Comprehensive 5-sprint remediation covering safety, soundness, and architecture:

- **Critical Safety** -- Fixed OnceLock::set() use-after-free soundness bug, fixed process_compat memory leak, added `#[must_use]` to KernelError
- **Static Mut Elimination** -- Converted 48 of 55 `static mut` declarations to safe patterns (OnceLock, Mutex, Atomics); 7 retained with documented SAFETY justifications (pre-heap boot, per-CPU data)
- **Panic-Free Syscalls** -- Removed 8 production panic paths from syscall/VFS handlers via error propagation
- **Error Type Migration** -- Converted 150+ functions across 18 files from `&'static str` errors to typed `KernelError` (legacy ratio reduced from ~65% to ~37%)
- **Architecture Abstractions** -- PlatformTimer trait with 3 arch implementations, memory barrier abstractions (memory_fence, data_sync_barrier, instruction_sync_barrier)
- **Dead Code Cleanup** -- Removed 25 incorrect `#[allow(dead_code)]` annotations plus 1 dead function

---

## v0.3.0 -- Phase 3 Security Hardening

**Released**: February 14, 2026

Architecture leakage reduction and comprehensive security hardening:

- **Architecture Leakage Reduction** -- `kprintln!`/`kprint!` macro family, `IpcRegisterSet` trait, heap/scheduler consolidation; `cfg(target_arch)` outside `arch/` reduced from 379 to 204 (46% reduction)
- **Test Expansion** -- Kernel-mode init tests expanded from 12 to 22, all passing on all architectures
- **Capability System Hardening** -- Root capability bootstrap, per-process resource quotas (256 cap limit), syscall enforcement (fork/exec/kill require Process cap)
- **MAC + Audit** -- MAC convenience functions wired into VFS `open()`/`mkdir()`, audit events for capability and process lifecycle
- **Memory Hardening** -- Speculation barriers (LFENCE/CSDB/FENCE.I) at syscall entry, guard pages in VMM, stack canary integration
- **Crypto Validation** -- SHA-256 NIST FIPS 180-4 test vector validation

---

## v0.2.5 -- RISC-V Crash Fix and Architecture Parity

**Released**: February 13, 2026

Full multi-architecture boot parity achieved with RISC-V post-BOOTOK crash fix, heap sizing corrections, and 30-second stability tests passing on all architectures.

---

## v0.2.4 -- Technical Debt Remediation

**Released**: February 13, 2026

Comprehensive codebase quality improvement:

- **550 `// SAFETY:` comments** added across 122 files (0.9% to 84.5% coverage)
- **180 new unit tests** across 7 modules (70 to 250 total)
- **5 god objects split** into focused submodules (0 files >1000 LOC remaining)
- **201 TODO/FIXME/HACK** comments triaged with phase tags
- **204 files** with module-level documentation (up from ~60)
- **39 files** cleaned of `#[allow(dead_code)]` with proper feature gating
- **161 files changed** total

---

## v0.2.3 -- Phase 2 User Space Foundation

**Released**: August 16, 2025 (architecturally complete). Runtime activation verified February 13, 2026.

Implementation achievements:

- **Virtual Filesystem (VFS) Layer** -- Mount points, ramfs, devfs (`/dev`), procfs (`/proc`)
- **File Descriptors and Operations** -- POSIX-style operations with full syscall suite (open, read, write, close, seek, mkdir, etc.)
- **Live System Information** -- `/proc` with real process and memory stats
- **Device Abstraction** -- `/dev/null`, `/dev/zero`, `/dev/random`, `/dev/console`
- **Process Server** -- Complete process management with resource handling
- **ELF Loader** -- Dynamic linking support for user-space applications
- **Thread Management** -- Complete APIs with TLS and scheduling policies
- **Standard Library** -- C-compatible foundation for user-space
- **Init System** -- Service management with dependencies and runlevels
- **Shell Implementation** -- 20+ built-in commands with environment management
- **Driver Suite** -- PCI/USB bus drivers, network drivers (Ethernet + loopback with TCP/IP stack), storage drivers (ATA/IDE), console drivers (VGA + serial)
- **Runtime Init Tests** -- 22 kernel-mode tests (6 VFS + 6 shell + 10 security/capability/crypto) verifying subsystem functionality at boot

---

## v0.2.1 -- Phase 1 Maintenance Release

**Released**: June 17, 2025

Maintenance release with boot fixes, AArch64 LLVM workaround, and all three architectures booting to Stage 6 BOOTOK.

---

## v0.2.0 -- Phase 1 Microkernel Core

**Released**: June 12, 2025 (Phase 1 completed in 5 days, started June 8, 2025)

Core subsystems implemented:

- **IPC System** -- Synchronous/asynchronous channels, registry, performance tracking, rate limiting, capability integration
- **Memory Management** -- Frame allocator, virtual memory, page tables, bootloader integration, VAS cleanup
- **Process Management** -- PCB, threads, context switching, synchronization primitives, syscalls
- **Scheduler** -- CFS, SMP support, load balancing, CPU hotplug, task management
- **Capability System** -- Tokens, rights, space management, inheritance, revocation, per-CPU cache
- **Test Framework** -- `no_std` test framework with benchmarks, IPC/scheduler/process tests

---

## v0.1.0 -- Phase 0 Foundation and Tooling

**Released**: June 7, 2025

Initial release establishing the development foundation:

- Rust nightly toolchain with custom target specifications
- Multi-architecture build system (x86_64, AArch64, RISC-V)
- CI/CD pipeline with GitHub Actions
- QEMU testing infrastructure
- GDB debugging support
- Documentation framework

---

## DEEP-RECOMMENDATIONS

All 9 of 9 recommendations complete:

1. Bootstrap circular dependency fix
2. AArch64 calling convention
3. Atomic operations
4. Capability overflow
5. User pointer validation
6. Custom test framework
7. Error type migration
8. RAII patterns
9. Phase 2 readiness

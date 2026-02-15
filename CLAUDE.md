# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## VeridianOS Overview

VeridianOS is a next-generation microkernel operating system written entirely in Rust, emphasizing security, modularity, and formal verification. It uses capability-based security and runs all drivers in user space for maximum isolation.

## Essential Commands

### Building the Kernel

#### Automated Build Script (Recommended)

```bash
# Build all architectures (dev mode)
./build-kernel.sh all dev

# Build all architectures (release mode)
./build-kernel.sh all release

# Build specific architecture
./build-kernel.sh x86_64 dev
./build-kernel.sh aarch64 release
./build-kernel.sh riscv64 dev
```

#### Manual Build Commands

```bash
# x86_64 with kernel code model (required for relocation fix)
cargo build --target targets/x86_64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc

# Standard bare metal targets for other architectures
cargo build --target aarch64-unknown-none -p veridian-kernel
cargo build --target riscv64gc-unknown-none-elf -p veridian-kernel
```

#### Important Notes

- x86_64 requires custom target JSON with kernel code model to avoid R_X86_64_32S relocation errors
- Kernel is linked at 0xFFFFFFFF80100000 (top 2GB of virtual memory)
- AArch64 and RISC-V use standard bare metal targets

### Running in QEMU (VERIFIED — Read This Carefully!)

**QEMU version: 10.2** — stricter about drive conflicts than older versions.

#### x86_64 (UEFI boot — requires OVMF + disk image)

x86_64 uses UEFI boot via bootloader 0.11+. It **CANNOT** use `-kernel` flag directly.
You MUST first build the UEFI disk image, then boot from it.

```bash
# Step 1: Build (creates UEFI disk image automatically)
./build-kernel.sh x86_64 dev

# Step 2: Run with OVMF firmware + disk image
qemu-system-x86_64 \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -serial stdio -display none -m 256M

# With debug exit device (for test scripts):
qemu-system-x86_64 \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    -serial stdio -display none -m 256M

# With GDB debugging (paused, connect on :1234):
qemu-system-x86_64 \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -serial stdio -display none -m 256M -s -S
```

**⚠️ COMMON x86_64 MISTAKES (QEMU 10.2):**
- **DO NOT** use `-kernel target/.../veridian-kernel` — fails with "Error loading uncompressed kernel without PVH ELF Note"
- **DO NOT** use `-bios` instead of `-drive if=pflash` — different semantics
- **DO NOT** pass positional arguments after flags — QEMU 10.2 treats them as implicit drives, causing "drive with bus=0, unit=0 (index=0) exists"
- **DO NOT** use `-cdrom` alongside `-drive` on the same bus/index
- **DO NOT** use `cargo run` for x86_64 — the runner in .cargo/config.toml is `bootimage runner` which is not the correct flow

#### AArch64 (direct kernel boot)

AArch64 boots directly with `-kernel` flag. No disk image or firmware needed.

```bash
# Step 1: Build
./build-kernel.sh aarch64 dev

# Step 2: Run
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \
    -kernel target/aarch64-unknown-none/debug/veridian-kernel \
    -serial stdio -display none

# With GDB debugging:
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \
    -kernel target/aarch64-unknown-none/debug/veridian-kernel \
    -serial stdio -display none -s -S
```

#### RISC-V 64 (direct kernel boot with OpenSBI)

RISC-V boots with OpenSBI firmware (provided by QEMU) + kernel.

```bash
# Step 1: Build
./build-kernel.sh riscv64 dev

# Step 2: Run (OpenSBI loads automatically with -bios default)
qemu-system-riscv64 -M virt -m 256M -bios default \
    -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel \
    -serial stdio -display none

# With GDB debugging:
qemu-system-riscv64 -M virt -m 256M -bios default \
    -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel \
    -serial stdio -display none -s -S
```

#### Expected Output (All Architectures)

All three architectures should boot through 6 stages and print:
```
BOOTOK
```
followed by 27/27 tests passing and a second `BOOTOK` after Stage 6 (user space transition).
x86_64 additionally shows "Attempting user-mode entry..." (Ring 3 via SYSCALL/SYSRET).

#### Quick Reference Table

| Arch | Boot Method | Firmware | Kernel Flag | Image |
|------|------------|----------|-------------|-------|
| x86_64 | UEFI disk | OVMF.4m.fd via `-drive if=pflash` | N/A (use `-drive format=raw,file=...uefi.img`) | `target/x86_64-veridian/debug/veridian-uefi.img` |
| AArch64 | Direct | None | `-kernel` | `target/aarch64-unknown-none/debug/veridian-kernel` |
| RISC-V | OpenSBI | `-bios default` | `-kernel` | `target/riscv64gc-unknown-none-elf/debug/veridian-kernel` |

### Testing

```bash
# IMPORTANT: Automated tests currently blocked by Rust toolchain limitation
# See docs/TESTING-STATUS.md for full explanation

# Manual kernel testing — use the QEMU commands above (NOT cargo run)

# Format and lint checks (always run these)
cargo fmt --all
cargo clippy --target x86_64-unknown-none -p veridian-kernel -- -D warnings
cargo clippy --target aarch64-unknown-none -p veridian-kernel -- -D warnings
cargo clippy --target riscv64gc-unknown-none-elf -p veridian-kernel -- -D warnings

# Run specific test
cargo test test_name

# Run integration tests
cargo test --test '*'

# Benchmark tests
cargo bench
```

### Development Tools

```bash
# Install required nightly toolchain
rustup toolchain install nightly-2025-01-15
rustup component add rust-src llvm-tools-preview

# Essential development tools
cargo install bootimage cargo-xbuild cargo-watch cargo-expand cargo-audit cargo-nextest
```

## Architecture Overview

### Microkernel Design

- **Core Services Only**: Memory management, scheduling, IPC, and basic hardware abstraction in kernel
- **User-Space Drivers**: All drivers run in isolated user space processes
- **Capability-Based Security**: Every resource access requires an unforgeable capability token
- **Zero-Copy IPC**: Data shared through memory mapping, not copying

### Memory Layout (x86_64)

```ascii
User Space:   0x0000_0000_0000_0000 - 0x0000_7FFF_FFFF_FFFF (128 TB)
Kernel Space: 0xFFFF_8000_0000_0000 - 0xFFFF_FFFF_FFFF_FFFF (128 TB)
  - Physical memory mapping: 0xFFFF_8000_0000_0000
  - Kernel heap:            0xFFFF_C000_0000_0000
  - Kernel stacks:          0xFFFF_E000_0000_0000
  - Memory-mapped I/O:      0xFFFF_F000_0000_0000
```

### Project Structure

```ascii
veridian-os/
├── kernel/
│   ├── src/
│   │   ├── arch/         # Architecture-specific (x86_64, aarch64, riscv64)
│   │   ├── mm/           # Memory management (frame allocator, page tables)
│   │   ├── sched/        # Scheduler (round-robin, priority-based)
│   │   ├── cap/          # Capability system implementation
│   │   └── ipc/          # Inter-process communication
├── drivers/              # User-space driver processes
├── services/             # System services (VFS, network stack)
├── userland/             # User applications and libraries
├── tools/                # Build tools and utilities
├── debug/                # Debug logs and scripts (gitignored)
│   ├── *.log            # Serial output, QEMU logs, build logs
│   ├── kernel-debug.sh  # Kernel debugging helper
│   ├── gdb-kernel.sh    # GDB debugging script
│   └── kernel.gdb       # GDB initialization commands
```

## Key Development Patterns

### Custom Target Configuration

Each architecture requires a custom target JSON file with specific settings:

- Panic strategy: "abort" (no unwinding in kernel)
- Disable red zone for interrupt safety
- Soft float for kernel code
- No standard library dependencies

### Workspace Organization

- Use workspace for managing multiple crates
- Shared dependencies in workspace Cargo.toml
- Profile settings: panic = "abort" for both dev and release

### Testing Strategy

- **Unit Tests**: Colocated with implementation using `#[cfg(test)]`
- **Integration Tests**: In `tests/` directory for each crate
- **System Tests**: QEMU-based testing for full kernel functionality
- **Property Testing**: Use `proptest` for complex invariants

### Security Considerations

- Minimize `unsafe` blocks - formal verification required for any unsafe code
- All system calls go through capability validation
- Hardware security features: Support for Intel TDX, AMD SEV-SNP, ARM CCA
- Post-quantum ready: Designed for ML-KEM and ML-DSA algorithms

## Development Phases

Currently implementing in phases:

1. **Phase 0** (Months 1-3): Foundation and tooling - **COMPLETE! ✅**
2. **Phase 1** (Months 4-9): Microkernel core - **COMPLETE! ✅**
3. **Phase 2** (Months 10-15): User space foundation - **100% COMPLETE** ✅ (v0.3.2)
4. **Phase 3** (Months 16-21): Security hardening - **100% COMPLETE** ✅ (v0.3.2)
5. **Phase 4** (Months 22-27): Package ecosystem - **100% COMPLETE** ✅ (v0.4.0)
6. **Phase 5** (Months 28-33): Performance optimization - **~10% actual** (data structures only)
7. **Phase 6** (Months 34-42): Advanced features and GUI - **~5% actual** (type definitions only)

## Project Status

| Area | Status |
|------|--------|
| **Repository** | <https://github.com/doublegate/VeridianOS> |
| **Latest Release** | v0.4.0 (February 15, 2026) - Phase 4 Milestone: 100% Complete with Syscall API Docs + 27/27 Tests |
| **Build** | ✅ All 3 architectures compile, zero warnings |
| **Boot** | ✅ All 3 architectures Stage 6 BOOTOK, 27/27 tests |
| **CI/CD** | ✅ GitHub Actions 100% pass rate |
| **Documentation** | ✅ 25+ guides, GitHub Pages, mdBook, Rustdoc |

**Previous Releases**: v0.3.9, v0.3.8, v0.3.7, v0.3.6, v0.3.5, v0.3.4, v0.3.3, v0.3.2, v0.3.1, v0.3.0, v0.2.5, v0.2.1, v0.2.0, v0.1.0

## Implementation Status

### Phase 1 (100% COMPLETE! 🎉)

| Subsystem | Key Features |
|-----------|-------------|
| **Memory Management** | Hybrid bitmap+buddy allocator, NUMA-aware, 4-level page tables, slab heap, zone management |
| **IPC System** | Sync/async channels, zero-copy, fast path <1μs, capability passing, O(1) registry |
| **Process Management** | PCB/TCB, context switching (all archs), synchronization primitives, TLS, NUMA |
| **Capability System** | 64-bit tokens, two-level O(1) lookup, rights management, IPC+syscall integration |
| **Scheduler** | Round-robin, load balancing, metrics tracking, idle task, CPU affinity |

### Driver Framework

- Drivers run as separate user processes
- Hardware access only through capability-controlled MMIO regions
- Interrupt forwarding from kernel to user-space drivers
- DMA buffer management with IOMMU protection

### Technical Decisions

| Decision | Choice |
|----------|--------|
| Language | Rust-only for memory safety |
| Architecture | Microkernel with user-space drivers |
| Security | Capability-based access control |
| Platforms | x86_64, AArch64, RISC-V |
| Memory | Zero-copy IPC with shared memory |
| Crypto | Post-quantum ready (ML-KEM, ML-DSA) |

### Network Stack Architecture (Future)
- lwIP integration initially, custom Rust stack later
- User-space networking with kernel bypass, DPDK, eBPF

## Common Development Tasks

### Adding a New System Call

1. Define capability requirements in `kernel/src/cap/`
2. Add system call handler in `kernel/src/syscall/`
3. Create user-space wrapper in `userland/libs/libveridian/`
4. Add tests in both kernel and user space

### Creating a New Driver

1. Create new crate in `drivers/` directory
2. Implement driver trait from `drivers/common/`
3. Register with driver manager service
4. Add capability definitions for hardware access

### Debugging Kernel Panics

- Use QEMU with `-s -S` flags for GDB debugging (server on :1234, start paused)
- Enable verbose logging with `RUST_LOG=trace`
- Check serial output for panic messages
- Use `addr2line` for stack trace analysis
- Run `just debug-<arch>` for automated GDB sessions
- Use `gdb-multiarch` for cross-architecture debugging
- GDB scripts in `scripts/gdb/` with custom commands (kernel-symbols, break-panic, examine-stack, walk-page-table)
- Documentation: `docs/GDB-DEBUGGING.md`

### Debug Directory

The `debug/` directory contains debugging tools and logs (gitignored):

```bash
./debug/kernel-debug.sh x86_64 60   # Run kernel with debug output
./debug/gdb-kernel.sh               # Start GDB debugging session
./debug/clean-logs.sh 7             # Clean logs older than 7 days
```

### AArch64-Specific Notes

- Iterator-based code causes hangs on bare metal - use safe_iter.rs utilities instead
- Working workarounds in `kernel/src/arch/aarch64/safe_iter.rs`
- UART at 0x09000000 for QEMU virt machine
- Stack at 0x80000 works reliably
- Use `aarch64_for!` macro for safe iteration when needed

## TODO System

Task tracking in `to-dos/`: MASTER_TODO.md, PHASE[0-6]_TODO.md, TESTING_TODO.md, ISSUES_TODO.md, RELEASE_TODO.md

## VeridianOS-Specific Development Patterns

### Build System Configuration

- **Current**: Use standard bare metal targets (x86_64-unknown-none, aarch64-unknown-none, riscv64gc-unknown-none-elf)
- **Legacy**: Custom target JSONs in `targets/` directory (preserved but not used)
- **Build Dependencies**: -Zbuild-std automatically handled by .cargo/config.toml
- Workspace structure with kernel as main crate
- Cargo.lock committed for reproducible builds
- Feature flags for conditional compilation:
  - `alloc` feature for heap-dependent code
  - Use `#[cfg(feature = "alloc")]` for optional allocator support
  - Conditional imports: `#[cfg(feature = "alloc")] use alloc::vec::Vec;`
  - Use extern crate alloc when needed

### IPC Development Patterns

| Pattern | Details |
|---------|---------|
| Message types | SmallMessage (<=64 bytes) fast path, LargeMessage for bulk |
| Fast path | Register-based transfer, <1μs latency |
| Architecture | Separate register mappings per arch |
| Capability tokens | 64-bit with generation counter for revocation |
| Zero-copy | SharedRegion with page remapping |
| Registry | Global O(1) lookup, lock-free async ring buffers |
| Rate limiting | Token bucket algorithm for DoS protection |
| NUMA | Built-in awareness from the start |
| Error handling | Result with IpcError, detailed typed errors |
| Type aliases | ProcessId = u64; import from `super::error::Result` |
| API migration | Restore tests when refactoring; consistent parameter order (id, owner, capacity) |

### Architecture-Specific Details

- **x86_64**: Uses bootloader crate, VGA text output, GDT/IDT setup
- **AArch64**: Custom boot sequence, PL011 UART at 0x09000000, stack at 0x80000
- **RISC-V**: OpenSBI integration, UART at 0x10000000

### CI/CD Configuration

- GitHub Actions with job consolidation, cargo caching, RUSTFLAGS="-D warnings"
- Security audit with rustsec/audit-check action
- Custom targets need -Zbuild-std; cancel-in-progress to prevent duplicate runs
- CI artifacts: `gh run download <run-id> --dir <dir>`, `gh release upload <tag> <files...> --clobber`
- AArch64: prefix unused vars with underscore (println! is no-op on non-x86_64)

### Clippy Fix Patterns

| Pattern | Fix |
|---------|-----|
| new_without_default | Add Default impl |
| manual_flatten | Use iter().flatten() |
| Unused vars on non-x86_64 | `#[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))]` |
| Unused imports in public APIs | `#[allow(unused_imports)]` for re-exports |
| Empty loop | Replace `loop {}` with `panic!("message")` |

### Development Workflow in Distrobox

- Working directory: `/var/home/parobek/Code/VeridianOS`
- User memory location: `/home/parobek/.claude/CLAUDE.md` (global user memory)
- Install git and gh in Ubuntu containers
- Use project-local paths for all file operations

### Key Design Documents

- `docs/design/MEMORY-ALLOCATOR-DESIGN.md` - Memory allocator implementation guide
- `docs/design/IPC-DESIGN.md` - IPC system architecture
- `docs/design/SCHEDULER-DESIGN.md` - Scheduler implementation
- `docs/design/CAPABILITY-SYSTEM-DESIGN.md` - Capability system design
- `docs/PHASE0-COMPLETION-SUMMARY.md` - Phase 0 achievements
- `docs/PHASE1-COMPLETION-CHECKLIST.md` - Phase 1 task tracking

### Performance Targets

| Metric | Target | Achieved |
|--------|--------|----------|
| IPC (small) | < 1μs | ✅ <1μs |
| IPC (large) | < 5μs | ✅ |
| Context switch | < 10μs | ✅ |
| Memory alloc | < 1μs | ✅ |
| Capability lookup | O(1) | ✅ |
| Process support | 1000+ | ✅ |
| Kernel size | < 15K LOC | ✅ ~15K |

### Documentation Organization

- **GitHub Pages**: <https://doublegate.github.io/VeridianOS/>
- **mdBook Source**: `docs/book/src/` directory
- **Building**: Run `mdbook build` in `docs/book/` directory

### Key Technical Patterns Learned

| Pattern | Details |
|---------|---------|
| R_X86_64_32S relocation | Kernel must be in top 2GB for +/-2GB addressing |
| Kernel code model | Required for x86_64 higher-half kernels |
| PIC initialization | Must mask interrupts during init to prevent double faults |
| Static heap | Use static arrays in kernel binary, not arbitrary addresses |
| Testing limitation | no_std kernel testing blocked by duplicate lang items |
| Build targets | Standard targets more compatible than custom JSON specs |
| OnceLock soundness | set() error path must extract value before dropping Box |
| Global allocate-once | process_compat allocate-once-and-reuse prevents leaks |
| PlatformTimer | Cross-arch timer abstraction in `kernel/src/arch/timer.rs` |
| Memory barriers | `arch/barriers.rs`: memory_fence(), data_sync_barrier(), instruction_sync_barrier() |
| #[must_use] on errors | Catches ignored KernelError at compile time |
| Static mut justification | 7 justified remain (early boot, per-CPU, heap) -- document with SAFETY |

### Key Implementation Files

| Path | Purpose |
|------|---------|
| `kernel/src/arch/` | Architecture-specific (aarch64/direct_uart.rs, safe_iter.rs) |
| `kernel/src/mm/` | Memory management (hybrid allocator, VMM, VAS) |
| `kernel/src/ipc/` | IPC (fast path <1μs) |
| `kernel/src/process/` | Process management (full lifecycle) |
| `kernel/src/sched/` | Scheduler (CFS, SMP, load balancing) |
| `kernel/src/cap/` | Capability system (inheritance, revocation, cache) |
| `kernel/src/syscall/` | System call interface |
| `kernel/src/arch/timer.rs` | PlatformTimer trait |
| `kernel/src/arch/barriers.rs` | Memory barrier abstractions |
| `kernel/src/test_framework.rs` | No-std test infrastructure |
| `kernel/src/perf/mod.rs` | Performance counters (AtomicU64) |
| `docs/DEFERRED-IMPLEMENTATION-ITEMS.md` | Deferred item tracking |

## Memory Allocator Design

| Component | Details |
|-----------|---------|
| Hybrid allocator | Bitmap (<512 frames) + buddy (large), threshold-based switching |
| NUMA | Per-node allocators for locality |
| Zones | DMA (0-16MB), Normal, High |
| Page tables | 4-level (x86_64/AArch64), Sv48 (RISC-V) |
| TLB | Shootdown required for cross-CPU updates |
| Slab | For kernel objects with cache awareness |
| Reserved memory | Track regions with overlap checking, filter allocations |
| Feature gating | cfg(feature = "alloc") for allocator-dependent code |

## Process Management Design

| Component | Details |
|-----------|---------|
| PCB | Atomic state management, thread-safe operations |
| Thread context | Architecture-independent trait for context switching |
| Process table | Global O(1) lookup via BTreeMap |
| Context switch | Save/restore all archs with FPU handling |
| Synchronization | Mutex, Semaphore, CondVar, RwLock, Barrier |
| Lifecycle | fork(), exec(), exit(), wait() with resource cleanup |
| Thread mgmt | TLS, CPU affinity, stack management, guard pages |
| Error handling | Typed KernelError variants (no &str errors remain) |
| Priority | Mapping between syscall and internal scheduler priorities |

## Architecture History (Key Decisions)

**Bootstrap refactoring** (Aug 2025): Simplified bootstrap.rs to ~150 lines; each arch has own entry.rs, bootstrap.rs, serial.rs. Print macros unified in print.rs.

**Static mut evolution**: Raw pointer pattern with Box::leak (Aug 2025) --> **SUPERSEDED** by Rust 2024 GlobalState pattern (Nov 2025). See current pattern below.

**v0.3.1 tech debt** (Feb 2026): OnceLock use-after-free fix, 48 static mut eliminated, 8 panic paths removed, 150+ functions migrated to typed errors, PlatformTimer trait + memory barrier abstractions.

### Rust 2024 Safe Global State Pattern - CURRENT
**RECOMMENDED PATTERN**: Complete elimination of `static mut` for Rust 2024 compatibility.

#### GlobalState Pattern (Most Common)
```rust
use crate::sync::once_lock::GlobalState;

// OLD (unsafe, deprecated)
static mut MANAGER: Option<Manager> = None;

pub fn init() -> Result<(), Error> {
    unsafe { MANAGER = Some(Manager::new()); }
    Ok(())
}

pub fn get() -> &'static mut Manager {
    unsafe { MANAGER.as_mut().unwrap() }
}

// NEW (safe, Rust 2024 compatible)
static MANAGER: GlobalState<Manager> = GlobalState::new();

pub fn init() -> Result<(), Error> {
    MANAGER.init(Manager::new())
        .map_err(|_| Error::AlreadyInitialized)?;
    Ok(())
}

pub fn with_manager<R, F: FnOnce(&Manager) -> R>(f: F) -> Option<R> {
    MANAGER.with(f)
}
```

#### GlobalState with Interior Mutability
For modules requiring mutation, wrap in `RwLock`:
```rust
static MANAGER: GlobalState<RwLock<Manager>> = GlobalState::new();

pub fn init() -> Result<(), Error> {
    MANAGER.init(RwLock::new(Manager::new()))
        .map_err(|_| Error::AlreadyInitialized)?;
    Ok(())
}

pub fn with_manager_mut<R, F: FnOnce(&mut Manager) -> R>(f: F) -> Option<R> {
    MANAGER.with(|lock| {
        let mut manager = lock.write();
        f(&mut manager)
    })
}
```

#### Benefits
- **Zero unsafe code** for global state
- **Compile-time initialization checks**
- **No data races** - enforced by type system
- **Rust 2024 edition compatible**
- **Zero performance overhead** - same as previous patterns

#### Modules Converted (120+ static mut eliminated)
**Initial conversion** (88): VFS, IPC Registry, Process Server, Shell, Thread API, Init System, Driver Framework, Package Manager, Security Services

**Rust 2024 migration** (30+): PTY, Terminal, Text Editor, File Manager, GPU, Wayland, Compositor, Window Manager

**v0.3.1 additional conversion** (48): Security (audit, mac, boot, auth, memory_protection, crypto), Network (device, socket, ip, dma_pool), Scheduler (numa), Drivers (pci, console, gpu, network, storage, usb), Services (process_server, driver_framework, init_system, shell), Graphics (framebuffer), Desktop (font), Package (mod), Crypto (random, keystore), IPC (rpc), stdlib, thread_api, fs, test_framework, simple_alloc_unsafe

**Patterns used**: OnceLock, spin::Mutex, AtomicU64/Usize/Bool/I32/U32

#### Justified Remaining static mut (7 instances)
These remain with documented SAFETY justifications:
- `PER_CPU_DATA` (sched/smp.rs) - Per-CPU data requires direct pointer access
- `READY_QUEUE_STATIC` (sched/queue.rs) - Scheduler hot path, lock-free access required
- `HEAP_MEMORY` (mm/heap.rs) - Backing storage for heap allocator itself
- `BOOT_INFO` (arch/x86_64/boot.rs) - Set once during early boot before any concurrency
- `EARLY_SERIAL` (arch/x86_64/early_serial.rs) - Pre-allocator serial output
- `KERNEL_STACK`/`STACK` (arch/x86_64/gdt.rs) - GDT circular references, early boot infrastructure

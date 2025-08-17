# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 🔒 RULE #1 - ALWAYS CHECK AND READ BEFORE WRITING

**CRITICAL**: ALWAYS check for existence and read from existing files BEFORE attempting to update or write to them. This prevents file corruption, data loss, and wasted time.

**Implementation Protocol**:

1. **Check Existence**: Use `Read`, `Glob`, or `LS` tools to verify file exists
2. **Read Current Content**: Always read the existing file content first
3. **Analyze Structure**: Understand current format, sections, and organization
4. **Plan Changes**: Determine what needs to be modified, added, or updated
5. **Execute Update**: Use `Edit` or `MultiEdit` for modifications, or `Write` only after reading

**Tools Usage Priority**:

- ✅ **ALWAYS**: `Read` → `Edit`/`MultiEdit` (for existing files)
- ✅ **CONDITIONAL**: `Read` → `Write` (only if complete rewrite needed after reading)
- ❌ **NEVER**: Direct `Write` without prior `Read` (unless explicitly creating new file)

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

# Run with QEMU (x86_64)
cargo run --target x86_64-unknown-none -p veridian-kernel -- -serial stdio -display none

# Run other architectures
qemu-system-aarch64 -M virt -cpu cortex-a57 -kernel target/aarch64-unknown-none/debug/veridian-kernel -serial stdio -display none
qemu-system-riscv64 -M virt -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel -serial stdio -display none
```

#### Important Notes

- x86_64 requires custom target JSON with kernel code model to avoid R_X86_64_32S relocation errors
- Kernel is linked at 0xFFFFFFFF80100000 (top 2GB of virtual memory)
- AArch64 and RISC-V use standard bare metal targets

### Testing

```bash
# IMPORTANT: Automated tests currently blocked by Rust toolchain limitation
# See docs/TESTING-STATUS.md for full explanation

# Manual test running (individual tests only)
cargo test --test basic_boot --target x86_64-unknown-none --no-run
./kernel/run-tests.sh  # Individual test runner script

# Manual kernel testing
cargo run --target x86_64-unknown-none -p veridian-kernel -- -serial stdio -display none

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
2. **Phase 1** (Months 4-9): Microkernel core - **COMPLETE! ✅** 🎉
3. **Phase 2** (Months 10-15): User space foundation - **NEXT**
4. **Phase 3** (Months 16-21): Security hardening
5. **Phase 4** (Months 22-27): Package ecosystem
6. **Phase 5** (Months 28-33): Performance optimization
7. **Phase 6** (Months 34-42): Advanced features and GUI

## Project Status

- **Repository**: <https://github.com/doublegate/VeridianOS>
- **Documentation**: Complete (25+ comprehensive guides) + GitHub Pages deployment
- **Infrastructure**: Directory structure, TODO system, and GitHub setup complete
- **CI/CD**: ✅ GitHub Actions workflow passing all checks (100% success rate)
- **Current Phase**: Phase 2 (User Space Foundation) - Ready to Start!
  - Phase 0 (Foundation) - 100% COMPLETE! ✅ (v0.1.0 - June 7, 2025)
  - Phase 1 (Microkernel Core) - 100% COMPLETE! ✅ (v0.2.0 - June 12, 2025)
  - IPC System: 100% complete - sync/async channels, registry, perf tracking, rate limiting, capability integration
  - Memory Management: 100% complete - frame allocator, VMM, heap, page tables, user space safety
  - Process Management: 100% complete - PCB, threads, context switching, synchronization primitives, syscalls
  - Capability System: 100% complete - inheritance, revocation, per-CPU cache, full integration
  - Scheduler: 100% complete - CFS/priority scheduling, load balancing, SMP support, CPU hotplug
- **Latest Release**: v0.2.1 (June 17, 2025) - Maintenance Release
  - All architectures boot successfully to Stage 6
  - AArch64 LLVM workaround with assembly-only approach
  - Zero warnings and clippy-clean across all platforms
  - Updated all documentation (39 files)
  - GitHub release with all CI artifacts
- **Latest Development**: Bootloader Modernization Complete (August 14, 2025)
  - ✅ **x86_64 BREAKTHROUGH**: Successfully resolved all bootloader issues - now boots to Stage 6 with BOOTOK!
  - ✅ **Bootloader API Migration**: Comprehensive upgrade from 0.9 → 0.11.11 with fallback strategy
  - ✅ **Multi-Architecture Parity**: All three architectures (x86_64, AArch64, RISC-V) fully operational
  - ✅ **MCP Tool Orchestration**: Demonstrated power of specialized sub-agents with full MCP server access
  - ✅ **Phase 2 Ready**: All critical blocking issues resolved, user space foundation development can begin
- **Previous Releases**: 
  - v0.2.0 (June 12, 2025) - Microkernel Core Complete
  - v0.1.0 (June 7, 2025) - Foundation & Tooling
- **Build Status**: ✅ Compiling successfully for all target architectures
- **Boot Status**: ✅ All architectures boot to Stage 6 successfully!
  - x86_64: Full bootstrap sequence working
  - AArch64: Assembly-only workaround for LLVM bugs
  - RISC-V: Complete initialization working
- **Code Quality**: ✅ All format and clippy checks passing with zero warnings
- **Debugging**: ✅ GDB infrastructure operational with custom commands
- **Testing**: ✅ No-std test framework and benchmarks implemented
- **Documentation**: ✅ Rustdoc and mdBook configured with automatic deployment
- **Version Control**: ✅ Git hooks, PR templates, and release automation ready
- **Phase 1 Completion**: June 12, 2025 - All microkernel core components complete!
  - Completed in just 5 days (June 8-12, 2025)!
  - All subsystems 100% implemented with comprehensive tracking
  - Performance targets achieved (IPC <1μs, context switch <10μs)
  - Builds successfully for all architectures with zero warnings
  - Boot issues resolved in v0.2.1 maintenance release

## Implementation Status

### Phase 1 Progress (100% COMPLETE! 🎉)

- **Memory Management**: 100% complete
  - Hybrid frame allocator (bitmap + buddy system)
  - NUMA-aware allocation with per-node allocators
  - Virtual memory manager with 4-level page tables
  - Kernel heap allocator with slab design
  - Reserved memory tracking and zone management
  - Architecture-specific MMU operations (CR3, TTBR0, SATP)

- **IPC System**: 100% complete
  - Synchronous/asynchronous channels with ring buffers
  - Zero-copy transfers with shared memory mappings
  - Fast path IPC with register-based transfer (<1μs achieved)
  - Capability passing with 64-bit tokens
  - Global registry with O(1) lookup
  - Rate limiting and comprehensive error handling

- **Process Management**: 100% complete
  - Process Control Block (PCB) with atomic state management
  - Thread Control Block (TCB) with full ThreadContext trait
  - Context switching for all architectures (x86_64, AArch64, RISC-V)
  - Synchronization primitives (Mutex, Semaphore, CondVar, RwLock, Barrier)
  - Process system calls (fork, exec, exit, wait, getpid, thread operations)
  - Thread-local storage (TLS) implementation
  - CPU affinity and NUMA awareness

- **Capability System**: 100% complete
  - 64-bit packed capability tokens with generation counters
  - Two-level capability space with O(1) lookup performance
  - Rights management (read, write, execute, grant, derive, manage)
  - Object references for memory, process, thread, endpoint objects
  - Basic operations: create, lookup, validate, revoke
  - Full IPC integration with permission validation
  - Memory operation capability checks
  - System call capability enforcement

- **Scheduler**: 100% complete
  - Round-robin scheduling with time slice management
  - Load balancing framework for multi-core systems
  - Comprehensive metrics tracking (context switches, CPU time)
  - Idle task management and CPU affinity support

### Driver Framework

- Drivers run as separate user processes
- Hardware access only through capability-controlled MMIO regions
- Interrupt forwarding from kernel to user-space drivers
- DMA buffer management with IOMMU protection

### Technical Decisions Made
- **Language**: Rust-only implementation for memory safety
- **Architecture**: Microkernel with user-space drivers
- **Security Model**: Capability-based access control
- **Target Platforms**: x86_64, AArch64, and RISC-V
- **Memory Model**: Zero-copy IPC with shared memory
- **Cryptography**: Post-quantum ready with ML-KEM and ML-DSA support

### Network Stack Architecture (Future)
- lwIP integration for initial implementation
- Custom Rust network stack planned for later phases
- User-space networking with kernel bypass
- DPDK support for high-performance networking
- eBPF for programmable packet processing


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

- Use QEMU with `-s -S` flags for GDB debugging
- Enable verbose logging with `RUST_LOG=trace`
- Check serial output for panic messages
- Use `addr2line` for stack trace analysis
- Run `just debug-<arch>` for automated GDB sessions
- Custom GDB commands available for kernel inspection

### Debug Directory

The `debug/` directory contains debugging tools and logs:

- **kernel-debug.sh**: Run kernel with debug output saved to timestamped logs
- **gdb-kernel.sh**: Start GDB debugging session with proper symbols
- **kernel.gdb**: GDB initialization with custom commands
- **clean-logs.sh**: Clean up old debug logs
- All debug logs are saved with timestamps for later analysis
- Directory is gitignored to avoid committing temporary files

Example usage:

```bash
# Run kernel with debug output
./debug/kernel-debug.sh x86_64 60

# Start GDB debugging session
./debug/gdb-kernel.sh

# Clean logs older than 7 days
./debug/clean-logs.sh 7
```


### AArch64-Specific Notes

- Iterator-based code causes hangs on bare metal - use safe_iter.rs utilities instead
- Working workarounds in `kernel/src/arch/aarch64/safe_iter.rs`
- UART at 0x09000000 for QEMU virt machine
- Stack at 0x80000 works reliably
- Use `aarch64_for!` macro for safe iteration when needed

## TODO System

Comprehensive task tracking is maintained in the `to-dos/` directory:

- **MASTER_TODO.md**: Overall project status and quick links
- **PHASE[0-6]_TODO.md**: Detailed tasks for each development phase
- **TESTING_TODO.md**: Testing strategy and test tracking
- **ISSUES_TODO.md**: Bug tracking and known issues
- **RELEASE_TODO.md**: Release planning and version milestones

Check these files regularly to track progress and identify next tasks.

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

- **Message Types**: SmallMessage (≤64 bytes) for fast path, LargeMessage for bulk data
- **Fast Path Design**: Use register-based transfer for <5μs latency
- **Architecture Abstraction**: Separate register mappings for x86_64, AArch64, RISC-V
- **Capability Tokens**: 64-bit format with generation counter for revocation
- **Error Handling**: Use Result with IpcError for all fallible operations
- **Performance Tracking**: CPU timestamp counters for latency measurement
- **Process Integration**: Extension traits for accessing process context
- **Zero-Copy Design**: SharedRegion with page remapping for large transfers
- **Type Aliases**: Use ProcessId = u64 for clarity
- **Result Imports**: Import from error module: `use super::error::Result;`

### Architecture-Specific Details

- **x86_64**: Uses bootloader crate, VGA text output, GDT/IDT setup
- **AArch64**: Custom boot sequence, PL011 UART at 0x09000000, stack at 0x80000
- **RISC-V**: OpenSBI integration, UART at 0x10000000

### CI/CD Configuration

- GitHub Actions with job consolidation for efficiency
- Caching of cargo registry and target directories
- Security audit with rustsec/audit-check action
- RUSTFLAGS="-D warnings" for strict checking

### Development Workflow in Distrobox

- Working directory: `/var/home/parobek/Code/VeridianOS`
- User memory location: `/home/parobek/.claude/CLAUDE.md` (global user memory)
- Install git and gh in Ubuntu containers
- Use project-local paths for all file operations

### GDB Debugging Setup

- Debug scripts: `scripts/debug-<arch>.sh` for each architecture
- GDB configuration files in `scripts/gdb/` directory
- Custom commands for kernel-specific inspection
- Automated QEMU+GDB integration with symbol loading
- Architecture-specific memory examination commands
- String arguments in GDB commands must be quoted
- Use `just debug-<arch>` commands for easy debugging
- Documentation: `docs/GDB-DEBUGGING.md`

### Key Design Documents

- `docs/design/MEMORY-ALLOCATOR-DESIGN.md` - Memory allocator implementation guide
- `docs/design/IPC-DESIGN.md` - IPC system architecture
- `docs/design/SCHEDULER-DESIGN.md` - Scheduler implementation
- `docs/design/CAPABILITY-SYSTEM-DESIGN.md` - Capability system design
- `docs/PHASE0-COMPLETION-SUMMARY.md` - Phase 0 achievements
- `docs/PHASE1-COMPLETION-CHECKLIST.md` - Phase 1 task tracking

### Performance Targets

- **IPC**: < 1μs (small messages), < 5μs (large transfers)
- **Context Switch**: < 10μs
- **Memory Allocation**: < 1μs
- **Capability Lookup**: O(1)
- **Process Support**: 1000+ concurrent processes
- **Kernel Size**: < 15,000 lines of code

### Documentation Organization

- **GitHub Pages**: <https://doublegate.github.io/VeridianOS/>
- **mdBook Source**: `docs/book/src/` directory
- **Building**: Run `mdbook build` in `docs/book/` directory
- **Content Sources**: Integrated from phase docs, design docs, and technical specs
- **TODO System**: Comprehensive task tracking in `to-dos/` directory
  - `MASTER_TODO.md`: Overall project status and quick links
  - `PHASE[0-6]_TODO.md`: Detailed tasks for each development phase
  - `TESTING_TODO.md`: Testing strategy and test tracking
  - `ISSUES_TODO.md`: Bug tracking and known issues
  - `RELEASE_TODO.md`: Release planning and version milestones

### Key Technical Patterns Learned

- **R_X86_64_32S Relocation**: Kernel must be in top 2GB of address space for ±2GB addressing
- **Kernel Code Model**: Required for x86_64 higher-half kernels above 2GB boundary
- **PIC Initialization**: Must mask interrupts during init to prevent double faults
- **Static Heap**: Use static arrays in kernel binary rather than arbitrary addresses
- **Testing Limitations**: no_std kernel testing blocked by duplicate lang items in Rust toolchain
- **Build Target Strategy**: Standard targets more compatible than custom JSON specs
- **Documentation Consolidation**: Single authoritative deferred items document essential
- **API Migration**: Systematic approach needed when changing core APIs
- **Memory Safety**: User-kernel boundary validation critical for security
- **Performance Validation**: All Phase 1 targets met (IPC <1μs, context switch <10μs)
- **Release Automation**: CI artifacts can be downloaded and attached to releases via gh CLI
- **Version Synchronization**: Update version numbers across all documentation consistently

### Key Implementation Files (Phase 1 - 100% Complete!)

- `kernel/src/arch/` - Architecture-specific implementations (100% - all working!)
  - `arch/aarch64/direct_uart.rs` - Assembly-only UART for LLVM workaround
  - `arch/aarch64/safe_iter.rs` - Loop-free utilities for AArch64
- `kernel/src/mm/` - Memory management implementation (100% - hybrid allocator, VMM, VAS)
- `kernel/src/ipc/` - Inter-process communication implementation (100% - fast path <1μs)
- `kernel/src/process/` - Process management implementation (100% - full lifecycle)
- `kernel/src/sched/` - Scheduler implementation (100% - CFS, SMP, load balancing)
- `kernel/src/cap/` - Capability system (100% - inheritance, revocation, cache)
- `kernel/src/syscall/` - System call interface (100% - user-space safety)
- `kernel/src/raii.rs` - RAII patterns for resource management
- `kernel/src/print.rs` - Kernel output macros
- `kernel/src/test_framework.rs` - No-std test infrastructure
- `kernel/src/bench.rs` - Benchmarking framework
- `docs/DEFERRED-IMPLEMENTATION-ITEMS.md` - Comprehensive tracking (1,415 lines)
- `docs/TESTING-STATUS.md` - Testing limitations and alternatives

## OS-Specific CI/CD Patterns

### Custom Target Requirements
- **Custom Targets Need -Zbuild-std**: Custom targets require building std library from source
- **Architecture-Specific Testing**: Run tests conditionally based on target architecture
- **QEMU Integration**: Set up automated testing with architecture emulators
- **Target Spec Validation**: Validate custom target specifications against built-in targets
- **Dead Code Warnings**: Add #[allow(dead_code)] for architecture-specific functions
- **Stable Feature Flags**: Remove feature flags that have been stabilized (like const_mut_refs)
- **Workflow Optimization**: Use environment variables for reusable values (BUILD_STD_FLAGS)
- **Concurrency Control**: Add cancel-in-progress to prevent duplicate runs
- **Documentation Pipeline**: Include mdBook building and rustdoc with custom themes
- **Release Artifacts**: Create comprehensive packages with kernel, symbols, and docs
- **Artifact Download**: Use `gh run download <run-id> --dir <dir>` to fetch CI artifacts
- **Release Asset Upload**: Use `gh release upload <tag> <files...> --clobber` to add artifacts
- **AArch64 Compilation**: Unused variables cause failures with -D warnings, prefix with underscore
- **Cross-Architecture Issues**: println! may be no-op on some targets, causing unused variable warnings

### Clippy Fix Patterns
- **Formatting Issues**: Remove extra blank lines, fix line breaks in println! macros
- **Clippy new_without_default**: Add Default impl for structs with new() methods
- **Clippy manual_flatten**: Use iter().flatten() instead of nested if let Some loops
- **Unused Variables on Non-x86_64**: println! is no-op, use #[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))]
- **Unused Imports in Public APIs**: Use #[allow(unused_imports)] for re-exported types
- **Macro Expression Issues**: Wrap println! in blocks when used in match expressions
- **Empty Loop Warning**: Replace `loop {}` with `panic!("message")` to satisfy clippy
- **Unused Variables**: Prefix with underscore (_var) to indicate intentional non-use

## Microkernel IPC Development Patterns

### IPC Architecture Design
- **Registry Pattern**: Global registry with O(1) lookup for endpoints and channels
- **Lock-Free Async**: Use lock-free ring buffers for async channels
- **Performance Measurement**: Track CPU cycles for latency measurement
- **Rate Limiting**: Token bucket algorithm for DoS protection
- **Zero-Copy Design**: Shared memory regions with page remapping
- **Fast Path Optimization**: Register-based transfer for small messages (≤64 bytes)
- **NUMA Awareness**: Build in NUMA support from the start
- **Message Size Optimization**: Small messages for register transfer, large for memory
- **Architecture Abstractions**: Separate register mappings per architecture
- **Error Handling**: Comprehensive Result<T> types with detailed errors
- **Capability Integration**: 64-bit tokens with generation counters

### IPC API Migration Patterns
- **Test Restoration**: When refactoring, restore tests rather than deleting them
- **API Migration**: When changing APIs, systematically update all tests and benchmarks
- **Method Naming**: Use descriptive names like send_async vs send for clarity
- **Constructor Parameters**: Maintain consistent parameter order (id, owner, capacity)

## Memory Allocator Implementation

### Hybrid Allocator Design
- **Hybrid Allocator**: Combine bitmap (small) and buddy (large) allocators
- **Threshold Selection**: Switch allocators at optimal frame count (e.g., 512 frames)
- **NUMA Support**: Per-node allocators for locality
- **Statistics Tracking**: Track allocation patterns for optimization
- **Feature Gating**: Use cfg(feature = "alloc") for allocator-dependent code
- **Array Initialization**: Use const patterns for non-Copy types in arrays
- **Zone Management**: DMA, Normal, High memory zones
- **Page Table Management**: 4-level for x86_64/AArch64, Sv48 for RISC-V
- **TLB Shootdown**: Required for cross-CPU virtual memory updates
- **Slab Allocator**: For kernel objects with cache awareness
- **Reserved Memory Handling**: Track reserved regions with overlap checking
- **Reserved Region Structure**: Start/end frames with description for debugging
- **Allocation Filtering**: Check allocated frames against reserved list

## Kernel Debugging Infrastructure

### GDB Script Organization
- **Script Structure**: Create scripts/gdb/ with common and arch-specific configs
- **Debug Launch Scripts**: Create executable debug scripts for easy debugging
- **Custom GDB Commands**: Implement architecture-specific memory examination
- **Symbol Loading**: Use kernel-symbols command with architecture parameter
- **Breakpoint Helpers**: Create break-panic, break-main, break-boot commands
- **Memory Inspection**: Implement examine-stack, examine-uart, walk-page-table commands
- **Documentation**: Always document debugging workflows and custom commands
- **QEMU GDB Server**: Use -s -S flags (server on :1234, start paused)
- **Multiarch GDB**: Use gdb-multiarch for cross-architecture debugging
- **String Arguments in GDB**: Quote string arguments to avoid symbol interpretation

## Process Management Implementation

### Process Control Architecture
- **Process Control Block**: Use atomic state management with thread-safe operations
- **Thread Context Trait**: Define architecture-independent interface for context switching
- **Process Table Design**: Global table with O(1) lookup using BTreeMap or fixed array
- **Context Switching**: Implement save/restore for all architectures with proper FPU handling
- **System Call Integration**: Complete syscall interface for process/thread operations
- **Synchronization Primitives**: Mutex, Semaphore, CondVar, RwLock, Barrier implementations
- **Process Lifecycle**: fork(), exec(), exit(), wait() with proper resource cleanup
- **Thread Management**: Thread creation with TLS, CPU affinity, and stack management
- **Error Handling**: Use &'static str for errors during early development, refactor later
- **Feature Gating**: Heavy use of cfg(feature = "alloc") for optional allocator support
- **Unsafe Code Management**: Document all unsafe blocks, minimize scope
- **Static References**: Use unsafe pointer casts for global process/thread access
- **Priority Mapping**: Convert between syscall priorities and internal scheduler priorities
- **Deferred Implementation Tracking**: Document all TODOs and stubs for future work

## Recent Architecture Refactoring (August 13, 2025)

### Bootstrap Simplification
- **Refactored**: Simplified bootstrap.rs from 439 lines to ~150 lines
- **Architecture Separation**: Created dedicated entry.rs and bootstrap.rs for each architecture
- **Module Organization**: Each architecture (x86_64, AArch64, RISC-V) has its own:
  - `entry.rs`: Early initialization and panic handling
  - `bootstrap.rs`: Stage-specific output functions
  - `serial.rs`: Architecture-specific serial implementations
- **Print Consolidation**: Unified print macros in main print.rs module
- **Build Status**: All three architectures compile successfully
- **Boot Testing**: AArch64 and RISC-V boot through Stage 3 successfully

### Gemini CLI Integration
- **Added**: Gemini CLI support for AI-assisted development (commit 0f77faa)
- **GEMINI.md**: Created for AI context about the project
- **GitHub Workflows**: Added automated PR reviews and issue triage
- **Integration**: Enhances development workflow with AI assistance

### x86_64 Bootloader Diagnosis (August 14, 2025)
- **Issue Identified**: x86_64 architecture doesn't boot due to bootloader 0.9 limitations
- **Root Cause**: Bootloader 0.9 cannot handle higher-half kernels (0xffffffff80000000+)
- **Current Status**: AArch64 and RISC-V boot to Stage 6 successfully, x86_64 blocked
- **Solutions Available**: Upgrade to bootloader 0.10+, switch to GRUB/Limine, or custom boot stub
- **Architecture Differences**: x86_64 requires bootloader, AArch64/RISC-V use direct QEMU loading
- **Implementation Added**: VGA/serial debug output, entry_point! macro, symbol conflict fixes

### Phase 2 Architecture-Specific Fixes (August 16, 2025)
- **AArch64 Static Mut Fix**: Resolved hangs by using pointer-based approach with Box::leak pattern
- **Memory Barriers**: DSB SY and ISB instructions required for AArch64 static pointer initialization
- **Services Fixed**: ThreadManager, InitSystem, DriverFramework all use pointer approach
- **RISC-V Status**: Reaches Stage 6 BOOTOK but immediately reboots (timer/interrupt issue)
- **x86_64 Status**: Early boot hang persists, needs further debugging
- **Achievement**: AArch64 100% functional with complete Phase 2 implementation


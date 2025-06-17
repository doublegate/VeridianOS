# Changelog

All notable changes to VeridianOS are documented here. This project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2025-06-17

### 🚀 Maintenance Release: All Architectures Boot to Stage 6

This maintenance release consolidates all critical fixes and confirms that all three architectures can successfully boot to Stage 6.

#### Added
- AArch64 assembly-only approach implementation
  - Created `direct_uart.rs` with pure assembly UART functions
  - Stage progression markers: S1-S6, MM, IPC, PROC, DONE
  - Complete bypass of LLVM loop compilation bugs
- Boot testing verification for all architectures
- Comprehensive documentation of assembly workarounds

#### Fixed
- AArch64 LLVM loop compilation bug (complete workaround)
- All architectures now boot to Stage 6 successfully
- Zero warnings across all architectures maintained
- Documentation reorganized (sessions moved to `docs/archive/sessions/`)

#### Status
- **x86_64**: ✅ Boots to Stage 6, reaches scheduler and bootstrap task
- **RISC-V**: ✅ Boots to Stage 6, reaches idle loop
- **AArch64**: ✅ Boots to Stage 6 with assembly workarounds
- **DEEP-RECOMMENDATIONS**: 9 of 9 items complete (100%)
- **Ready for Phase 2**: All critical blockers resolved

## [Unreleased] - 2025-06-17

### 🎯 DEEP-RECOMMENDATIONS Implementation Complete (9 of 9)

#### Added
- Comprehensive RAII patterns for automatic resource cleanup (TODO #8 ✅)
  - FrameGuard for physical memory management
  - FramesGuard for multiple frame management
  - MappedRegion for virtual memory cleanup
  - CapabilityGuard for automatic capability revocation
  - ProcessResources for complete process cleanup
  - ChannelGuard for IPC channel cleanup
  - ScopeGuard with `defer!` macro support
- Enhanced frame allocator with `allocate_frame_raii()` methods
- Virtual address space with `map_region_raii()` for temporary mappings
- Comprehensive test suite for RAII patterns
- RAII examples demonstrating usage patterns
- Documentation archive organization
  - Created `docs/archive/{book,doc_updates,format,phase_0,phase_1,sessions}`
  - Summary files for essential information
  - Historical documentation preservation

#### Fixed
- Bootstrap module implementation fixing boot sequence circular dependency
- AArch64 calling convention with proper BSS clearing (`&raw const` syntax)
- Unsafe static mutable access replaced with atomic operations
- Capability token generation overflow with atomic compare-exchange
- Comprehensive user pointer validation with page table walking
- Custom test framework bypassing Rust lang_items conflicts
- Error type migration from string literals to KernelError enum
- Clippy warnings for lifetime elision and explicit auto-deref
- Safety documentation for unsafe functions

#### Status
- **8 of 9 DEEP-RECOMMENDATIONS items completed**
- **Phase 2 ready**: All kernel components stable with RAII foundation
- **TODO #9**: Ready to begin user space foundation implementation

## [0.2.0] - 2025-06-12

### 🎆 Phase 1 Complete: Microkernel Core

**Phase 1 is now 100% complete!** This release marks the successful implementation of all core microkernel functionality, achieving all performance targets and establishing a solid foundation for user-space development.

#### Major Milestones Achieved

##### IPC System (100% Complete)
- ✅ Synchronous message passing with ring buffers
- ✅ Fast path IPC with register-based transfer (<1μs latency achieved)
- ✅ Zero-copy shared memory infrastructure
- ✅ Asynchronous channels with lock-free buffers
- ✅ Global channel registry with O(1) lookup
- ✅ Rate limiting with token bucket algorithm
- ✅ Complete IPC-Capability integration (June 11, 2025)

##### Memory Management (100% Complete)
- ✅ Hybrid frame allocator (bitmap + buddy system)
- ✅ Virtual memory manager with 4-level page tables
- ✅ Kernel heap allocator with slab design
- ✅ NUMA-aware allocation support
- ✅ TLB management for all architectures
- ✅ Bootloader memory map integration
- ✅ Virtual Address Space (VAS) cleanup and user-space safety
- ✅ User-kernel memory validation
- ✅ Frame deallocation in VAS::destroy()

##### Process Management (100% Complete)
- ✅ Process Control Block with comprehensive state management
- ✅ Thread management with full ThreadContext trait
- ✅ Context switching for all architectures
- ✅ Synchronization primitives (Mutex, Semaphore, CondVar, RwLock, Barrier)
- ✅ Process system calls (fork, exec, exit, wait, getpid, thread operations)
- ✅ Thread-local storage implementation

##### Scheduler (100% Complete)
- ✅ Round-robin scheduling algorithm
- ✅ Basic priority scheduling support
- ✅ Idle task management
- ✅ Timer setup for all architectures
- ✅ CPU affinity enforcement
- ✅ CFS (Completely Fair Scheduler) implementation
- ✅ SMP support with per-CPU run queues
- ✅ CPU hotplug support
- ✅ Inter-Processor Interrupts (IPI) for all architectures
- ✅ Load balancing with task migration

##### Capability System (100% Complete)
- ✅ 64-bit packed capability tokens
- ✅ Two-level capability space with O(1) lookup
- ✅ Rights management system
- ✅ IPC and memory operation integration
- ✅ Hierarchical capability inheritance with policies
- ✅ Cascading revocation with delegation trees
- ✅ Per-CPU capability cache
- ✅ Full process table integration
- ✅ System call capability enforcement

## [0.1.0] - 2025-06-07

### 🎉 Phase 0 Complete: Foundation & Tooling

**Phase 0 is now 100% complete!** This release marks the successful establishment of all foundational infrastructure for VeridianOS development.

### Major Achievements

#### Infrastructure
- **Build System**: Complete Cargo workspace with custom target specifications
- **CI/CD Pipeline**: GitHub Actions workflow 100% operational
- **Documentation**: 25+ comprehensive technical guides
- **Testing Framework**: No-std test infrastructure with benchmarks
- **Version Control**: Git hooks, PR templates, and release automation

#### Technical Milestones
- **Multi-Architecture Boot**: All three architectures (x86_64, AArch64, RISC-V) boot successfully
- **Serial I/O**: Working debug output on all platforms
- **GDB Debugging**: Full remote debugging support with custom commands
- **Code Quality**: Zero warnings policy enforced with automated checks

#### Architecture Support
| Platform | Build | Boot | Serial | Debug |
|----------|-------|------|--------|-------|
| x86_64   | ✅    | ✅   | ✅     | ✅    |
| AArch64  | ✅    | ✅   | ✅     | ✅    |
| RISC-V   | ✅    | ✅   | ✅     | ✅    |

### Added
- Comprehensive project structure with modular kernel design
- Custom target specifications for bare metal development
- Architecture abstraction layer for platform independence
- VGA text output for x86_64 debugging
- PL011 UART driver for AArch64
- SBI console support for RISC-V
- Automated build system using Justfile
- Integration test framework with QEMU
- Performance benchmarking infrastructure
- Developer documentation with mdBook
- API documentation with rustdoc
- 10+ TODO tracking documents
- GitHub Pages deployment

### Fixed
- **ISSUE-0001**: CI build failures for custom targets (added -Zbuild-std)
- **ISSUE-0002**: RISC-V target missing llvm-abiname field
- **ISSUE-0003**: Incorrect llvm-target specifications
- **ISSUE-0004**: Cargo.lock missing from repository
- **ISSUE-0005**: Clippy warnings and dead code
- **ISSUE-0006**: AArch64 boot sequence hanging
- **ISSUE-0007**: GDB script string quoting errors

### Technical Details

#### Build System
- Rust nightly-2025-01-15 with custom targets
- Requires `-Zbuild-std=core,compiler_builtins,alloc`
- Automated dependency installation
- Cross-compilation support

#### Kernel Features
- Panic handler with serial output
- Global allocator stub
- Architecture-specific entry points
- Modular subsystem organization

#### Development Tools
- GDB scripts for kernel debugging
- QEMU integration for testing
- Code formatting enforcement
- Security vulnerability scanning

### Documentation
All documentation is available in the repository:
- Architecture overview and design principles
- Development setup and build instructions
- API reference structure
- Contributing guidelines
- Testing strategy
- Phase implementation guides
- Troubleshooting guide

### Next: Phase 1
With Phase 0 complete, development moves to Phase 1: Microkernel Core
- Memory management implementation
- Process and thread management
- Inter-process communication
- Capability system
- System call interface

---

### Performance Achievements

| Metric | Target | Achieved | Status |
|--------|--------|----------|--------|
| IPC Latency | <5μs | <1μs | ✅ Exceeded! |
| Context Switch | <10μs | <10μs | ✅ Met |
| Memory Allocation | <1μs | <1μs | ✅ Met |
| Capability Lookup | O(1) | O(1) | ✅ Met |
| Kernel Size | <15K LOC | ~15K LOC | ✅ Met |

### Added in v0.2.0

- Complete IPC implementation with async channels
- Memory management with hybrid frame allocator
- Full process and thread lifecycle management
- CFS scheduler with SMP and CPU hotplug support
- Complete capability system with inheritance and revocation
- System call interface for all kernel operations
- Inter-Processor Interrupts for all architectures
- Per-CPU data structures and schedulers
- NUMA-aware memory allocation
- Comprehensive synchronization primitives
- Thread-local storage implementation
- Virtual Address Space management with safety
- Zero-copy IPC with shared memory regions
- Rate limiting for IPC channels
- Performance metrics and tracking

### Fixed in v0.2.0

- Implemented proper x86_64 syscall entry with naked functions
- Fixed VAS::destroy() to properly free physical frames
- Implemented SMP wake_up_aps() functionality
- Fixed RISC-V IPI implementation using SBI ecalls
- Added missing get_main_thread_id() method
- Fixed IPC shared memory capability creation
- Resolved all clippy warnings across architectures
- Fixed architecture-specific TLB flushing
- Corrected capability system imports
- Added naked_functions feature flag

## [Unreleased]

### Phase 2 Planning

- Init process creation and management
- Shell implementation
- User-space driver framework
- System libraries
- Basic file system support

## Version History

- **0.2.1** (2025-06-17): Maintenance Release - All architectures boot to Stage 6 ✅
- **0.2.0** (2025-06-12): Phase 1 - Microkernel Core ✅
- **0.1.0** (2025-06-07): Phase 0 - Foundation & Tooling ✅
- **0.0.1** (2025-01-06): Initial repository creation

[Unreleased]: https://github.com/doublegate/VeridianOS/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/doublegate/VeridianOS/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/doublegate/VeridianOS/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/doublegate/VeridianOS/releases/tag/v0.1.0
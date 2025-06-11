# Changelog

All notable changes to VeridianOS are documented here. This project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Phase 1 Progress (Current Development)

**Phase 1 Status**: ~65% Complete as of June 11, 2025

#### Major Milestones Achieved

##### IPC System (100% Complete)
- ✅ Synchronous message passing with ring buffers
- ✅ Fast path IPC with register-based transfer (<1μs latency achieved)
- ✅ Zero-copy shared memory infrastructure
- ✅ Asynchronous channels with lock-free buffers
- ✅ Global channel registry with O(1) lookup
- ✅ Rate limiting with token bucket algorithm
- ✅ Complete IPC-Capability integration (June 11, 2025)

##### Memory Management (~95% Complete)
- ✅ Hybrid frame allocator (bitmap + buddy system)
- ✅ Virtual memory manager with 4-level page tables
- ✅ Kernel heap allocator with slab design
- ✅ NUMA-aware allocation support
- ✅ TLB management for all architectures
- ✅ Bootloader memory map integration

##### Process Management (100% Complete)
- ✅ Process Control Block with comprehensive state management
- ✅ Thread management with full ThreadContext trait
- ✅ Context switching for all architectures
- ✅ Synchronization primitives (Mutex, Semaphore, CondVar, RwLock, Barrier)
- ✅ Process system calls (fork, exec, exit, wait, getpid, thread operations)
- ✅ Thread-local storage implementation

##### Scheduler (~35% Complete)
- ✅ Round-robin scheduling algorithm
- ✅ Basic priority scheduling support
- ✅ Idle task management
- ✅ Timer setup for all architectures
- ✅ CPU affinity enforcement

##### Capability System (~45% Complete)
- ✅ 64-bit packed capability tokens
- ✅ Two-level capability space with O(1) lookup
- ✅ Rights management system
- ✅ IPC and memory operation integration
- ✅ Basic inheritance and revocation

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

## Version History

- **0.1.0** (2025-06-07): Phase 0 - Foundation & Tooling ✅
- **0.0.1** (2025-01-06): Initial repository creation

[Unreleased]: https://github.com/doublegate/VeridianOS/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/doublegate/VeridianOS/releases/tag/v0.1.0
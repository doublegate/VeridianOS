# Changelog

All notable changes to VeridianOS will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Phase 1 Progress (Started June 8, 2025) - Updated January 9, 2025

- Phase 1 ~10% overall complete
- IPC implementation ~45% complete
  - ✅ Synchronous message passing with ring buffers
  - ✅ Fast path IPC with register-based transfer (<1μs latency achieved)
  - ✅ Zero-copy shared memory infrastructure
  - ✅ Capability system integration (64-bit tokens)
  - ✅ System call interface for IPC operations
  - ✅ Global channel registry with O(1) lookup
  - ✅ Architecture-specific syscall entry points
  - ✅ Asynchronous channels with lock-free buffers
  - ✅ Performance tracking infrastructure (<1μs average)
  - ✅ Rate limiting with token bucket algorithm
  - ✅ IPC tests and benchmarks restored
- Memory management ~20% complete
  - ✅ Hybrid frame allocator (bitmap + buddy system)
  - ✅ NUMA-aware allocation support
  - ✅ Performance statistics tracking
  - 🔲 Virtual memory manager pending
  - 🔲 Kernel heap allocator pending
- Process management not yet started
- Full capability system not yet started

## [0.1.0] - 2025-06-07

### Phase 0 Completion - Foundation & Tooling 🎉

**Phase 0: Foundation is now 100% complete!** This marks a major milestone in VeridianOS
development. All foundational infrastructure is in place and operational.

### Added

- Initial project structure with complete directory hierarchy
- Comprehensive documentation for all development phases
- Architecture overview and design principles
- API reference documentation structure
- Development and contribution guidelines
- Testing strategy and framework design
- Troubleshooting guide and FAQ
- Project logos and branding assets
- Complete TODO tracking system with 10+ tracking documents
- GitHub repository structure (issues templates, PR templates)
- Project configuration files (.editorconfig, rustfmt.toml, .clippy.toml)
- Cargo workspace configuration with kernel crate
- Custom target specifications for x86_64, aarch64, and riscv64
- Basic kernel module structure with architecture abstractions
- CI/CD pipeline (GitHub Actions) fully operational
- VGA text output for x86_64
- GDT and IDT initialization for x86_64
- Architecture stubs for all supported platforms
- GDB debugging infrastructure with architecture-specific scripts
- Comprehensive debugging documentation and workflows
- Test framework foundation with no_std support
- Documentation framework setup with rustdoc configuration
- Version control hooks and pre-commit checks
- Development tool integrations (VS Code workspace, rust-analyzer config)
- Phase 0 completion with all infrastructure ready for Phase 1

### Fixed

- Clippy warnings for unused imports and dead code (ISSUE-0005) - **RESOLVED 2025-06-06**
  - Removed unused `core::fmt::Write` import in serial.rs
  - Added `#[allow(dead_code)]` attributes to placeholder functions
  - Fixed formatting issues in multiple files to pass `cargo fmt` checks
  - Resolved all clippy warnings across the codebase
  - **CI/CD pipeline now 100% passing all checks!** 🎉
- AArch64 boot sequence issues (ISSUE-0006) - **RESOLVED 2025-06-07**
  - Discovered iterator-based code causes hangs on bare metal AArch64
  - Simplified boot sequence to use direct memory writes
  - Fixed assembly-to-Rust calling convention issues
  - Created working-simple/ directory for known-good implementations
  - AArch64 now successfully boots to kernel_main
- GDB debugging scripts string quoting issues - **RESOLVED 2025-06-07**
  - Fixed "No symbol" errors in architecture-specific GDB scripts
  - Added quotes around architecture strings in break-boot commands
  - All architectures now work with GDB remote debugging

### Documentation

- Phase 0: Foundation and tooling setup guide
- Phase 1: Microkernel core implementation guide
- Phase 2: User space foundation guide
- Phase 3: Security hardening guide
- Phase 4: Package ecosystem guide
- Phase 5: Performance optimization guide
- Phase 6: Advanced features and GUI guide
- Master TODO list and phase-specific TODO documents
- Testing, QA, and release management documentation
- Meeting notes and decision tracking templates

### Project Setup

- Complete project directory structure (kernel/, drivers/, services/, libs/, etc.)
- GitHub repository initialization and remote setup
- Development tool configurations (Justfile, install scripts)
- Version tracking (VERSION file)
- Security policy and contribution guidelines
- MIT and Apache 2.0 dual licensing

### Technical Progress

- Rust toolchain configuration (nightly-2025-01-15)
- Build system using Just with automated commands
- Cargo.lock included for reproducible builds
- Fixed CI workflow to use -Zbuild-std for custom targets
- Fixed RISC-V target specification (added llvm-abiname)
- Fixed llvm-target values for all architectures
- All clippy and format checks passing
- Security audit integrated with rustsec/audit-check
- All CI jobs passing (Quick Checks, Build & Test, Security Audit)
- QEMU testing infrastructure operational
- x86_64 kernel boots successfully with serial I/O
- RISC-V kernel boots successfully with OpenSBI
- AArch64 kernel boots successfully with serial I/O (Fixed 2025-06-07)
- Generic serial port abstraction for all architectures
- Architecture-specific boot sequences implemented
- All three architectures now boot to kernel_main successfully

### Completed

- **Phase 0: Foundation (100% Complete - 2025-06-07)**
  - All development environment setup complete
  - CI/CD pipeline fully operational and passing all checks
  - Custom target specifications working for all architectures
  - Basic kernel structure with modular architecture
  - All architectures booting successfully (x86_64, AArch64, RISC-V)
  - GDB debugging infrastructure operational
  - Test framework foundation established
  - Documentation framework configured
  - Version control hooks and git configuration complete
  - Development tool integrations ready
  - Comprehensive technical documentation created
  - Ready to begin Phase 1: Microkernel Core implementation

## [0.2.0] - TBD

### Planned for Next Release (Phase 1 Completion)

- Complete IPC implementation with async channels
- Memory management with hybrid allocator
- Process and thread management
- Basic scheduler implementation
- Full capability system
- System call interface

### Known Issues

- No driver support yet
- No user space support
- Limited hardware support
- No file system
- No networking

## Versioning Scheme

VeridianOS follows Semantic Versioning:

- **MAJOR** version (X.0.0): Incompatible API changes
- **MINOR** version (0.X.0): Backwards-compatible functionality additions
- **PATCH** version (0.0.X): Backwards-compatible bug fixes

### Pre-1.0 Versioning

While in pre-1.0 development:

- Minor version bumps may include breaking changes
- Patch versions are for bug fixes only
- API stability not guaranteed until 1.0.0

### Version Milestones

- **0.1.0** - Basic microkernel functionality
- **0.2.0** - Process and memory management
- **0.3.0** - IPC and capability system
- **0.4.0** - User space support
- **0.5.0** - Driver framework
- **0.6.0** - File system support
- **0.7.0** - Network stack
- **0.8.0** - Security features
- **0.9.0** - Package management
- **1.0.0** - First stable release

[Unreleased]: https://github.com/doublegate/VeridianOS/compare/main...HEAD

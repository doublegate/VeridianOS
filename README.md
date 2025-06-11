<div align="center">

# VeridianOS

<img src="images/VeridianOS_Logo-Only.png" alt="VeridianOS Logo" width="60%" />

## A next-generation microkernel operating system built with Rust

</div>

[![CI Status](https://github.com/doublegate/VeridianOS/workflows/CI/badge.svg)](https://github.com/doublegate/VeridianOS/actions)
[![Coverage](https://codecov.io/gh/doublegate/VeridianOS/branch/main/graph/badge.svg)](https://codecov.io/gh/doublegate/VeridianOS)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE-MIT)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE-APACHE)
[![Discord](https://img.shields.io/discord/123456789?label=Discord&logo=discord)](https://discord.gg/veridian)

## Overview

VeridianOS is a modern microkernel operating system written entirely in Rust, emphasizing security, modularity, and performance. It features a capability-based security model, zero-copy IPC, and supports multiple architectures with a focus on reliability and performance.

### Key Features

- 🛡️ **Capability-based security** - Unforgeable tokens for all resource access
- 🚀 **Microkernel architecture** - Minimal kernel with services in user space
- 🦀 **Written in Rust** - Memory safety without garbage collection
- ⚡ **High performance** - Lock-free algorithms, zero-copy IPC
- 🔧 **Multi-architecture** - x86_64, AArch64, and RISC-V support
- 🔒 **Security focused** - Mandatory access control, secure boot, hardware security
- 📦 **Modern package management** - Source and binary package support
- 🖥️ **Wayland compositor** - Modern display server with GPU acceleration

## Project Status

### 🎉 Phase 0: Foundation & Tooling (100% Complete! - v0.1.0)

**Released**: June 7, 2025
**Status**: COMPLETE - v0.1.0 Released 🎉

### 🚀 Phase 1: Microkernel Core (In Progress)

**Started**: June 8, 2025
**Status**: ~65% overall progress

**Components**:

- IPC System: 100% complete ✅ (sync/async channels, registry, perf tracking, rate limiting, capability integration done)
- Memory Management: ~95% complete (frame allocator, virtual memory, page tables, bootloader integration complete)
- Process Management: 100% complete ✅ (PCB, threads, context switching, synchronization primitives, syscalls done)
- Scheduler: ~35% complete (round-robin algorithm, idle task, timer setup, SMP support, CPU affinity, IPC integration done)
- Capability System: ~45% complete (tokens, rights, space management, IPC/memory integration done; inheritance/revocation pending)

**Phase 0 Achievements**:

- ✅ Development environment setup and automation
- ✅ CI/CD pipeline (GitHub Actions) - 100% PASSING across all architectures!
- ✅ Custom target specifications for x86_64, AArch64, and RISC-V
- ✅ Basic kernel structure with modular architecture design
- ✅ Code quality enforcement: formatting, linting, zero warnings policy
- ✅ QEMU testing infrastructure with automated debugging
- ✅ Bootloader integration (working on all three architectures!)
- ✅ GDB debugging infrastructure with custom commands
- ✅ Test framework foundation with no_std support
- ✅ Documentation framework (rustdoc + mdBook) fully configured
- ✅ Version control hooks and automated quality checks
- ✅ Development tool integrations (VS Code, rust-analyzer)
- ✅ Comprehensive technical documentation (25+ documents)
- ✅ GitHub Pages documentation deployment
- ✅ Release automation and artifact generation

### Architecture Support Status

| Architecture | Build | Boot | Serial I/O | Status |
|--------------|-------|------|------------|---------|
| x86_64       | ✅    | ✅   | ✅         | **Fully Working** |
| RISC-V 64    | ✅    | ✅   | ✅         | **Fully Working** |
| AArch64      | ✅    | ✅   | ✅         | **Fully Working** (Fixed 2025-06-07) |

## Quick Start

### Prerequisites

- Rust nightly-2025-01-15 or later
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

# Build and run in QEMU
just run

# Or build manually for specific architectures
cargo build --target targets/x86_64-veridian.json \
    -p veridian-kernel \
    -Zbuild-std=core,compiler_builtins,alloc \
    -Zbuild-std-features=compiler-builtins-mem

# Run in QEMU (x86_64)
cargo bootimage --target targets/x86_64-veridian.json
qemu-system-x86_64 \
    -drive format=raw,file=target/x86_64-veridian/debug/bootimage-veridian-kernel.bin \
    -serial stdio \
    -display none

# Run in QEMU (RISC-V)
qemu-system-riscv64 \
    -M virt \
    -nographic \
    -kernel target/riscv64gc-veridian/debug/veridian-kernel
```

For detailed build instructions, see [BUILD-INSTRUCTIONS.md](docs/BUILD-INSTRUCTIONS.md).

## Documentation

- 📖 [Architecture Overview](docs/ARCHITECTURE-OVERVIEW.md) - System design and architecture
- 🛠️ [Development Guide](docs/DEVELOPMENT-GUIDE.md) - Getting started with development
- 📚 [API Reference](docs/API-REFERENCE.md) - System call and library APIs
- 🧪 [Testing Strategy](docs/TESTING-STRATEGY.md) - Testing approach and guidelines
- 🔍 [Troubleshooting](docs/TROUBLESHOOTING.md) - Common issues and solutions

### Implementation Guides

- 🗺️ [Implementation Roadmap](docs/IMPLEMENTATION-ROADMAP.md) - Detailed 42-month development plan
- 🔄 [Software Porting Guide](docs/SOFTWARE-PORTING-GUIDE.md) - How to port Linux software to VeridianOS
- 🔧 [Compiler Toolchain Guide](docs/COMPILER-TOOLCHAIN-GUIDE.md) - Native compiler integration strategy
- ✅ [Phase 0 Completion Checklist](docs/PHASE0-COMPLETION-CHECKLIST.md) - Remaining tasks for foundation phase
- 🚀 [Future Development Insights](docs/FUTURE-DEVELOPMENT-INSIGHTS.md) - AI-assisted analysis and recommendations

### Development Phases

The project follows a phased development approach:

1. [Phase 0: Foundation](docs/00-PHASE-0-FOUNDATION.md) - Build system and tooling
2. [Phase 1: Microkernel Core](docs/01-PHASE-1-MICROKERNEL-CORE.md) - Core kernel functionality
3. [Phase 2: User Space Foundation](docs/02-PHASE-2-USER-SPACE-FOUNDATION.md) - Essential services
4. [Phase 3: Security Hardening](docs/03-PHASE-3-SECURITY-HARDENING.md) - Security features
5. [Phase 4: Package Ecosystem](docs/04-PHASE-4-PACKAGE-ECOSYSTEM.md) - Package management
6. [Phase 5: Performance Optimization](docs/05-PHASE-5-PERFORMANCE-OPTIMIZATION.md) - Performance tuning
7. [Phase 6: Advanced Features](docs/06-PHASE-6-ADVANCED-FEATURES.md) - GUI and advanced features

See [PROJECT-STATUS.md](docs/PROJECT-STATUS.md) for detailed status information and [Master TODO](to-dos/MASTER_TODO.md) for task tracking.

## Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details on:

- Code of Conduct
- Development workflow
- Coding standards
- Pull request process

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

## Performance

VeridianOS targets high-performance scenarios with:

- Sub-microsecond system call latency
- Lock-free data structures in critical paths
- Zero-copy IPC for efficient communication
- NUMA-aware memory allocation
- io_uring for high-performance I/O

### Performance Targets (AI-Enhanced)

**Phase 1 Goals**:

- IPC Latency: < 5μs
- Context Switch: < 10μs
- Microkernel Size: < 15,000 lines of code

**Phase 5 Goals**:

- IPC Latency: < 1μs
- Memory Allocation: < 1μs
- System Call Overhead: < 100ns
- Support for 1000+ concurrent processes

## Security

Security is a fundamental design principle:

- **Capability-based access control** - Fine-grained permissions
- **Secure boot** - Full chain of trust verification
- **Memory safety** - Rust's guarantees + runtime checks
- **Mandatory access control** - SELinux-style policies
- **Hardware security** - TPM, HSM, and TEE integration

## Supported Platforms

### Architectures

- x86_64 (full support)
- AArch64 (full support)
- RISC-V (RV64GC) (experimental)

### Minimum Requirements

- 64-bit CPU with MMU
- 256MB RAM
- 1GB storage

### Recommended Requirements

- Multi-core CPU with virtualization
- 4GB+ RAM
- NVMe storage

## Community

- 💬 [Discord Server](https://discord.gg/24KbHS4C) - Real-time chat
- 📧 [Mailing List](https://lists.veridian-os.org) - Development discussions
- 🐛 [Issue Tracker](https://github.com/doublegate/VeridianOS/issues) - Bug reports and features
- 📝 [Forum](https://forum.veridian-os.org) - Long-form discussions

## License

VeridianOS is dual-licensed under:

- MIT License ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

You may choose either license for your use.

## Acknowledgments

VeridianOS builds upon ideas from many excellent operating systems:

- seL4 - Formal verification and capability systems
- Redox OS - Rust OS development practices
- Fuchsia - Component-based architecture
- FreeBSD - Driver framework inspiration
- Linux - Hardware support reference

## Technical Roadmap (AI-Enhanced)

### Near-term (2025)

- [x] Complete Phase 0 (Foundation) - **DONE 2025-06-07!** ✅
- [ ] Phase 1: Microkernel Core (4-5 months) - **IN PROGRESS**
  - [~] IPC implementation first (< 5μs latency target) - ~45% complete
  - [~] Memory management (hybrid buddy + bitmap) - ~95% complete
  - [x] Process/Thread management - 100% complete
  - [~] Scheduler implementation - ~30% complete (round-robin, CPU affinity working)
  - [ ] Capability system foundation
- [ ] Phase 2: User Space Foundation (5-6 months)
  - [ ] Three-layer POSIX architecture
  - [ ] Init system and shell
  - [ ] Basic driver framework

### Mid-term (2026)

- [ ] Phase 3: Security Hardening (5-6 months)
  - [ ] SELinux policies
  - [ ] Secure boot implementation
  - [ ] Audit framework
- [ ] Phase 4: Package Ecosystem & Self-Hosting (5-6 months)
  - [ ] 15-month self-hosting roadmap
  - [ ] Ports system with 50+ packages
  - [ ] LLVM toolchain priority

### Long-term (2027+)

- [ ] Phase 5: Performance Optimization (5-6 months)
  - [ ] < 1μs IPC latency
  - [ ] Lock-free kernel paths
  - [ ] DPDK networking
- [ ] Phase 6: Advanced Features (8-9 months)
  - [ ] Wayland compositor
  - [ ] Desktop environment
  - [ ] Cloud-native features
  - [ ] Production certifications

---

<div align="center">

<img src="images/VeridianOS_Full-Logo.png" alt="VeridianOS Full Banner" width="60%" />

**Building the future of operating systems, one commit at a time.**

</div>

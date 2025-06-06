# VeridianOS

<p align="center">
  <img src="images/VeridianOS_Logo-Only.png" alt="VeridianOS Logo" width="200">
</p>

<p align="center">
  <strong>A next-generation microkernel operating system built with Rust</strong>
</p>

<p align="center">
  <a href="https://github.com/doublegate/VeridianOS/actions"><img src="https://github.com/doublegate/VeridianOS/workflows/CI/badge.svg" alt="CI Status"></a>
  <a href="https://codecov.io/gh/doublegate/VeridianOS"><img src="https://codecov.io/gh/doublegate/VeridianOS/branch/main/graph/badge.svg" alt="Coverage"></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
  <a href="LICENSE-APACHE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" alt="License: Apache 2.0"></a>
  <a href="https://discord.gg/veridian"><img src="https://img.shields.io/discord/123456789?label=Discord&logo=discord" alt="Discord"></a>
</p>

## Overview

VeridianOS is a modern microkernel operating system written entirely in Rust, emphasizing security, modularity, and performance. It features a capability-based security model, zero-copy IPC, and support for multiple architectures.

### Key Features

- 🛡️ **Capability-based security** - Unforgeable tokens for all resource access
- 🚀 **Microkernel architecture** - Minimal kernel with services in user space
- 🦀 **Written in Rust** - Memory safety without garbage collection
- ⚡ **High performance** - Lock-free algorithms, zero-copy IPC
- 🔧 **Multi-architecture** - x86_64, AArch64, and RISC-V support
- 🔒 **Security focused** - Mandatory access control, secure boot, hardware security
- 📦 **Modern package management** - Source and binary package support
- 🖥️ **Wayland compositor** - Modern display server with GPU acceleration

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
```

For detailed build instructions, see [BUILD-INSTRUCTIONS.md](docs/BUILD-INSTRUCTIONS.md).

## Documentation

- 📖 [Architecture Overview](docs/ARCHITECTURE-OVERVIEW.md) - System design and architecture
- 🛠️ [Development Guide](docs/DEVELOPMENT-GUIDE.md) - Getting started with development
- 📚 [API Reference](docs/API-REFERENCE.md) - System call and library APIs
- 🧪 [Testing Strategy](docs/TESTING-STRATEGY.md) - Testing approach and guidelines
- 🔍 [Troubleshooting](docs/TROUBLESHOOTING.md) - Common issues and solutions

### Development Phases

The project follows a phased development approach:

1. [Phase 0: Foundation](docs/00-PHASE-0-FOUNDATION.md) - Build system and tooling
2. [Phase 1: Microkernel Core](docs/01-PHASE-1-MICROKERNEL-CORE.md) - Core kernel functionality
3. [Phase 2: User Space Foundation](docs/02-PHASE-2-USER-SPACE-FOUNDATION.md) - Essential services
4. [Phase 3: Security Hardening](docs/03-PHASE-3-SECURITY-HARDENING.md) - Security features
5. [Phase 4: Package Ecosystem](docs/04-PHASE-4-PACKAGE-ECOSYSTEM.md) - Package management
6. [Phase 5: Performance Optimization](docs/05-PHASE-5-PERFORMANCE-OPTIMIZATION.md) - Performance tuning
7. [Phase 6: Advanced Features](docs/06-PHASE-6-ADVANCED-FEATURES.md) - GUI and advanced features

## Project Status

**Current Phase**: Documentation Complete, Ready for Phase 0 Implementation

- ✅ Complete project structure created
- ✅ Comprehensive documentation for all phases
- ✅ TODO tracking system established
- ⏳ Phase 0: Foundation and tooling (next step)

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
┌─────────────────────────────────────────────────────────────┐
│                    User Applications                        │
├─────────────────────────────────────────────────────────────┤
│   System Services (VFS, Network, Display, Audio)            │
├─────────────────────────────────────────────────────────────┤
│   User-Space Drivers (Block, Network, GPU, USB)             │
├─────────────────────────────────────────────────────────────┤
│   Microkernel (Memory, Scheduling, IPC, Capabilities)       │
└─────────────────────────────────────────────────────────────┘
```

## Performance

VeridianOS targets high-performance scenarios with:

- Sub-microsecond system call latency
- Lock-free data structures in critical paths
- Zero-copy IPC for efficient communication
- NUMA-aware memory allocation
- io_uring for high-performance I/O

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

- 💬 [Discord Server](https://discord.gg/veridian) - Real-time chat
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

## Roadmap

### Near-term (2025)
- [ ] Complete Phase 0 implementation
- [ ] Basic x86_64 boot and initialization
- [ ] Core memory management
- [ ] Initial IPC implementation

### Mid-term (2026)
- [ ] User-space driver framework
- [ ] Basic POSIX compatibility
- [ ] Network stack
- [ ] Package manager

### Long-term (2027+)
- [ ] GUI desktop environment
- [ ] Cloud-native features
- [ ] Production deployments
- [ ] Security certifications

---

<p align="center">
  <img src="images/VeridianOS_Full-Logo.png" alt="VeridianOS" width="300">
</p>

<p align="center">
  <strong>Building the future of operating systems, one commit at a time.</strong>
</p>

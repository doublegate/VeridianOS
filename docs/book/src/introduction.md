# Introduction

<p align="center">
  <img src="images/VeridianOS_Logo-Only.png" alt="VeridianOS Logo" width="200">
</p>

<p align="center">
  <strong>A next-generation microkernel operating system built with Rust</strong>
</p>

## Welcome to VeridianOS

VeridianOS is a modern microkernel operating system written entirely in Rust, emphasizing security, modularity, and performance. All 13 development phases (0-12) are complete as of v0.25.1, including full KDE Plasma 6 desktop integration cross-compiled from source.

This book serves as the comprehensive guide for understanding, building, and contributing to VeridianOS.

Parts of the book describe the design rather than the current state. What is not yet in effect is
listed in [Known Limitations](https://github.com/doublegate/VeridianOS/blob/main/docs/KNOWN-LIMITATIONS.md).

## Key Features

- **Capability-based security** - 64-bit tokens with generation counters and O(1) lookup, guarding IPC, memory sharing and process creation
- **Microkernel architecture** - Designed for drivers and services in user space; today they still run in the kernel (C6)
- **Written in Rust** - Memory safety without garbage collection, 99%+ SAFETY comment coverage
- **Performance-oriented design** - Per-CPU frame caches, shared-region (zero-copy) IPC, an EEVDF/real-time/deadline scheduling policy
- **Multi-architecture** - x86_64, AArch64 and RISC-V all boot to Stage 6 under QEMU; user programs run on x86_64 only so far
- **Security focused** - Post-quantum crypto (ML-DSA-65 verification per FIPS 204; experimental non-standard Kyber, N-57), NX/W^X page protections, MAC/RBAC model
- **KDE Plasma 6 desktop** - Cross-compiled from source with Qt 6.8.3, KDE Frameworks 6.12.0 (the build scripts now pin Qt 6.12.0, KDE Frameworks 6.30.0 and Plasma 6.7.5; not yet rebuilt)
- **Self-hosting** - Native GCC 14.2, binutils, make, ninja, vpkg toolchain
- **Modern package management** - Source and binary package support
- **153 shell builtins** - Full-featured vsh shell with job control and scripting

## Why VeridianOS?

Traditional monolithic kernels face challenges in security, reliability, and maintainability. VeridianOS addresses these challenges through:

1. **Microkernel Design**: Only essential services run in kernel space, minimizing the attack surface
2. **Capability-Based Security**: Fine-grained access control with capability tokens
3. **Memory Safety**: Rust's ownership system prevents entire classes of vulnerabilities
4. **Modern Architecture**: Designed for contemporary hardware with multi-core, NUMA, and heterogeneous computing support

## Project Philosophy

VeridianOS follows these core principles:

- **Security First**: Every design decision prioritizes security
- **Correctness Over Performance**: We optimize only after proving correctness
- **Modularity**: Components are loosely coupled and independently updatable
- **Transparency**: All development happens in the open with clear documentation

## Current Status

**Version**: v0.25.1 (March 10, 2026) | **All Phases Complete** (0-12)

- 4,095+ tests passing across host-target and kernel boot tests
- 3 architectures booting to Stage 6 BOOTOK with 29/29 tests each
- CI pipeline: 11/11 jobs passing
- Zero clippy warnings across all targets
- KDE Plasma 6 cross-compiled from source (kwin_wayland, plasmashell, dbus-daemon)
- 153 shell builtins, 9 desktop apps, 8 settings panels

See [Project Status](./project/status.md) for detailed metrics and [Roadmap](./project/roadmap.md) for phase completion history.

## What This Book Covers

This book is organized into several sections:

- **Getting Started**: Prerequisites, building, and running VeridianOS
- **Architecture**: Deep dive into the system design and components
- **Development Guide**: How to contribute code and work with the codebase
- **Platform Support**: Architecture-specific implementation details
- **API Reference**: Complete system call and kernel API documentation
- **Design Documents**: Detailed specifications for major subsystems
- **Development Phases**: All 13 phases from foundation to KDE cross-compilation

## Join the Community

VeridianOS is an open-source project welcoming contributions from developers worldwide. Whether you're interested in kernel development, system programming, or just learning about operating systems, there's a place for you in our community.

- **GitHub**: [github.com/doublegate/VeridianOS](https://github.com/doublegate/VeridianOS)
- **Discord**: [discord.gg/WGcgrnuVHt](https://discord.gg/WGcgrnuVHt)
- **Documentation**: [doublegate.github.io/VeridianOS](https://doublegate.github.io/VeridianOS)

## License

VeridianOS is dual-licensed under MIT and Apache 2.0 licenses. See the LICENSE files for details.

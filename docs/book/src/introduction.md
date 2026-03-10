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

## Key Features

- **Capability-based security** - Unforgeable 64-bit tokens for all resource access with O(1) lookup
- **Microkernel architecture** - Minimal kernel with drivers and services in user space
- **Written in Rust** - Memory safety without garbage collection, 99%+ SAFETY comment coverage
- **High performance** - Lock-free algorithms, zero-copy IPC (<1us latency)
- **Multi-architecture** - x86_64, AArch64, and RISC-V support (all boot to Stage 6)
- **Security focused** - Post-quantum crypto (ML-KEM, ML-DSA), KASLR, SMEP/SMAP, MAC/RBAC
- **KDE Plasma 6 desktop** - Cross-compiled from source with Qt 6.8.3, KDE Frameworks 6.12.0
- **Self-hosting** - Native GCC 14.2, binutils, make, ninja, vpkg toolchain
- **Modern package management** - Source and binary package support
- **153 shell builtins** - Full-featured vsh shell with job control and scripting

## Why VeridianOS?

Traditional monolithic kernels face challenges in security, reliability, and maintainability. VeridianOS addresses these challenges through:

1. **Microkernel Design**: Only essential services run in kernel space, minimizing the attack surface
2. **Capability-Based Security**: Fine-grained access control with unforgeable capability tokens
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
- **Discord**: [discord.gg/veridian](https://discord.gg/veridian)
- **Documentation**: [doublegate.github.io/VeridianOS](https://doublegate.github.io/VeridianOS)

## License

VeridianOS is dual-licensed under MIT and Apache 2.0 licenses. See the LICENSE files for details.

# Introduction

<p align="center">
  <img src="../../../images/VeridianOS_Logo-Only.png" alt="VeridianOS Logo" width="200">
</p>

<p align="center">
  <strong>A next-generation microkernel operating system built with Rust</strong>
</p>

## Welcome to VeridianOS

VeridianOS is a modern microkernel operating system written entirely in Rust, emphasizing security, modularity, and performance. This book serves as the comprehensive guide for understanding, building, and contributing to VeridianOS.

## Key Features

- 🛡️ **Capability-based security** - Unforgeable tokens for all resource access
- 🚀 **Microkernel architecture** - Minimal kernel with services in user space
- 🦀 **Written in Rust** - Memory safety without garbage collection
- ⚡ **High performance** - Lock-free algorithms, zero-copy IPC
- 🔧 **Multi-architecture** - x86_64, AArch64, and RISC-V support
- 🔒 **Security focused** - Mandatory access control, secure boot, hardware security
- 📦 **Modern package management** - Source and binary package support
- 🖥️ **Wayland compositor** - Modern display server with GPU acceleration

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

### 🎉 **Phase 0: Foundation & Tooling** (100% Complete! - v0.1.0)

**Released**: June 7, 2025  
**Milestone**: Complete development infrastructure ready for microkernel implementation

- ✅ Development environment setup and automation
- ✅ CI/CD pipeline (GitHub Actions) - 100% PASSING across all architectures!
- ✅ Custom target specifications for x86_64, AArch64, and RISC-V
- ✅ Basic kernel structure with modular architecture design
- ✅ All architectures booting successfully with serial output
- ✅ Comprehensive documentation (25+ technical guides)
- ✅ Release automation with artifact generation

🚀 **Next**: Phase 1 - Microkernel Core Implementation (Memory Management, IPC, Scheduling)

## What This Book Covers

This book is organized into several sections:

- **Getting Started**: Prerequisites, building, and running VeridianOS
- **Architecture**: Deep dive into the system design and components
- **Development Guide**: How to contribute code and work with the codebase
- **Platform Support**: Architecture-specific implementation details
- **API Reference**: Complete system call and kernel API documentation
- **Design Documents**: Detailed specifications for major subsystems
- **Development Phases**: Roadmap and implementation timeline

## Join the Community

VeridianOS is an open-source project welcoming contributions from developers worldwide. Whether you're interested in kernel development, system programming, or just learning about operating systems, there's a place for you in our community.

- **GitHub**: [github.com/doublegate/VeridianOS](https://github.com/doublegate/VeridianOS)
- **Discord**: [discord.gg/veridian](https://discord.gg/veridian)
- **Documentation**: [doublegate.github.io/VeridianOS](https://doublegate.github.io/VeridianOS)

## License

VeridianOS is dual-licensed under MIT and Apache 2.0 licenses. See the LICENSE files for details.

Let's build the future of operating systems together!
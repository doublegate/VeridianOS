# Frequently Asked Questions (FAQ)

## General Questions

### What is VeridianOS?

VeridianOS is a next-generation microkernel operating system written entirely in Rust. It emphasizes security, modularity, and performance through a capability-based security model and modern OS design principles.

### Why another operating system?

VeridianOS addresses several limitations in existing systems:
- **Security**: Capability-based security from the ground up
- **Safety**: Rust's memory safety eliminates entire classes of bugs
- **Modularity**: True microkernel design with isolated services
- **Performance**: Modern algorithms and zero-copy IPC
- **Simplicity**: Clean codebase without decades of legacy

### What makes VeridianOS different?

Key differentiators:
1. Written entirely in Rust (no C except bootloader)
2. Capability-based security model throughout
3. Designed for modern hardware (64-bit only)
4. Native support for virtualization and containers
5. Post-quantum cryptography ready

### What's the project status?

VeridianOS released v0.2.0 on June 12, 2025, marking the completion of Phase 1 (Microkernel Core). The project achieved 100% completion of all core microkernel subsystems including IPC (<1μs latency), memory management, process management, scheduler, and capability system. All foundation infrastructure is operational and the project is ready for Phase 2 (User Space Foundation). See [PROJECT-STATUS.md](PROJECT-STATUS.md) for details.

**Recent Updates (December 2025)**: Fixed x86_64 build issues, improved boot sequence, and created automated build tools.

### When will it be ready for production use?

We're targeting:
- 2025: Core kernel functionality
- 2026: Basic usability with drivers and userspace
- 2027: Production readiness for specific use cases

## Technical Questions

### What architectures are supported?

- **x86_64**: Full support planned
- **AArch64**: Full support planned
- **RISC-V (RV64GC)**: Experimental support

All architectures require 64-bit CPUs with MMU.

### What's a microkernel?

A microkernel runs minimal code in kernel space - only:
- Memory management
- CPU scheduling
- Inter-process communication (IPC)
- Basic security enforcement

Everything else (drivers, filesystems, networking) runs in user space as isolated services.

### What are capabilities?

Capabilities are unforgeable tokens that grant specific permissions to resources. Instead of checking "who is asking" (identity-based), we check "what token do they have" (capability-based). This provides:
- Fine-grained access control
- Easy permission delegation
- No confused deputy problems
- Natural sandboxing

### Why Rust?

Rust provides:
- Memory safety without garbage collection
- Zero-cost abstractions
- Excellent performance
- Strong type system
- Great tooling
- Active community

### Will it run Linux software?

Eventually, yes. We plan:
- POSIX compatibility layer
- Linux syscall emulation
- Container support for Linux apps
- Wine-like translation for binaries

Native VeridianOS apps will have better performance and security.

### How does IPC work?

VeridianOS uses synchronous IPC with:
- Zero-copy message passing
- Capability-based endpoints
- Type-safe interfaces
- Optional async wrappers

### What about drivers?

Drivers run in user space with:
- Memory-mapped I/O access via capabilities
- Interrupt handling via IPC
- DMA buffer management
- Hot-plug support

### Is it POSIX compliant?

Not natively, but we'll provide:
- POSIX compatibility library
- System call translation
- Familiar tools and utilities

Native APIs are capability-based and more secure.

## Development Questions

### How can I contribute?

See [CONTRIBUTING.md](../CONTRIBUTING.md) for details. We need help with:
- Kernel development
- Driver writing
- Documentation
- Testing
- Security auditing

### What do I need to know?

Helpful skills:
- **Essential**: Rust programming
- **Helpful**: OS concepts, computer architecture
- **Bonus**: Security, formal methods, specific hardware

### Where do I start?

1. Read the [Architecture Overview](ARCHITECTURE-OVERVIEW.md)
2. Set up the [Development Environment](DEVELOPMENT-GUIDE.md)
3. Look for "good first issue" tags
4. Join our Discord community

### How do I build it?

```bash
# Clone repository
git clone https://github.com/doublegate/VeridianOS
cd VeridianOS

# Install dependencies
./scripts/install-deps.sh

# Build and run
just run
```

### How do I test changes?

```bash
# Run all tests
just test

# Run specific test
cargo test test_name

# Run in QEMU
just run
```

### What's the code style?

We follow:
- Official Rust style guide
- Clippy lints
- Comprehensive documentation
- See [CONTRIBUTING.md](../CONTRIBUTING.md#coding-standards)

## Community Questions

### How do I get help?

- Discord: [#help channel](https://discord.gg/veridian)
- Mailing list: help@veridian-os.org
- Stack Overflow: tag `veridian-os`
- GitHub Discussions

### How do I report bugs?

1. Check existing issues
2. Create detailed bug report
3. Include reproduction steps
4. See [bug report template](../.github/ISSUE_TEMPLATE/bug_report.md)

### How do I suggest features?

1. Check roadmap and existing issues
2. Discuss on Discord/forums first
3. Create feature request issue
4. See [feature request template](../.github/ISSUE_TEMPLATE/feature_request.md)

### Is there commercial support?

Not yet, but planned for the future. Current support:
- Community support (free)
- Consulting available for sponsors
- Commercial support after 1.0

### Can I use it in my project?

Yes! VeridianOS is dual-licensed under MIT and Apache 2.0. Choose whichever license works for your project.

## Comparison Questions

### How does it compare to Linux?

| Aspect | VeridianOS | Linux |
|--------|------------|-------|
| Architecture | Microkernel | Monolithic |
| Language | Rust | C |
| Security | Capability-based | DAC/MAC |
| Drivers | User space | Kernel space |
| Legacy | None | 30+ years |

### How does it compare to seL4?

Both are capability-based microkernels, but:
- seL4: Formally verified, C code
- VeridianOS: Rust safety, more features

### How does it compare to Redox OS?

Both are Rust-based, but:
- Redox: Unix-like, started earlier
- VeridianOS: Capability-based, different architecture

### Will it replace Linux/Windows/macOS?

Not trying to replace, but to provide:
- Better security for critical systems
- Modern architecture for new applications
- Research platform for OS innovation
- Alternative for specific use cases

## Troubleshooting Questions

### Build fails with "rust-src not found"

Install rust-src component:
```bash
rustup component add rust-src
```

### QEMU won't start

Check:
1. QEMU is installed
2. KVM permissions (Linux)
3. Sufficient RAM
4. See [TROUBLESHOOTING.md](TROUBLESHOOTING.md)

### Tests fail locally

Ensure:
1. Latest Rust nightly
2. All dependencies installed
3. Clean build: `cargo clean`
4. Run: `just test`

### Where are the logs?

- Build logs: `target/`
- QEMU output: Serial console
- Enable debug: `RUST_LOG=debug just run`

## More Questions?

- Check documentation in `docs/`
- Ask on [Discord](https://discord.gg/veridian)
- Email: info@veridian-os.org
- Create a GitHub issue

This FAQ is regularly updated based on community questions!
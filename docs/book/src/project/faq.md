# Frequently Asked Questions

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
1. Written entirely in Rust (no C/C++ in kernel)
2. Capability-based security model throughout
3. Designed for modern hardware (64-bit only)
4. Native support for virtualization and containers
5. Post-quantum cryptography ready
6. Formal verification of critical components

### What's the project status?

VeridianOS has completed Phase 0 (Foundation) as of v0.1.0 (June 2025) and is now starting Phase 1 (Microkernel Core). All foundation infrastructure is in place and development is proceeding to kernel implementation.

### When will it be ready for daily use?

Our timeline targets:
- **2025**: Core kernel functionality (Phase 1)
- **2026**: Basic usability with drivers and userspace (Phase 2-3)
- **2027**: Production readiness for specific use cases (Phase 4-5)
- **2028**: Desktop and general use (Phase 6)

## Technical Questions

### What architectures are supported?

Current support:
- **x86_64**: Full support, primary platform
- **AArch64**: Full support, including Apple Silicon
- **RISC-V (RV64GC)**: Experimental support

All architectures require:
- 64-bit CPUs with MMU
- 4KB page size support
- Atomic operations

### What's a microkernel?

A microkernel runs minimal code in privileged mode:
- Memory management
- CPU scheduling
- Inter-process communication (IPC)
- Capability management

Everything else runs in user space:
- Device drivers
- File systems
- Network stack
- System services

Benefits include better security, reliability, and modularity.

### What are capabilities?

Capabilities are unforgeable tokens that grant specific permissions:
- **Not "who you are"**: No user IDs or access control lists
- **But "what you can do"**: Hold a capability = have permission
- **Composable**: Combine capabilities for complex permissions
- **Revocable**: Invalidate capabilities to revoke access

Example:
```rust
// A capability to read from a file
let read_cap: Capability<FileRead> = file.get_read_capability()?;

// Use the capability
let data = read_cap.read(buffer)?;

// Delegate to another process
other_process.send_capability(read_cap)?;
```

### Why Rust?

Rust provides unique advantages for OS development:
- **Memory Safety**: No buffer overflows, use-after-free, etc.
- **Zero-Cost Abstractions**: High-level code with no overhead
- **No Garbage Collection**: Predictable performance
- **Excellent Tooling**: Cargo, rustfmt, clippy
- **Strong Type System**: Catch bugs at compile time
- **Active Community**: Growing ecosystem

### Will it run Linux applications?

Yes, through multiple compatibility layers:
1. **POSIX Layer**: For portable Unix applications
2. **Linux ABI**: Binary compatibility for Linux executables
3. **Containers**: Run full Linux environments
4. **Wine-like Layer**: For complex applications

Native VeridianOS applications will have better:
- Performance (direct capability use)
- Security (fine-grained permissions)
- Integration (native IPC)

### How fast is the IPC?

Performance targets:
- **Small messages (≤64 bytes)**: < 1μs latency
- **Large transfers**: Zero-copy via shared memory
- **Throughput**: > 1M messages/second
- **Scalability**: Lock-free for multiple cores

### What about real-time support?

VeridianOS will support soft real-time with:
- Priority-based preemptive scheduling
- Bounded interrupt latency
- Reserved CPU cores
- Deadline scheduling (future)

Hard real-time may be added in later phases.

## Development Questions

### How can I contribute?

Many ways to help:
1. **Code**: Pick issues labeled "good first issue"
2. **Documentation**: Improve guides and examples
3. **Testing**: Write tests, report bugs
4. **Ideas**: Suggest features and improvements
5. **Advocacy**: Spread the word

See our [Contributing Guide](../contributing/how-to.md).

### What's the development process?

1. Discussion in GitHub issues
2. Design documents for major features
3. Implementation with tests
4. Code review by maintainers
5. CI/CD validation
6. Merge to main branch

### What languages can I use?

- **Kernel**: Rust only (with minimal assembly)
- **Drivers**: Rust strongly preferred
- **Applications**: Any language with VeridianOS bindings
- **Tools**: Rust, Python, or shell scripts

### How do I set up the development environment?

See our [Development Setup Guide](../getting-started/dev-setup.md). Basic steps:
1. Install Rust nightly
2. Install QEMU
3. Clone repository
4. Run `just build`

### Where can I get help?

- **Documentation**: This book and GitHub docs
- **GitHub Issues**: For bugs and features
- **Discord**: [discord.gg/veridian](https://discord.gg/veridian)
- **Mailing List**: dev@veridian-os.org

## Philosophy Questions

### What are the design principles?

1. **Security First**: Every decision considers security
2. **Simplicity**: Prefer simple, correct solutions
3. **Performance**: But not at the cost of security
4. **Modularity**: Components should be independent
5. **Transparency**: Open development and documentation

### Why capability-based security?

Capabilities solve many security problems:
- **Ambient Authority**: No more confused deputy
- **Least Privilege**: Natural, fine-grained permissions
- **Delegation**: Easy, safe permission sharing
- **Revocation**: Clean permission removal

### Will VeridianOS be free software?

Yes! VeridianOS is dual-licensed under:
- MIT License
- Apache License 2.0

This allows maximum compatibility with other projects.

### What's the long-term vision?

VeridianOS aims to be:
- A secure foundation for critical systems
- A research platform for OS innovation
- A practical alternative to existing systems
- A teaching tool for OS concepts

We believe operating systems can be both secure and usable!
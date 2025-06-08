# Introduction

Welcome to the VeridianOS Developer Guide!

## What is VeridianOS?

VeridianOS is a next-generation microkernel operating system written entirely in Rust. It emphasizes:

- **Security**: Capability-based access control for all resources
- **Reliability**: Memory safety through Rust's type system
- **Performance**: Designed for modern hardware with < 10μs context switches
- **Modularity**: True microkernel with drivers in user space

## Key Features

- **Multi-Architecture Support**: x86_64, AArch64, and RISC-V
- **Zero-Copy IPC**: Efficient message passing between processes
- **Hardware Security**: Support for Intel TDX, AMD SEV-SNP, ARM CCA
- **Formal Verification**: Critical components mathematically verified
- **POSIX Compatibility**: Through optional compatibility layer

## Design Philosophy

VeridianOS follows these core principles:

1. **Minimal Kernel**: Only essential services in kernel space
2. **Capability Security**: Unforgeable tokens for all access control
3. **Fault Isolation**: Drivers and services isolated in user space
4. **Verifiable Design**: Built for formal verification from the start

## Project Status

VeridianOS is currently in active development:

- **Phase 0**: Foundation (100% complete!) ✅
- **Phase 1**: Microkernel Core (Starting now)
- **Phase 2-6**: Future development

## Getting Help

- [GitHub Issues](https://github.com/doublegate/VeridianOS/issues)
- [Contributing Guide](./contributing/how-to.md)
- [Architecture Overview](./architecture/overview.md)

## License

VeridianOS is licensed under the MIT License. See the LICENSE file for details.
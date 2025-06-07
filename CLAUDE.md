# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## VeridianOS Overview

VeridianOS is a next-generation microkernel operating system written entirely in Rust, emphasizing security, modularity, and formal verification. It uses capability-based security and runs all drivers in user space for maximum isolation.

## Essential Commands

### Building the Kernel
```bash
# Build for specific architectures (requires -Zbuild-std for custom targets)
cargo build --target targets/x86_64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem
cargo build --target targets/aarch64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem
cargo build --target targets/riscv64gc-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem

# Build all targets
just build-all

# Generate bootable image
cargo bootimage

# Run with QEMU
cargo run --target targets/x86_64-veridian.json -p veridian-kernel -- -serial stdio -display none
```

### Testing
```bash
# Run all tests
cargo test

# Run tests with coverage
cargo tarpaulin --out Html --output-dir coverage

# Run specific test
cargo test test_name

# Run integration tests
cargo test --test '*'

# Benchmark tests
cargo bench
```

### Development Tools
```bash
# Install required nightly toolchain
rustup toolchain install nightly-2025-01-15
rustup component add rust-src llvm-tools-preview

# Essential development tools
cargo install bootimage cargo-xbuild cargo-watch cargo-expand cargo-audit cargo-nextest
```

## Architecture Overview

### Microkernel Design
- **Core Services Only**: Memory management, scheduling, IPC, and basic hardware abstraction in kernel
- **User-Space Drivers**: All drivers run in isolated user space processes
- **Capability-Based Security**: Every resource access requires an unforgeable capability token
- **Zero-Copy IPC**: Data shared through memory mapping, not copying

### Memory Layout (x86_64)
```
User Space:   0x0000_0000_0000_0000 - 0x0000_7FFF_FFFF_FFFF (128 TB)
Kernel Space: 0xFFFF_8000_0000_0000 - 0xFFFF_FFFF_FFFF_FFFF (128 TB)
  - Physical memory mapping: 0xFFFF_8000_0000_0000
  - Kernel heap:            0xFFFF_C000_0000_0000
  - Kernel stacks:          0xFFFF_E000_0000_0000
  - Memory-mapped I/O:      0xFFFF_F000_0000_0000
```

### Project Structure
```
veridian-os/
├── kernel/
│   ├── src/
│   │   ├── arch/         # Architecture-specific (x86_64, aarch64, riscv64)
│   │   ├── mm/           # Memory management (frame allocator, page tables)
│   │   ├── sched/        # Scheduler (round-robin, priority-based)
│   │   ├── cap/          # Capability system implementation
│   │   └── ipc/          # Inter-process communication
├── drivers/              # User-space driver processes
├── services/             # System services (VFS, network stack)
├── userland/             # User applications and libraries
└── tools/                # Build tools and utilities
```

## Key Development Patterns

### Custom Target Configuration
Each architecture requires a custom target JSON file with specific settings:
- Panic strategy: "abort" (no unwinding in kernel)
- Disable red zone for interrupt safety
- Soft float for kernel code
- No standard library dependencies

### Workspace Organization
- Use workspace for managing multiple crates
- Shared dependencies in workspace Cargo.toml
- Profile settings: panic = "abort" for both dev and release

### Testing Strategy
- **Unit Tests**: Colocated with implementation using `#[cfg(test)]`
- **Integration Tests**: In `tests/` directory for each crate
- **System Tests**: QEMU-based testing for full kernel functionality
- **Property Testing**: Use `proptest` for complex invariants

### Security Considerations
- Minimize `unsafe` blocks - formal verification required for any unsafe code
- All system calls go through capability validation
- Hardware security features: Support for Intel TDX, AMD SEV-SNP, ARM CCA
- Post-quantum ready: Designed for ML-KEM and ML-DSA algorithms

## Development Phases

Currently implementing in phases:
1. **Phase 0** (Months 1-3): Foundation and tooling - **READY TO START**
2. **Phase 1** (Months 4-9): Microkernel core
3. **Phase 2** (Months 10-15): User space foundation
4. **Phase 3** (Months 16-21): Security hardening
5. **Phase 4** (Months 22-27): Package ecosystem
6. **Phase 5** (Months 28-33): Performance optimization
7. **Phase 6** (Months 34-42): Advanced features and GUI

## Project Status

- **Repository**: https://github.com/doublegate/VeridianOS
- **Documentation**: Complete (25+ comprehensive guides)
- **Infrastructure**: Directory structure, TODO system, and GitHub setup complete
- **CI/CD**: ✅ GitHub Actions workflow passing all checks (optimized pipeline)
- **Current Phase**: Phase 0 (Foundation) - ~45% complete
- **Build Status**: ✅ Compiling successfully for all target architectures
- **Code Quality**: ✅ All format and clippy checks passing
- **Next Milestone**: Boot kernel in QEMU with basic output

## Critical Implementation Notes

### Memory Management
- Hybrid frame allocator: Buddy system for large allocations, bitmap for single frames
- NUMA-aware from the start
- Support for CXL memory and hardware memory tagging (Intel LAM, ARM MTE)

### IPC Implementation
- Synchronous message passing for small messages
- Asynchronous channels for streaming data
- Zero-copy transfers using shared memory mappings
- Capability passing integrated into IPC

### Driver Framework
- Drivers run as separate user processes
- Hardware access only through capability-controlled MMIO regions
- Interrupt forwarding from kernel to user-space drivers
- DMA buffer management with IOMMU protection

## Common Development Tasks

### Adding a New System Call
1. Define capability requirements in `kernel/src/cap/`
2. Add system call handler in `kernel/src/syscall/`
3. Create user-space wrapper in `userland/libs/libveridian/`
4. Add tests in both kernel and user space

### Creating a New Driver
1. Create new crate in `drivers/` directory
2. Implement driver trait from `drivers/common/`
3. Register with driver manager service
4. Add capability definitions for hardware access

### Debugging Kernel Panics
- Use QEMU with `-s -S` flags for GDB debugging
- Enable verbose logging with `RUST_LOG=trace`
- Check serial output for panic messages
- Use `addr2line` for stack trace analysis

## TODO System

Comprehensive task tracking is maintained in the `to-dos/` directory:
- **MASTER_TODO.md**: Overall project status and quick links
- **PHASE[0-6]_TODO.md**: Detailed tasks for each development phase
- **TESTING_TODO.md**: Testing strategy and test tracking
- **ISSUES_TODO.md**: Bug tracking and known issues
- **RELEASE_TODO.md**: Release planning and version milestones

Check these files regularly to track progress and identify next tasks.
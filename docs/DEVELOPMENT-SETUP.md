# VeridianOS Development Environment Setup

This guide will help you set up your development environment for working on VeridianOS.

## Prerequisites

- Linux-based development machine (Fedora, Ubuntu, or similar)
- At least 16GB RAM (recommended for faster builds)
- 20GB+ free disk space
- Internet connection for downloading dependencies

## Required Software

### 1. Rust Toolchain

VeridianOS requires Rust nightly-2025-01-15. The project includes a `rust-toolchain.toml` file that will automatically install the correct version.

```bash
# Install rustup if you haven't already
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# The correct toolchain will be installed automatically when you build
```

### 2. System Dependencies

#### Fedora/RHEL/CentOS
```bash
sudo dnf install -y qemu qemu-system-x86 qemu-system-aarch64 \
    gdb gcc make binutils grub2-tools xorriso mtools
```

#### Ubuntu/Debian
```bash
sudo apt-get update
sudo apt-get install -y qemu-system-x86 qemu-system-arm \
    gdb gcc make binutils grub-pc-bin xorriso mtools
```

### 3. Development Tools

Install the required Cargo tools:

```bash
# This will install all required tools
just install-tools
```

Or manually:
```bash
cargo install bootimage cargo-xbuild cargo-binutils cargo-watch cargo-audit
```

## Quick Start

1. Clone the repository:
```bash
git clone https://github.com/doublegate/VeridianOS.git
cd VeridianOS
```

2. Install development dependencies:
```bash
just install-tools
```

3. Build the kernel:
```bash
just build
```

4. Run in QEMU:
```bash
just run
```

## Build Targets

VeridianOS supports multiple architectures:

- **x86_64**: `just build-x86_64`
- **AArch64**: `just build-aarch64`
- **RISC-V 64**: `just build-riscv64`

## Development Workflow

### Building
```bash
# Build default target (x86_64)
just build

# Build specific architecture
just build-arch x86_64
just build-arch aarch64
just build-arch riscv64

# Or manually with -Zbuild-std for custom targets
cargo build --target targets/x86_64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem
```

### Testing
```bash
# Run all tests
just test

# Run tests with output
just test-verbose
```

### Code Quality
```bash
# Format code
just fmt

# Run clippy linter
just clippy

# Run all CI checks locally
just ci-checks
```

### Debugging
```bash
# Run with GDB debugging
just debug

# In another terminal, connect with GDB
gdb target/x86_64-veridian/debug/veridian-kernel
(gdb) target remote :1234
(gdb) break kernel_main
(gdb) continue
```

## Project Structure

```
VeridianOS/
├── .cargo/           # Cargo configuration
├── kernel/           # Kernel source code
│   ├── src/
│   │   ├── arch/     # Architecture-specific code
│   │   ├── cap/      # Capability system
│   │   ├── ipc/      # Inter-process communication
│   │   ├── mm/       # Memory management
│   │   └── sched/    # Scheduler
│   └── Cargo.toml
├── targets/          # Custom target specifications
├── drivers/          # User-space drivers
├── services/         # System services
├── libs/             # Shared libraries
├── userland/         # User applications
└── docs/             # Documentation
```

## Troubleshooting

### Build Errors

1. **"can't find crate for `core`"**
   - Ensure rust-src component is installed: `rustup component add rust-src`

2. **"error: no matching package named `veridian-kernel`"**
   - Make sure you're in the project root directory
   - Check that kernel/Cargo.toml exists

3. **QEMU errors**
   - Ensure QEMU is installed for your target architecture
   - Check that KVM is available: `ls /dev/kvm`

### Common Issues

- **Slow builds**: Use `just build` instead of `cargo build` for optimized settings
- **Out of memory**: Close other applications or increase system RAM
- **Permission denied**: Some operations may require sudo (like installing packages)

## IDE Setup

### VS Code
1. Install the rust-analyzer extension
2. Open the workspace root directory
3. The extension should automatically detect the project configuration

### IntelliJ IDEA / CLion
1. Install the Rust plugin
2. Open the project root as a Cargo project
3. Configure the custom toolchain in Settings → Rust

## Next Steps

- Read the [Architecture Overview](ARCHITECTURE-OVERVIEW.md)
- Check the [Development Guide](DEVELOPMENT-GUIDE.md)
- Review the [Phase 0 TODO](../to-dos/PHASE0_TODO.md) for current tasks
- Join the development discussion on GitHub

## Getting Help

- Check the [FAQ](FAQ.md) for common questions
- Open an issue on [GitHub](https://github.com/doublegate/VeridianOS/issues)
- Read the [Troubleshooting Guide](TROUBLESHOOTING.md)
# VeridianOS Build Instructions

**Last Updated**: June 13, 2025

## Quick Start

```bash
# Clone the repository
git clone https://github.com/doublegate/VeridianOS.git
cd VeridianOS

# Install dependencies (Ubuntu/Debian)
./scripts/install-deps.sh

# Build all architectures using the automated script
./build-kernel.sh all dev

# Or build specific architecture
./build-kernel.sh x86_64 dev
./build-kernel.sh aarch64 dev
./build-kernel.sh riscv64 dev

# Run in QEMU (RISC-V recommended as it boots successfully)
just run-riscv
```

## Detailed Build Instructions

### Prerequisites

#### Required Tools

- Rust nightly-2025-01-15 or later
- QEMU 8.0+ (for testing)
- Python 3.8+
- Git

#### System Packages

**Ubuntu/Debian:**
```bash
sudo apt update
sudo apt install -y \
    build-essential \
    nasm \
    mtools \
    xorriso \
    qemu-system-x86 \
    qemu-system-arm \
    qemu-system-misc \
    gdb-multiarch \
    clang \
    lld
```

**Fedora:**
```bash
sudo dnf install -y \
    @development-tools \
    nasm \
    mtools \
    xorriso \
    qemu-system-x86 \
    qemu-system-aarch64 \
    qemu-system-riscv \
    gdb \
    clang \
    lld
```

**macOS:**
```bash
brew install \
    nasm \
    mtools \
    xorriso \
    qemu \
    x86_64-elf-gdb \
    aarch64-elf-gdb \
    llvm
```

### Installing Rust Toolchain

```bash
# Install rustup if not already installed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Install specific nightly version
rustup toolchain install nightly-2025-01-15
rustup default nightly-2025-01-15

# Add required components
rustup component add rust-src llvm-tools-preview rustfmt clippy

# Add target architectures
rustup target add x86_64-unknown-none
rustup target add aarch64-unknown-none
rustup target add riscv64gc-unknown-none-elf

# Install cargo tools
cargo install cargo-xbuild bootimage just
```

## Building VeridianOS

### Build Status

| Architecture | Build Status | Notes |
|--------------|-------------|-------|
| x86_64       | ✅ Working  | Uses bootloader crate |
| AArch64      | ✅ Working  | Custom boot sequence |
| RISC-V 64    | ✅ Working  | Works with OpenSBI |

### Using the Build Script (Recommended)

The project includes `build-kernel.sh` which handles architecture-specific build configurations and fixes:

```bash
# Build all architectures (development mode)
./build-kernel.sh all dev

# Build all architectures (release mode)
./build-kernel.sh all release

# Build specific architecture
./build-kernel.sh x86_64 dev      # Uses kernel code model to fix relocation issues
./build-kernel.sh aarch64 release
./build-kernel.sh riscv64 dev
```

### Using Just

We use `just` as our command runner. All common tasks have just recipes:

```bash
# Build for default target (x86_64)
just build

# Build for specific architecture
just build-x86_64
just build-aarch64
just build-riscv64

# Build all architectures
just build-all

# Build in release mode
just build-release

# Clean build artifacts
just clean
```

### Manual Build

#### Building the Kernel

**Important**: x86_64 requires custom target JSON with kernel code model to avoid R_X86_64_32S relocation errors:

```bash
# Build for x86_64 (with kernel code model)
cargo build --target targets/x86_64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc

# Build for AArch64 (standard bare metal target)
cargo build --target aarch64-unknown-none -p veridian-kernel

# Build for RISC-V (standard bare metal target)
cargo build --target riscv64gc-unknown-none-elf -p veridian-kernel

# Release build
cargo build --release --target targets/x86_64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc
```

**Note**: The x86_64 kernel is linked at 0xFFFFFFFF80100000 (top 2GB of virtual memory) to work with the kernel code model.

#### Creating Bootable Image

```bash
# For x86_64 with UEFI
cargo bootimage --target targets/x86_64-veridian.json

# The bootable image will be at:
# target/x86_64-veridian/debug/bootimage-veridian-kernel.bin
```

### Build Options

#### Features

Enable specific features during build:

```bash
# Enable test harness
cargo build --features test-harness

# Enable debugging features
cargo build --features debug-assertions,memory-debug

# Enable all optimizations
cargo build --release --features optimize-size
```

#### Environment Variables

Control build behavior with environment variables:

```bash
# Set log level
RUST_LOG=debug cargo build

# Enable link-time optimization
CARGO_PROFILE_RELEASE_LTO=true cargo build --release

# Set specific CPU features
RUSTFLAGS="-C target-cpu=native" cargo build
```

## Cross-Compilation

### Building for Different Architectures

#### x86_64 → AArch64

```bash
# Install cross-compilation tools
sudo apt install gcc-aarch64-linux-gnu

# Build
cargo build --target targets/aarch64-veridian.json
```

#### x86_64 → RISC-V

```bash
# Install cross-compilation tools
sudo apt install gcc-riscv64-linux-gnu

# Build
cargo build --target targets/riscv64-veridian.json
```

### Using Docker for Builds

We provide Docker images for consistent build environments:

```bash
# Build using Docker
docker run --rm -v $(pwd):/work veridian/build-env \
    cargo build --target targets/x86_64-veridian.json

# Or use docker-compose
docker-compose run build
```

**Dockerfile:**
```dockerfile
FROM rust:latest

# Install dependencies
RUN apt-get update && apt-get install -y \
    nasm \
    mtools \
    xorriso \
    clang \
    lld

# Install Rust tools
RUN rustup component add rust-src llvm-tools-preview
RUN cargo install cargo-xbuild bootimage

WORKDIR /work
```

## Building Components

### Building Drivers

```bash
# Build all drivers
just build-drivers

# Build specific driver
cargo build -p nvme-driver

# Build driver for specific target
cargo build -p nvme-driver --target targets/x86_64-veridian.json
```

### Building Services

```bash
# Build all services
just build-services

# Build specific service
cargo build -p vfs-service

# Build with specific features
cargo build -p network-service --features "tcp ipv6"
```

### Building User Applications

```bash
# Build all userland applications
just build-userland

# Build specific application
cargo build -p vsh

# Build with static linking
RUSTFLAGS="-C target-feature=+crt-static" cargo build -p vsh
```

## Build Optimization

### Size Optimization

```toml
# Cargo.toml
[profile.release]
opt-level = "z"     # Optimize for size
lto = true          # Enable LTO
codegen-units = 1   # Single codegen unit
strip = true        # Strip symbols
```

### Performance Optimization

```toml
# Cargo.toml
[profile.release]
opt-level = 3       # Maximum optimization
lto = "thin"        # Thin LTO for faster builds
codegen-units = 16  # Parallel codegen
```

### Debug Build Optimization

```toml
# Cargo.toml
[profile.dev]
opt-level = 1       # Basic optimization in debug
debug = 2           # Full debug info
```

## Running and Testing

### Running in QEMU

#### x86_64

```bash
# Build kernel first
./build-kernel.sh x86_64 dev

# Run with QEMU directly
qemu-system-x86_64 -kernel target/x86_64-veridian/debug/veridian-kernel -serial stdio -display none

# Or build and run with cargo
cargo run --target targets/x86_64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc -- -serial stdio -display none

# Or use just
just run
```

#### AArch64

```bash
# Build kernel
./build-kernel.sh aarch64 dev

# Run with QEMU
qemu-system-aarch64 -M virt -cpu cortex-a57 -nographic -kernel target/aarch64-unknown-none/debug/veridian-kernel
```

#### RISC-V

```bash
# Build kernel
./build-kernel.sh riscv64 dev

# Run with QEMU
qemu-system-riscv64 -M virt -nographic -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel
```

### Testing Status

| Architecture | Build | Boot | Serial I/O | Notes |
|--------------|-------|------|------------|-------|
| x86_64       | ✅    | ✅   | ✅         | Fully working with kernel code model fix |
| AArch64      | ✅    | ✅   | ✅         | Boot sequence fixed and working |
| RISC-V 64    | ✅    | ✅   | ✅         | Works with OpenSBI firmware |

### Running Tests

```bash
# Run unit tests
cargo test --target targets/x86_64-veridian.json

# Run integration tests
just test-integration

# Run all tests
just test-all
```

## Troubleshooting

### Common Build Errors

#### "can't find crate for `core`"

This means the rust-src component is missing:
```bash
# Install rust-src
rustup component add rust-src

# For x86_64 custom target, use -Zbuild-std:
cargo build --target targets/x86_64-veridian.json -Zbuild-std=core,compiler_builtins,alloc

# For standard targets (aarch64, riscv64), no special flags needed
```

#### "error: linker `rust-lld` not found"

Install LLVM tools:
```bash
rustup component add llvm-tools-preview
```

#### "NASM not found"

Install NASM:
```bash
# Ubuntu/Debian
sudo apt install nasm

# macOS
brew install nasm
```

#### Out of Memory During Build

Reduce parallel jobs:
```bash
# Limit to 2 parallel jobs
cargo build -j 2
```

Or increase system swap:
```bash
# Create 4GB swap file
sudo fallocate -l 4G /swapfile
sudo chmod 600 /swapfile
sudo mkswap /swapfile
sudo swapon /swapfile
```

### Platform-Specific Issues

#### macOS

If you encounter linking issues on macOS:
```bash
# Use GNU linker
brew install x86_64-elf-binutils
export CARGO_TARGET_X86_64_UNKNOWN_NONE_LINKER=x86_64-elf-ld
```

#### Windows (WSL2)

Build under WSL2 for best compatibility:
```bash
# Ensure WSL2 is used
wsl --set-default-version 2

# Install Ubuntu
wsl --install -d Ubuntu

# Follow Linux instructions inside WSL2
```

## Continuous Integration

### GitHub Actions

Our CI builds on every commit and is **fully operational**:

```yaml
# .github/workflows/ci.yml
name: CI

on: [push, pull_request]

jobs:
  quick-checks:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: nightly-2025-01-15
          components: rustfmt, clippy
      - run: cargo fmt --all -- --check
      - run: cargo clippy --all -- -D warnings

  build-and-test:
    strategy:
      matrix:
        include:
          - arch: x86_64
            target: targets/x86_64-veridian.json
            build-std: true
          - arch: aarch64
            target: aarch64-unknown-none
            build-std: false
          - arch: riscv64
            target: riscv64gc-unknown-none-elf
            build-std: false
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: nightly-2025-01-15
          components: rust-src, llvm-tools
      - run: |
          if [ "${{ matrix.build-std }}" = "true" ]; then
            cargo build --target ${{ matrix.target }} -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc
          else
            cargo build --target ${{ matrix.target }} -p veridian-kernel
          fi

  security-audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: rustsec/audit-check@v2
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
```

### Local CI Testing

Test CI locally using act:
```bash
# Install act
cargo install act

# Run CI locally
act -j build
```

## Build Performance

### Incremental Builds

Enable incremental compilation:
```bash
export CARGO_INCREMENTAL=1
cargo build
```

### Sccache

Use sccache for faster rebuilds:
```bash
# Install sccache
cargo install sccache

# Configure
export RUSTC_WRAPPER=sccache
cargo build
```

### Build Caching

Cache dependencies:
```bash
# Build dependencies only
cargo build --dependencies-only

# Use cargo-chef for Docker
FROM rust as planner
WORKDIR /app
RUN cargo install cargo-chef
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM rust as cacher
WORKDIR /app
RUN cargo install cargo-chef
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

FROM rust as builder
WORKDIR /app
COPY . .
COPY --from=cacher /app/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
RUN cargo build --release
```

## Release Builds

### Creating Release

```bash
# Full release build
just release

# This runs:
# - cargo build --release for all targets
# - cargo test --release
# - Creates bootable images
# - Generates checksums
# - Creates release archive
```

### Signing Releases

```bash
# Sign with GPG
gpg --detach-sign --armor target/release/veridian-kernel

# Verify signature
gpg --verify veridian-kernel.asc veridian-kernel
```

## Next Steps

After successfully building VeridianOS:

1. [Run in QEMU](RUNNING.md)
2. [Run tests](TESTING-STRATEGY.md)
3. [Start developing](DEVELOPMENT-GUIDE.md)
4. [Deploy to hardware](DEPLOYMENT.md)
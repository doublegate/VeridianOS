# VeridianOS Build Instructions

## Quick Start

```bash
# Clone the repository
git clone https://github.com/veridian-os/veridian.git
cd veridian

# Install dependencies (Ubuntu/Debian)
./scripts/install-deps.sh

# Build and run
just run
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

### Using Just (Recommended)

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

```bash
# Build for x86_64
cargo build --target targets/x86_64-veridian.json

# Build for AArch64
cargo build --target targets/aarch64-veridian.json

# Build for RISC-V
cargo build --target targets/riscv64-veridian.json

# Release build
cargo build --release --target targets/x86_64-veridian.json
```

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

## Troubleshooting

### Common Build Errors

#### "can't find crate for `core`"

This means the rust-src component is missing:
```bash
rustup component add rust-src
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

Our CI builds on every commit:

```yaml
# .github/workflows/build.yml
name: Build

on: [push, pull_request]

jobs:
  build:
    strategy:
      matrix:
        target: [x86_64, aarch64, riscv64]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: dtolnay/rust-toolchain@nightly
        with:
          toolchain: nightly-2025-01-15
          components: rust-src, llvm-tools-preview
      - run: just build-${{ matrix.target }}
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
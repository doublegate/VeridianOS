# Building VeridianOS

This guide covers building VeridianOS from source for all supported architectures.

## Prerequisites

Before building, ensure you have:
- Completed the [development setup](./dev-setup.md)
- Rust nightly toolchain installed
- Required system packages
- At least 2GB free disk space

## Quick Build

The easiest way to build VeridianOS using the automated build script:

```bash
# Build all architectures (development)
./build-kernel.sh all dev

# Build specific architecture
./build-kernel.sh x86_64 dev

# Build release version
./build-kernel.sh all release

# Alternative: using just
just build
just run
```

## Architecture-Specific Builds

### x86_64

**Note**: x86_64 requires custom target with kernel code model to avoid relocation errors.

```bash
# Recommended: using build script
./build-kernel.sh x86_64 dev

# Using just
just build-x86_64

# Manual build (with kernel code model)
cargo build --target targets/x86_64-veridian.json \
    -p veridian-kernel \
    -Zbuild-std=core,compiler_builtins,alloc
```

Output: `target/x86_64-veridian/debug/veridian-kernel`

### AArch64

```bash
# Recommended: using build script
./build-kernel.sh aarch64 dev

# Using just
just build-aarch64

# Manual build (standard bare metal target)
cargo build --target aarch64-unknown-none \
    -p veridian-kernel
```

Output: `target/aarch64-unknown-none/debug/veridian-kernel`

### RISC-V 64

```bash
# Recommended: using build script
./build-kernel.sh riscv64 dev

# Using just
just build-riscv64

# Manual build (standard bare metal target)
cargo build --target riscv64gc-unknown-none-elf \
    -p veridian-kernel
```

Output: `target/riscv64gc-unknown-none-elf/debug/veridian-kernel`

## Build Options

### Release Builds

For optimized builds:

```bash
# Using build script (recommended)
./build-kernel.sh all release

# Using just
just build-release

# Manual for x86_64
cargo build --release --target targets/x86_64-veridian.json \
    -p veridian-kernel \
    -Zbuild-std=core,compiler_builtins,alloc
```

### Build All Architectures

```bash
just build-all
```

This builds debug versions for all three architectures.

## Build Flags Explained

### -Zbuild-std

Custom targets require building the Rust standard library from source:
- `core`: Core library (no_std)
- `compiler_builtins`: Low-level compiler intrinsics
- `alloc`: Allocation support (when ready)

### -Zbuild-std-features

Enables memory-related compiler builtins required for kernel development.

## Creating Bootable Images

### x86_64 Boot Image

```bash
# Create bootable image
cargo bootimage --target targets/x86_64-veridian.json

# Output location
ls target/x86_64-veridian/debug/bootimage-veridian-kernel.bin
```

### Other Architectures

AArch64 and RISC-V use the raw kernel binary directly:
- AArch64: Load at 0x40080000
- RISC-V: Load with OpenSBI

## Build Artifacts

Build outputs are organized by architecture:

```
target/
├── x86_64-veridian/
│   ├── debug/
│   │   ├── veridian-kernel
│   │   └── bootimage-veridian-kernel.bin
│   └── release/
├── aarch64-veridian/
│   ├── debug/
│   │   └── veridian-kernel
│   └── release/
└── riscv64gc-veridian/
    ├── debug/
    │   └── veridian-kernel
    └── release/
```

## Common Issues

### Rust Toolchain

```
error: failed to run `rustc` to learn about target-specific information
```

**Solution**: Install the correct nightly toolchain:
```bash
rustup toolchain install nightly-2025-01-15
rustup override set nightly-2025-01-15
```

### Missing Components

```
error: the component `rust-src` is required
```

**Solution**: Add required components:
```bash
rustup component add rust-src llvm-tools-preview
```

### Build Cache

If builds fail unexpectedly:
```bash
# Clean and rebuild
just clean
just build
```

## Build Performance

### Incremental Builds

Rust automatically uses incremental compilation. First build is slow (~2 minutes), subsequent builds are much faster (~30 seconds).

### Parallel Builds

Cargo uses all available CPU cores by default. To limit:
```bash
cargo build -j 4  # Use 4 cores
```

### Build Cache

The target directory can grow large. Clean periodically:
```bash
just clean        # Clean debug builds
just clean-all    # Clean everything
```

## CI/CD Builds

Our GitHub Actions workflow builds all architectures on every push. Check the [Actions tab](https://github.com/doublegate/VeridianOS/actions) for build status.

## Next Steps

After building successfully:
- [Run in QEMU](./running.md)
- [Debug with GDB](../development/debugging.md)
- [Run tests](../development/testing.md)

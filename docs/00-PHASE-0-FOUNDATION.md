# Phase 0: Foundation and Tooling Setup (Months 1-3)

**Current Status**: IN PROGRESS (~50% Complete) - CI/CD fully operational, basic kernel structure implemented

## Overview

Phase 0 establishes the fundamental development environment, build infrastructure, and project scaffolding for VeridianOS. This phase focuses on creating a solid foundation for all subsequent development work.

## Objectives

1. **Development Environment Setup**: Configure Rust toolchain and development tools
2. **Build Infrastructure**: Create build system and custom target specifications
3. **Project Scaffolding**: Establish basic project structure and workspace
4. **Bootloader Integration**: Minimal boot capability for testing
5. **CI/CD Pipeline**: Automated testing and build infrastructure
6. **Documentation Framework**: Establish documentation standards and tooling

## Detailed Implementation Plan

### 1. Development Environment Setup

#### 1.1 Rust Toolchain Configuration
```bash
# Install specific nightly version for kernel development
rustup toolchain install nightly-2025-01-15
rustup default nightly-2025-01-15

# Add required components
rustup component add rust-src llvm-tools-preview rustfmt clippy

# Target additions for cross-compilation
rustup target add x86_64-unknown-none
rustup target add aarch64-unknown-none
rustup target add riscv64gc-unknown-none-elf
```

#### 1.2 Development Tools Installation
```bash
# Essential cargo tools
cargo install cargo-xbuild      # Cross-compilation support
cargo install bootimage         # Bootable image creation
cargo install cargo-watch       # Auto-rebuild on changes
cargo install cargo-expand      # Macro expansion debugging
cargo install cargo-audit       # Security vulnerability scanning
cargo install cargo-nextest     # Advanced test runner
cargo install cargo-binutils    # Binary manipulation tools

# Optional but recommended
cargo install cargo-outdated    # Dependency version checking
cargo install cargo-tree        # Dependency visualization
cargo install cargo-bloat       # Binary size analysis
```

#### 1.3 System Dependencies
```bash
# Ubuntu/Debian
sudo apt install qemu-system-x86 qemu-system-arm qemu-system-misc
sudo apt install gdb-multiarch gcc-aarch64-linux-gnu gcc-riscv64-linux-gnu
sudo apt install nasm mtools xorriso

# Fedora
sudo dnf install qemu qemu-system-x86 qemu-system-aarch64 qemu-system-riscv
sudo dnf install gdb gcc-aarch64-linux-gnu gcc-riscv64-linux-gnu
sudo dnf install nasm mtools xorriso
```

### 2. Build Infrastructure

#### 2.1 Custom Target Specifications

Create `targets/` directory with architecture-specific JSON files:

**targets/x86_64-veridian.json**
```json
{
  "llvm-target": "x86_64-unknown-none",
  "data-layout": "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128",
  "arch": "x86_64",
  "target-endian": "little",
  "target-pointer-width": "64",
  "target-c-int-width": "32",
  "os": "none",
  "executables": true,
  "linker-flavor": "ld.lld",
  "linker": "rust-lld",
  "panic-strategy": "abort",
  "disable-redzone": true,
  "features": "-mmx,-sse,+soft-float",
  "pre-link-args": {
    "ld.lld": [
      "--script=kernel/src/arch/x86_64/linker.ld"
    ]
  }
}
```

**targets/aarch64-veridian.json**
```json
{
  "llvm-target": "aarch64-unknown-none",
  "data-layout": "e-m:e-i8:8:32-i16:16:32-i64:64-i128:128-n32:64-S128",
  "arch": "aarch64",
  "target-endian": "little",
  "target-pointer-width": "64",
  "target-c-int-width": "32",
  "os": "none",
  "executables": true,
  "linker-flavor": "ld.lld",
  "linker": "rust-lld",
  "panic-strategy": "abort",
  "features": "+strict-align,-neon,-fp-armv8",
  "pre-link-args": {
    "ld.lld": [
      "--script=kernel/src/arch/aarch64/linker.ld"
    ]
  }
}
```

**targets/riscv64-veridian.json**
```json
{
  "llvm-target": "riscv64gc-unknown-none-elf",
  "data-layout": "e-m:e-p:64:64-i64:64-i128:128-n64-S128",
  "arch": "riscv64",
  "target-endian": "little",
  "target-pointer-width": "64",
  "target-c-int-width": "32",
  "os": "none",
  "executables": true,
  "linker-flavor": "ld.lld",
  "linker": "rust-lld",
  "panic-strategy": "abort",
  "features": "+m,+a,+c",
  "pre-link-args": {
    "ld.lld": [
      "--script=kernel/src/arch/riscv64/linker.ld"
    ]
  }
}
```

#### 2.2 Build System Configuration

**Justfile** (root directory)
```makefile
# Default recipe
default:
    @just --list

# Build for all architectures
build-all: build-x86_64 build-aarch64 build-riscv64

# Architecture-specific builds
build-x86_64:
    cargo build --target targets/x86_64-veridian.json

build-aarch64:
    cargo build --target targets/aarch64-veridian.json

build-riscv64:
    cargo build --target targets/riscv64-veridian.json

# Run with QEMU
run-x86_64:
    cargo run --target targets/x86_64-veridian.json -- \
        -serial stdio -display none -m 128M

run-aarch64:
    cargo run --target targets/aarch64-veridian.json -- \
        -machine virt -cpu cortex-a72 -serial stdio -display none -m 128M

run-riscv64:
    cargo run --target targets/riscv64-veridian.json -- \
        -machine virt -serial stdio -display none -m 128M

# Testing
test:
    cargo test --workspace
    cargo test --workspace --doc
    cargo nextest run

# Formatting and linting
fmt:
    cargo fmt --all

check:
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings

# Clean build artifacts
clean:
    cargo clean
    rm -rf target/

# Documentation
doc:
    cargo doc --no-deps --workspace --open

# Security audit
audit:
    cargo audit

# Create bootable image (x86_64 only initially)
bootimage:
    cargo bootimage --target targets/x86_64-veridian.json
```

### 3. Project Scaffolding

#### 3.1 Workspace Structure

**Cargo.toml** (root)
```toml
[workspace]
resolver = "2"
members = [
    "kernel",
    "bootloader",
    "libs/common",
    "libs/veridian-abi",
    "tools/build-utils",
]

[workspace.package]
version = "0.1.0"
authors = ["VeridianOS Contributors"]
edition = "2021"
license = "MIT OR Apache-2.0"
repository = "https://github.com/doublegate/VeridianOS"

[workspace.dependencies]
# Core dependencies
spin = "0.9"
bitflags = "2.4"
log = "0.4"

# Architecture-specific
x86_64 = "0.14"
cortex-a = "8.1"
riscv = "0.10"

# Testing
proptest = "1.4"
mockall = "0.12"

[profile.dev]
panic = "abort"
opt-level = 0
debug = true

[profile.release]
panic = "abort"
opt-level = 3
lto = true
codegen-units = 1
strip = true
```

#### 3.2 Kernel Crate Structure

**kernel/Cargo.toml**
```toml
[package]
name = "veridian-kernel"
version.workspace = true
authors.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
spin.workspace = true
bitflags.workspace = true
log.workspace = true

[target.'cfg(target_arch = "x86_64")'.dependencies]
x86_64.workspace = true

[target.'cfg(target_arch = "aarch64")'.dependencies]
cortex-a.workspace = true

[target.'cfg(target_arch = "riscv64")'.dependencies]
riscv.workspace = true

[dev-dependencies]
proptest.workspace = true
mockall.workspace = true
```

**kernel/src/lib.rs**
```rust
#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(asm_const)]
#![feature(core_intrinsics)]
#![feature(alloc_error_handler)]

// Architecture selection
#[cfg(target_arch = "x86_64")]
pub mod arch {
    pub mod x86_64;
    pub use x86_64::*;
}

#[cfg(target_arch = "aarch64")]
pub mod arch {
    pub mod aarch64;
    pub use aarch64::*;
}

#[cfg(target_arch = "riscv64")]
pub mod arch {
    pub mod riscv64;
    pub use riscv64::*;
}

// Core modules (stubs for Phase 0)
pub mod boot;
pub mod panic;
pub mod serial;

// Kernel entry point
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // Initialize serial output
    serial::init();
    
    // Print boot message
    println!("VeridianOS v{} booting...", env!("CARGO_PKG_VERSION"));
    println!("Architecture: {}", core::any::type_name::<arch::Arch>());
    
    // Halt
    loop {
        arch::halt();
    }
}
```

### 4. Minimal Boot Implementation

#### 4.1 x86_64 Boot Stub

**kernel/src/arch/x86_64/boot.s**
```assembly
.section .boot
.global _start
.code64

_start:
    # Set up stack
    mov $stack_top, %rsp
    
    # Clear direction flag
    cld
    
    # Call kernel main
    call kernel_main
    
    # Should never reach here
    cli
    hlt

.section .bss
.align 16
stack_bottom:
    .space 16384  # 16KB stack
stack_top:
```

#### 4.2 Linker Script

**kernel/src/arch/x86_64/linker.ld**
```ld
ENTRY(_start)

SECTIONS {
    . = 1M;
    
    .boot :
    {
        KEEP(*(.boot))
    }
    
    .text :
    {
        *(.text .text.*)
    }
    
    .rodata :
    {
        *(.rodata .rodata.*)
    }
    
    .data :
    {
        *(.data .data.*)
    }
    
    .bss :
    {
        *(.bss .bss.*)
        *(COMMON)
    }
    
    /DISCARD/ :
    {
        *(.eh_frame)
        *(.note.*)
    }
}
```

### 5. CI/CD Pipeline

#### 5.1 GitHub Actions Workflow

**.github/workflows/ci.yml**
```yaml
name: CI

on:
  push:
    branches: [ main, develop ]
  pull_request:
    branches: [ main ]

env:
  RUST_BACKTRACE: 1
  RUSTFLAGS: -D warnings

jobs:
  check:
    name: Check
    runs-on: ubuntu-latest
    strategy:
      matrix:
        target: [x86_64, aarch64, riscv64]
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@nightly
        with:
          toolchain: nightly-2025-01-15
          components: rust-src, llvm-tools-preview
      
      - name: Cache Dependencies
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
      
      - name: Check
        run: cargo check --target targets/${{ matrix.target }}-veridian.json

  test:
    name: Test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@nightly
        with:
          toolchain: nightly-2025-01-15
      
      - name: Run Tests
        run: cargo nextest run --workspace

  fmt:
    name: Format
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@nightly
        with:
          toolchain: nightly-2025-01-15
          components: rustfmt
      
      - name: Check Format
        run: cargo fmt --all -- --check

  clippy:
    name: Clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@nightly
        with:
          toolchain: nightly-2025-01-15
          components: clippy
      
      - name: Run Clippy
        run: cargo clippy --all-targets --all-features -- -D warnings

  audit:
    name: Security Audit
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
      
      - name: Install cargo-audit
        run: cargo install cargo-audit
      
      - name: Run Audit
        run: cargo audit
```

### 6. Documentation Framework

#### 6.1 Documentation Standards

**docs/DOCUMENTATION-STANDARDS.md**
```markdown
# VeridianOS Documentation Standards

## Documentation Types

1. **API Documentation**: In-code documentation using rustdoc
2. **Architecture Documents**: High-level design decisions in `docs/`
3. **Implementation Guides**: Step-by-step implementation details
4. **User Documentation**: End-user guides and tutorials

## Rustdoc Guidelines

- Every public item must have documentation
- Include examples for non-trivial APIs
- Use `# Safety` sections for unsafe functions
- Use `# Panics` sections where applicable
- Use `# Examples` liberally

## Markdown Standards

- Use ATX-style headers (#)
- Code blocks must specify language
- Keep line length under 100 characters
- Use reference-style links for repeated URLs
```

## Deliverables for Phase 0

### Week 1-2: Environment Setup
- [ ] Install Rust toolchain and tools
- [ ] Configure development environment
- [ ] Set up version control

### Week 3-4: Build Infrastructure
- [ ] Create custom target specifications
- [ ] Implement build system (Justfile)
- [ ] Configure workspace structure

### Week 5-6: Basic Boot
- [ ] Implement minimal boot stub for x86_64
- [ ] Create linker scripts
- [ ] Verify QEMU execution

### Week 7-8: CI/CD Pipeline
- [ ] Set up GitHub Actions
- [ ] Configure automated testing
- [ ] Implement security scanning

### Week 9-10: Project Foundation
- [ ] Complete project scaffolding
- [ ] Document all configurations
- [ ] Create development guides

### Week 11-12: Validation and Documentation
- [ ] Test all build configurations
- [ ] Complete Phase 0 documentation
- [ ] Prepare for Phase 1

## Success Criteria

1. **Build System**: Can build kernel for all three architectures
2. **Boot Test**: Minimal kernel boots in QEMU for x86_64
3. **CI/CD**: All checks pass in automated pipeline
4. **Documentation**: Complete setup guides available
5. **Tooling**: All development tools installed and configured

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Toolchain instability | High | Pin specific nightly version |
| Cross-compilation issues | Medium | Test on multiple host systems |
| CI/CD complexity | Medium | Start simple, iterate |
| Documentation drift | Low | Automate doc generation |

## Dependencies for Phase 1

Phase 0 must deliver:
- Working build system for all architectures
- Basic project structure
- CI/CD pipeline
- Development environment documentation
- Minimal boot capability for testing

## Notes

- Focus on stability over features
- Document everything thoroughly
- Test on multiple development platforms
- Keep build times reasonable
- Prepare comprehensive handoff to Phase 1
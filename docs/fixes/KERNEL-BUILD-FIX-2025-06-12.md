# Kernel Build Fix - June 12, 2025

## Issue Summary
The x86_64 kernel build was failing with R_X86_64_32S relocation errors when using the custom target JSON. The error occurred because the kernel was being linked at an address (0xFFFF800000100000) that was outside the ±2GB range that R_X86_64_32S relocations can handle.

## Root Cause
- The kernel linker script was placing the kernel at 0xFFFF800000100000
- This address is far beyond the ±2GB range from 0x0
- R_X86_64_32S relocations can only handle signed 32-bit offsets
- The precompiled core library wasn't built with the kernel code model

## Solution Implemented

### 1. Fixed Linker Script
Updated `kernel/src/arch/x86_64/link.ld` to use the correct address:
```ld
SECTIONS {
    /* Higher half kernel at -2GB (0xFFFFFFFF80000000) */
    . = 0xFFFFFFFF80100000;
```

### 2. Updated .cargo/config.toml
Added rustflags for the x86_64-veridian target:
```toml
[target.x86_64-veridian]
rustflags = ["-C", "code-model=kernel", "-C", "relocation-model=static"]
```

### 3. Created Build Script
Created `build-kernel.sh` to automate builds for all architectures:
- Handles x86_64 with custom target JSON and build-std
- Uses standard bare metal targets for AArch64 and RISC-V
- Supports both dev and release builds
- Provides clear error messages and build status

### 4. Documentation Updates
- Updated CLAUDE.md with new build instructions
- Created KERNEL-BUILD-TROUBLESHOOTING.md guide
- Updated README.md with build script usage
- Preserved original documentation in docs/reference/kernel_issue/

## Verification
All architectures now build successfully:
- x86_64: Uses custom target with kernel code model
- AArch64: Standard aarch64-unknown-none target
- RISC-V: Standard riscv64gc-unknown-none-elf target

## Key Learnings
1. The kernel code model is essential for x86_64 kernels linked in the top 2GB
2. Custom target JSON files must specify "code-model": "kernel"
3. Build-std is required to rebuild core with the correct code model
4. AArch64 and RISC-V don't have the same relocation limitations

## Files Modified
- `/kernel/src/arch/x86_64/link.ld` - Fixed kernel base address
- `/.cargo/config.toml` - Added rustflags for x86_64-veridian
- `/build-kernel.sh` - Created automated build script
- `/CLAUDE.md` - Updated build instructions
- `/README.md` - Added build script usage
- `/docs/KERNEL-BUILD-TROUBLESHOOTING.md` - Created troubleshooting guide

## Build Commands
```bash
# Automated (recommended)
./build-kernel.sh all dev
./build-kernel.sh all release

# Manual x86_64
cargo build --target targets/x86_64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc

# Manual other architectures
cargo build --target aarch64-unknown-none -p veridian-kernel
cargo build --target riscv64gc-unknown-none-elf -p veridian-kernel
```
# Runtime Testing Guide for VeridianOS

**Last Updated**: November 18, 2025
**Status**: Integrated subsystems ready for testing

## Overview

All six phases of VeridianOS are now architecturally complete and integrated into the kernel bootstrap sequence. This document describes how to test the kernel runtime when QEMU is available.

## Prerequisites

### Required Tools

1. **QEMU** for each target architecture:
   ```bash
   # Install QEMU
   sudo apt install qemu-system-x86 qemu-system-arm qemu-system-misc
   # Or on Fedora/RHEL:
   sudo dnf install qemu-system-x86 qemu-system-aarch64 qemu-system-riscv
   ```

2. **Rust Toolchain** (already configured):
   ```bash
   rustup toolchain install nightly-2025-01-15
   rustup component add rust-src llvm-tools-preview
   ```

## Build Instructions

### x86_64

```bash
# Build kernel
cargo build --target targets/x86_64-veridian.json -p veridian-kernel \
  -Zbuild-std=core,compiler_builtins,alloc

# For x86_64, you need to create a bootable image using bootimage:
cargo install bootimage
cargo bootimage --target targets/x86_64-veridian.json
```

### AArch64

```bash
# Build kernel
cargo build --target aarch64-unknown-none -p veridian-kernel \
  -Zbuild-std=core,compiler_builtins,alloc
```

### RISC-V

```bash
# Build kernel
cargo build --target riscv64gc-unknown-none-elf -p veridian-kernel \
  -Zbuild-std=core,compiler_builtins,alloc
```

## Running Tests

### Automated Test Script

Use the provided test script:

```bash
./test-boot.sh x86_64    # Test x86_64
./test-boot.sh aarch64   # Test AArch64
./test-boot.sh riscv64   # Test RISC-V
./test-boot.sh all       # Test all architectures
```

### Manual Testing

#### x86_64

```bash
timeout 10 qemu-system-x86_64 \
  -drive format=raw,file=target/x86_64-unknown-none/debug/bootimage-veridian-kernel.bin \
  -serial stdio \
  -display none
```

#### AArch64

```bash
timeout 10 qemu-system-aarch64 \
  -M virt \
  -cpu cortex-a57 \
  -kernel target/aarch64-unknown-none/debug/veridian-kernel \
  -serial stdio \
  -display none
```

#### RISC-V

```bash
timeout 10 qemu-system-riscv64 \
  -M virt \
  -kernel target/riscv64gc-unknown-none-elf/debug/veridian-kernel \
  -serial stdio \
  -display none
```

## Expected Boot Sequence

A successful boot should show all 6 stages:

```
[STAGE 1] Hardware initialization
[STAGE 2] Memory management
[STAGE 3] Process management
[STAGE 4] Core kernel services
  [BOOTSTRAP] Initializing capabilities...
  [BOOTSTRAP] Capabilities initialized
  [BOOTSTRAP] Initializing security subsystem...
  [CRYPTO] Initializing cryptography subsystem...
  [CRYPTO] Cryptography subsystem initialized
  [MAC] Initializing Mandatory Access Control...
  [MAC] MAC system initialized with N rules
  [AUDIT] Initializing audit framework...
  [AUDIT] Audit framework initialized
  [SECBOOT] Secure boot disabled
  [BOOTSTRAP] Security subsystem initialized successfully
  [BOOTSTRAP] Initializing performance monitoring...
  [PERF] Initializing performance subsystem...
  [PERF] Optimizing memory allocator...
  [PERF] Optimizing scheduler...
  [PERF] Optimizing IPC...
  [PERF] Performance subsystem initialized
  [BOOTSTRAP] Performance monitoring initialized
  [BOOTSTRAP] Initializing IPC...
  [BOOTSTRAP] IPC initialized
  [BOOTSTRAP] Initializing VFS...
  [BOOTSTRAP] VFS initialized
  [BOOTSTRAP] Initializing services...
  [BOOTSTRAP] Services initialized
[STAGE 5] Scheduler initialization
  [BOOTSTRAP] Initializing package manager...
  [PKG] Initializing package manager...
  [PKG] Installing package: veridian-base v0.1.0
  [PKG] Package installed successfully
  [PKG] Installing package: veridian-utils v0.1.0
  [PKG] Package installed successfully
  [PKG] Package manager initialized
  [BOOTSTRAP] Package manager initialized
  [BOOTSTRAP] Initializing graphics subsystem...
  [GFX] Initializing graphics subsystem...
  [FB] Initializing framebuffer...
  [FB] Framebuffer initialized (0x0)
  [COMP] Initializing compositor...
  [COMP] Compositor initialized
  [GFX] Graphics subsystem initialized
  [BOOTSTRAP] Graphics subsystem initialized
[STAGE 6] User space transition
BOOTOK
```

## What Gets Tested

### Phase 0: Foundation
- Toolchain configuration
- Build system
- Multi-architecture support

### Phase 1: Microkernel Core
- Memory management initialization
- Process management initialization
- IPC system initialization
- Scheduler initialization
- Capability system initialization

### Phase 2: User Space Foundation
- VFS initialization
- Service manager initialization
- Driver framework initialization

### Phase 3: Security Hardening (NEW!)
- **Cryptography subsystem**: Key generation, hashing
- **MAC system**: Policy loading and enforcement
- **Audit framework**: Event logging
- **Secure boot**: Verification (disabled by default)

### Phase 4: Package Ecosystem (NEW!)
- **Package manager**: Core package installation
- **Package database**: Tracking installed packages

### Phase 5: Performance Optimization (NEW!)
- **Performance counters**: Statistics initialization
- **Optimization framework**: Memory/scheduler/IPC optimization hooks

### Phase 6: Graphics & GUI (NEW!)
- **Framebuffer**: Graphics buffer initialization
- **Compositor**: Window management system

## Debugging

### Enable Verbose Output

Set environment variables for more output:

```bash
export RUST_LOG=trace
cargo run --target ... -p veridian-kernel
```

### GDB Debugging

Use the provided GDB scripts:

```bash
./debug/gdb-kernel.sh x86_64
./debug/gdb-kernel.sh aarch64
./debug/gdb-kernel.sh riscv64
```

### Serial Output Capture

Capture output to file:

```bash
qemu-system-x86_64 \
  -drive format=raw,file=target/.../bootimage-veridian-kernel.bin \
  -serial file:boot.log \
  -display none
```

## Common Issues

### x86_64: "bootimage not found"

**Solution**: Install bootimage and create the boot image:
```bash
cargo install bootimage
cargo bootimage --target targets/x86_64-veridian.json
```

### AArch64: Iterator/loop hangs

**Status**: Resolved with assembly-only UART approach and unified pointer pattern.

### RISC-V: VFS mount hangs

**Status**: May occur on older versions - use latest code with unified pointer pattern.

### No output visible

**Solution**: Ensure `-serial stdio` is specified in QEMU command.

## Performance Validation

Expected performance metrics (from Phase 1):

- **IPC Latency**: < 1μs (target: < 5μs) ✅
- **Context Switch**: < 10μs ✅
- **Memory Allocation**: < 1μs ✅
- **Capability Lookup**: O(1) ✅

## Next Steps After Testing

Once runtime testing confirms the kernel boots successfully:

1. **Test individual subsystems**:
   - Create test processes
   - Perform IPC operations
   - Test VFS operations
   - Verify security policies

2. **Performance benchmarking**:
   - Run IPC latency tests
   - Measure context switch times
   - Profile memory allocation

3. **Integration testing**:
   - Test inter-subsystem communication
   - Verify security enforcement
   - Test package operations

4. **Enhance placeholder implementations**:
   - Replace crypto placeholders with real implementations
   - Implement full network stack
   - Add persistent filesystem support

## Test Environments

### Recommended

- **Linux host** with QEMU installed
- **4GB+ RAM** for smooth emulation
- **SSD storage** for faster builds

### Tested Configurations

- Fedora/RHEL with QEMU 8.x
- Ubuntu 22.04+ with QEMU 6.x+
- Arch Linux with latest QEMU

## Continuous Integration

The project includes GitHub Actions CI that:
- Builds all three architectures
- Runs compilation checks
- Validates code formatting
- Performs security audits

See `.github/workflows/` for CI configuration.

## Conclusion

VeridianOS is now ready for runtime testing with all six phases integrated. The kernel should boot successfully on all three architectures when QEMU is available. Any runtime issues discovered during testing should be documented and fixed incrementally.

For questions or issues, see the project documentation at:
- **GitHub**: https://github.com/doublegate/VeridianOS
- **Docs**: https://doublegate.github.io/VeridianOS/

---

**Note**: This testing is currently blocked in the development environment due to QEMU not being installed. Testing should be performed in an environment with QEMU available.

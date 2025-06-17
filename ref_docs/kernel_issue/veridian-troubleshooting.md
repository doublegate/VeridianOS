# VeridianOS Kernel Linking Troubleshooting Guide

**Current Status:** Phase 1 COMPLETE (v0.2.1 - June 17, 2025)
- Latest release: v0.2.1 - Maintenance Release
- All three architectures (x86_64, AArch64, RISC-V) boot to Stage 6
- Zero warnings and clippy-clean across all architectures
- Ready for Phase 2 User Space Foundation development

## Common R_X86_64_32S Relocation Errors

### Understanding the Error

The error `relocation R_X86_64_32S out of range` occurs when:
- Code tries to reference data/code that's more than ±2GB away
- The linker can't fit the address difference in a signed 32-bit value
- Usually happens with higher-half kernels

### Quick Diagnosis

1. **Check your linking address**:
   ```bash
   readelf -S target/x86_64-veridian/debug/veridian-kernel | grep -E "\.text|\.rodata"
   ```
   
2. **Verify relocations**:
   ```bash
   readelf -r target/x86_64-veridian/debug/veridian-kernel | grep R_X86_64_32S
   ```

### Solutions by Scenario

#### Scenario 1: Kernel at 0xFFFFFFFF80000000 (Recommended)

Use the **kernel code model**:
- ✅ Most efficient code generation
- ✅ Supported by standard tooling
- ✅ Used by Linux and most OS kernels

```toml
# In .cargo/config.toml
rustflags = ["-C", "code-model=kernel", "-C", "relocation-model=static"]
```

#### Scenario 2: Kernel beyond ±2GB range

Use the **large code model**:
- ⚠️ Less efficient (uses 64-bit addresses)
- ⚠️ Larger code size
- ✅ Works at any address

```toml
rustflags = ["-C", "code-model=large", "-C", "relocation-model=static"]
```

### Common Pitfalls

1. **Forgetting to rebuild std**:
   ```toml
   [unstable]
   build-std = ["core", "compiler_builtins", "alloc"]
   ```

2. **Mismatched code models**: 
   - Your kernel AND dependencies must use the same code model
   - This is why `build-std` is required

3. **Incorrect linker script addresses**:
   - Virtual and physical addresses must be correctly calculated
   - Use `AT()` directive for physical addresses

### Verification Steps

1. **Check generated code model**:
   ```bash
   objdump -d target/x86_64-veridian/debug/veridian-kernel | grep -E "movabs|lea" | head -20
   ```
   - Kernel model: Uses `lea` with RIP-relative addressing
   - Large model: Uses `movabs` for 64-bit immediates

2. **Verify section addresses**:
   ```bash
   objdump -h target/x86_64-veridian/debug/veridian-kernel
   ```

3. **Check symbol addresses**:
   ```bash
   nm target/x86_64-veridian/debug/veridian-kernel | grep -E "rodata|text|data" | sort
   ```

### Advanced Debugging

1. **Enable verbose linking**:
   ```bash
   RUSTFLAGS="-C link-arg=-Wl,--verbose" cargo build
   ```

2. **Generate map file**:
   ```bash
   RUSTFLAGS="-C link-arg=-Wl,-Map=kernel.map" cargo build
   ```

3. **Check relocation details**:
   ```bash
   readelf -r target/x86_64-veridian/debug/veridian-kernel | grep -B2 -A2 "out of range"
   ```

### Alternative Approaches

1. **Position Independent Code (PIC)**:
   ```json
   "relocation-model": "pic",
   "code-model": "kernel"
   ```
   - More flexible but slightly less efficient

2. **Split kernel sections**:
   - Keep critical code within ±2GB
   - Place large data structures elsewhere

3. **Use mcmodel=medium** (if using gcc directly):
   - Compromise between small and large models
   - Good for mixed code/data sizes

### Getting Help

If you're still having issues:

1. Check exact error addresses:
   - Note the negative number (e.g., -140737486798800)
   - Convert to hex to understand the address

2. Share your:
   - Target JSON file
   - Linker script
   - Cargo.toml configuration
   - Exact error message

3. Verify toolchain versions:
   ```bash
   rustc --version
   cargo --version
   rust-lld --version
   ```
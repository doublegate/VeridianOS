# Kernel Build Troubleshooting Guide

## Common Build Issues and Solutions

### R_X86_64_32S Relocation Errors (x86_64)

#### Symptoms
```
rust-lld: error: relocation R_X86_64_32S out of range: -140737486798800 is not in [-2147483648, 2147483647]
```

#### Root Cause
The kernel is being linked at an address outside the ±2GB range that R_X86_64_32S relocations can handle. This happens when:
- The kernel is linked at a high virtual address (e.g., 0xFFFF800000100000)
- The compiler generates code assuming symbols can be reached with 32-bit signed offsets
- The precompiled `core` library wasn't built with the appropriate code model

#### Solution
1. **Update the linker script** (`kernel/src/arch/x86_64/link.ld`):
   ```ld
   SECTIONS {
       /* Higher half kernel at -2GB (0xFFFFFFFF80000000) */
       . = 0xFFFFFFFF80100000;
   ```

2. **Use the custom target JSON** with kernel code model:
   ```bash
   cargo build --target targets/x86_64-veridian.json -p veridian-kernel -Zbuild-std=core,compiler_builtins,alloc
   ```

3. **Or use the build script**:
   ```bash
   ./build-kernel.sh x86_64 dev
   ```

### Build Script Usage

The project includes `build-kernel.sh` which handles architecture-specific build configurations:

```bash
# Build all architectures
./build-kernel.sh all dev        # Development build
./build-kernel.sh all release    # Release build

# Build specific architecture
./build-kernel.sh x86_64 dev
./build-kernel.sh aarch64 release
./build-kernel.sh riscv64 dev
```

### Architecture-Specific Notes

#### x86_64
- **Requires**: Custom target JSON with kernel code model
- **Linked at**: 0xFFFFFFFF80100000 (top 2GB of virtual memory)
- **Build command**: Uses `-Zbuild-std` to rebuild core with kernel code model
- **Target file**: `targets/x86_64-veridian.json`

#### AArch64
- **Uses**: Standard bare metal target `aarch64-unknown-none`
- **No special configuration needed**
- **Working implementations**: Preserved in `kernel/src/arch/aarch64/working-simple/`

#### RISC-V
- **Uses**: Standard bare metal target `riscv64gc-unknown-none-elf`
- **No special configuration needed**

### Verification Steps

1. **Check build artifacts**:
   ```bash
   ls -la target/x86_64-veridian/debug/veridian-kernel
   ls -la target/aarch64-unknown-none/debug/veridian-kernel
   ls -la target/riscv64gc-unknown-none-elf/debug/veridian-kernel
   ```

2. **Verify linking addresses** (x86_64):
   ```bash
   objdump -h target/x86_64-veridian/debug/veridian-kernel | grep -E "\.text|\.rodata"
   ```
   Should show addresses starting with 0xffffffff8...

3. **Check for relocation issues**:
   ```bash
   readelf -r target/x86_64-veridian/debug/veridian-kernel | grep R_X86_64_32S
   ```

### Clean Build

If you encounter persistent issues:
```bash
cargo clean
./build-kernel.sh all dev
```

### Required Tools

Ensure you have the nightly toolchain with rust-src:
```bash
rustup override set nightly
rustup component add rust-src
```

### Configuration Files

#### `.cargo/config.toml`
```toml
[unstable]
build-std-features = ["compiler-builtins-mem"]
build-std = ["core", "compiler_builtins", "alloc"]

[target.x86_64-veridian]
rustflags = ["-C", "code-model=kernel", "-C", "relocation-model=static"]
```

#### `targets/x86_64-veridian.json`
Key settings:
```json
{
  "code-model": "kernel",
  "relocation-model": "static",
  "disable-redzone": true
}
```

### References
- [OSDev Wiki - Higher Half Kernel](https://wiki.osdev.org/Higher_Half_Kernel)
- [System V ABI x86-64 Supplement](https://refspecs.linuxbase.org/elf/x86_64-abi-0.99.pdf)
- `docs/reference/kernel_issue/` - Additional documentation and solutions
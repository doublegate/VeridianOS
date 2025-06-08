# Troubleshooting Guide

This guide covers common issues encountered when building, running, and developing VeridianOS, along with their solutions.

## Build Issues

### Custom Target Build Failures

**Problem**: Build fails with "can't find crate for `core`" or similar errors.

**Solution**: Custom targets require building the standard library from source:
```bash
cargo build --target targets/x86_64-veridian.json \
    -Zbuild-std=core,compiler_builtins,alloc \
    -Zbuild-std-features=compiler-builtins-mem
```

### Clippy Warnings

**Problem**: Clippy fails with various warnings during CI.

**Common Solutions**:

1. **Format string warnings**: Use inline variables
```rust
// Bad
println!("{}", var);

// Good  
println!("{var}");
```

2. **Dead code warnings**: Add for architecture-specific code
```rust
#[allow(dead_code)]
fn platform_specific_function() { }
```

3. **Unsafe block warnings**: Remove unnecessary unsafe blocks
```rust
// Bad
unsafe {
    unsafe_function();
}

// Good (if already in unsafe context)
unsafe_function();
```

### Missing Cargo.lock

**Problem**: Security audit fails with "no Cargo.lock found".

**Solution**: Ensure Cargo.lock is committed to the repository:
```bash
git add Cargo.lock
git commit -m "Add Cargo.lock for reproducible builds"
```

## Boot Issues

### AArch64 Boot Hang

**Problem**: AArch64 kernel hangs after "Booting kernel..." message.

**Known Issues**:
- Iterator-based code causes hangs on bare metal
- Complex boot code may fail silently

**Solution**: Use simple, direct memory operations:
```rust
// Bad - causes hang
let uart = 0x09000000 as *mut u8;
"HELLO\n".bytes().enumerate().for_each(|(i, byte)| {
    unsafe { uart.add(i).write_volatile(byte); }
});

// Good - works reliably
let uart = 0x09000000 as *mut u8;
unsafe {
    uart.add(0).write_volatile(b'H');
    uart.add(1).write_volatile(b'E');
    // ... etc
}
```

### RISC-V Target Specification

**Problem**: RISC-V build fails with "unknown target triple".

**Solution**: Ensure target JSON includes required fields:
```json
{
    "llvm-target": "riscv64-unknown-none-elf",
    "llvm-abiname": "lp64d",
    // ... other fields
}
```

### Stack Corruption

**Problem**: Random crashes or corrupted output during boot.

**Common Causes**:
1. Stack too small or misaligned
2. Stack overlapping with other memory regions
3. Missing stack initialization

**Solution**: Ensure proper stack setup in boot assembly:
```asm
# AArch64 example
.section .boot, "ax"
.global _start
_start:
    # Set stack pointer to known good address
    ldr x30, =0x80000
    mov sp, x30
    
    # Clear BSS section
    ldr x0, =__bss_start
    ldr x1, =__bss_end
1:  
    str xzr, [x0], #8
    cmp x0, x1
    b.lo 1b
    
    # Call Rust entry point
    bl boot_main
```

## Debugging Issues

### GDB String Arguments

**Problem**: GDB commands fail with "No symbol X in current context".

**Solution**: Quote string arguments in custom GDB commands:
```gdb
# Bad
kernel-symbols kernel

# Good  
kernel-symbols "kernel"
```

### Debug Symbol Extraction

**Problem**: Debug symbols can't be extracted from kernel binary.

**Solution**: Use fallback strategy:
```bash
# Try rust-objcopy first
rust-objcopy --only-keep-debug kernel kernel.debug

# If that fails, use system objcopy
objcopy --only-keep-debug kernel kernel.debug
```

### Remote GDB Connection

**Problem**: GDB can't connect to QEMU gdbserver.

**Solution**: Ensure QEMU is started with correct flags:
```bash
qemu-system-x86_64 \
    -s \              # Enable gdbserver on port 1234
    -S \              # Start paused
    -kernel kernel \
    -display none \
    -serial stdio
```

## CI/CD Issues

### GitHub Actions Failures

**Problem**: CI passes locally but fails on GitHub.

**Common Causes**:
1. Missing tools in CI environment
2. Different dependency versions
3. Platform-specific behavior

**Solutions**:

1. **Install required tools**:
```yaml
- name: Install tools
  run: |
    sudo apt-get update
    sudo apt-get install -y llvm binutils
```

2. **Use exact dependency versions**:
```toml
[dependencies]
bootloader = "=0.11.0"  # Exact version
```

3. **Handle platform differences**:
```rust
#[cfg(target_os = "linux")]
fn platform_specific() { }
```

### Pages Deployment

**Problem**: GitHub Pages deployment fails with "not configured".

**Solution**: Use conditional deployment:
```yaml
- name: Deploy to Pages
  uses: actions/deploy-pages@v4
  continue-on-error: true
  if: github.event_name == 'push' && github.ref == 'refs/heads/main'
```

### Artifact Upload

**Problem**: Release artifacts not available after CI build.

**Solution**: Download and upload artifacts to release:
```bash
# Download artifacts from CI run
gh run download <run-id> --dir release-artifacts

# Upload to release
gh release upload v0.1.0 release-artifacts/* --clobber
```

## Development Environment

### Rust Toolchain

**Problem**: Build requires specific nightly features.

**Solution**: Use rust-toolchain.toml:
```toml
[toolchain]
channel = "nightly-2025-01-15"
components = ["rust-src", "llvm-tools-preview"]
targets = ["x86_64-unknown-linux-gnu"]
```

### Missing Build Dependencies

**Problem**: Build fails with missing system libraries.

**Solution**: Install development packages:
```bash
# Debian/Ubuntu
sudo apt-get install build-essential clang llvm

# Fedora
sudo dnf install gcc clang llvm-devel

# macOS
brew install llvm
```

### Workspace Issues

**Problem**: Cargo can't find workspace members.

**Solution**: Ensure Cargo.toml lists all members:
```toml
[workspace]
members = [
    "kernel",
    "bootloader",
    # Add all crate directories
]
```

## Runtime Issues

### Serial Output Not Appearing

**Problem**: No output from kernel despite successful boot.

**Possible Causes**:
1. Wrong serial port address
2. Serial port not initialized
3. Output buffering issues

**Solution**: Verify serial configuration:
```rust
// x86_64
const SERIAL_PORT: u16 = 0x3F8;

// AArch64  
const UART_BASE: usize = 0x09000000;

// Ensure volatile writes
unsafe {
    ptr::write_volatile(uart_addr, byte);
}
```

### Memory Allocation Failures

**Problem**: Kernel panics during memory allocation.

**Common Causes**:
1. Heap not initialized
2. Heap too small
3. Corrupted allocator state

**Solution**: Ensure proper heap initialization:
```rust
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub fn init_heap() {
    let heap_start = 0x_4444_4444_0000;
    let heap_size = 100 * 1024; // 100 KiB
    
    unsafe {
        ALLOCATOR.lock().init(heap_start as *mut u8, heap_size);
    }
}
```

## Performance Issues

### Slow Build Times

**Problem**: Builds take excessive time.

**Solutions**:

1. **Use incremental compilation**:
```toml
[profile.dev]
incremental = true
```

2. **Reduce optimization in dev**:
```toml
[profile.dev]
opt-level = 0
```

3. **Use mold linker**:
```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

### High Memory Usage

**Problem**: Development environment uses too much RAM.

**Solutions**:

1. **Limit parallel jobs**:
```bash
cargo build -j 4
```

2. **Use cargo-hakari** for workspace optimization
3. **Split large crates into smaller ones**

## Testing Issues

### Test Failures in CI

**Problem**: Tests pass locally but fail in CI.

**Common Causes**:
1. Race conditions
2. Hardcoded paths
3. Missing test fixtures

**Solutions**:

1. **Use proper synchronization**:
```rust
use std::sync::Once;

static INIT: Once = Once::new();

#[test]
fn test_requiring_init() {
    INIT.call_once(|| {
        initialize_test_environment();
    });
    // ... test code
}
```

2. **Use relative paths**:
```rust
let test_file = env::current_dir()
    .unwrap()
    .join("tests/fixtures/test.txt");
```

### Integration Test Timeouts

**Problem**: Integration tests timeout in CI.

**Solution**: Increase timeout and add progress reporting:
```rust
#[tokio::test(flavor = "multi_thread")]
#[timeout(Duration::from_secs(60))]
async fn long_running_test() {
    for i in 0..100 {
        do_work(i).await;
        if i % 10 == 0 {
            eprintln!("Progress: {}/100", i);
        }
    }
}
```

## Common Error Messages

### "can't find crate for `std`"

**Meaning**: Trying to use std in no_std environment.

**Solution**: Use core and alloc instead:
```rust
#![no_std]
#![no_main]

extern crate alloc;

use core::mem;
use alloc::vec::Vec;
```

### "error: requires `start` lang item"

**Meaning**: Missing entry point for no_std binary.

**Solution**: Define panic handler and start:
```rust
#[no_mangle]
pub extern "C" fn _start() -> ! {
    main();
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // Handle panic
    loop {}
}
```

### "LLVM ERROR: Do not know how to split this operator's operand!"

**Meaning**: LLVM codegen issue with certain Rust patterns.

**Solution**: Simplify the code or use different approach:
```rust
// May cause issues
let complex = (a + b) * (c + d) / (e + f);

// Better
let sum1 = a + b;
let sum2 = c + d;
let sum3 = e + f;
let complex = (sum1 * sum2) / sum3;
```

## Getting Help

### Debug Information

When reporting issues, include:

1. **System information**:
```bash
uname -a
rustc --version
cargo --version
```

2. **Full error output**:
```bash
cargo build --verbose 2>&1 | tee build.log
```

3. **Minimal reproduction**:
- Smallest code that triggers the issue
- Exact commands to reproduce
- Expected vs actual behavior

### Community Resources

- **GitHub Issues**: [github.com/doublegate/VeridianOS/issues](https://github.com/doublegate/VeridianOS/issues)
- **Discord**: Join our development Discord
- **IRC**: #veridian on irc.libera.chat
- **Mailing List**: veridian-dev@lists.veridian.org

### Known Issues

Check the [ISSUES_TODO.md](https://github.com/doublegate/VeridianOS/blob/main/to-dos/ISSUES_TODO.md) file for:
- Currently open issues
- Resolved issues and their fixes
- Workarounds for known problems
- Platform-specific quirks
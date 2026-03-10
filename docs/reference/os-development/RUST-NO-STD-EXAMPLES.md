# Rust no_std OS Development Examples

This document contains examples and patterns from the Rust documentation for OS development.

## Minimal no_std Application

### Basic Structure
```rust
#![no_std]
#![no_main]
#![feature(lang_items)]

use core::panic::PanicInfo;

#[no_mangle]
pub extern "C" fn main(_argc: core::ffi::c_int, _argv: *const *const u8) -> core::ffi::c_int {
    // Kernel initialization
    0
}

#[panic_handler]
fn panic(_panic: &PanicInfo<'_>) -> ! {
    loop {}
}

#[lang = "eh_personality"]
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}
```

## Architecture-Specific Examples

### x86_64 with UEFI
```rust
#![no_main]
#![no_std]

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[export_name = "efi_main"]
pub extern "C" fn main(_h: *mut core::ffi::c_void, _st: *mut core::ffi::c_void) -> usize {
    0
}
```

### AArch64/RISC-V Halt Implementation
```rust
pub fn halt() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") };  // Wait For Interrupt
    }
}
```

## Building with Custom Targets

### Build Commands
```bash
# Build with custom target and -Zbuild-std
cargo build --target targets/x86_64-veridian.json -p veridian-kernel \
    -Zbuild-std=core,compiler_builtins,alloc \
    -Zbuild-std-features=compiler-builtins-mem

# For nightly features
cargo +nightly build -Z build-std=core --target x86_64-unknown-none
```

### Cargo Configuration (.cargo/config.toml)
```toml
[build]
target = "x86_64-unknown-none"
rustflags = ["-C", "target-cpu=native"]

[unstable]
build-std = ["core", "compiler_builtins", "alloc"]
```

## Memory Management

### Custom Box Implementation
```rust
#![feature(lang_items, core_intrinsics, rustc_private)]

extern crate libc;

use core::ffi::c_void;
use core::intrinsics;

#[lang = "exchange_malloc"]
unsafe fn allocate(size: usize, _align: usize) -> *mut u8 {
    let p = libc::malloc(size) as *mut u8;
    if p.is_null() {
        intrinsics::abort();
    }
    p
}
```

## QEMU Testing

### Running x86_64 Kernel
```bash
qemu-system-x86_64 -monitor none -display none -kernel ./kernel.bin

# With debugging
qemu-system-x86_64 -s -S -kernel ./kernel.bin
```

### Running AArch64
```bash
qemu-system-aarch64 -M virt -cpu cortex-a57 -kernel ./kernel.bin
```

### Running RISC-V
```bash
qemu-system-riscv64 -M virt -kernel ./kernel.bin
```

## Target Specification Notes

Key fields for custom targets:
- `"panic-strategy": "abort"` - No unwinding in kernel
- `"disable-redzone": true` - Required for interrupt safety
- `"features": "-mmx,-sse,+soft-float"` - Disable hardware floating point
- `"executables": true` - Can build executables
- `"linker-flavor": "ld.lld"` - Use LLVM linker

## Debugging Tips

1. Use `rustc --print target-spec-json -Z unstable-options --target <builtin>` to check format
2. For RISC-V: Use `"llvm-abiname": "lp64d"` not `"abi"`
3. LLVM targets should match Rust's built-in format (e.g., "riscv64" not full triple)

## References

- Rust Embedded Book: https://doc.rust-lang.org/embedded-book/
- OS Dev Wiki: https://wiki.osdev.org/
- QEMU Documentation: https://www.qemu.org/docs/master/
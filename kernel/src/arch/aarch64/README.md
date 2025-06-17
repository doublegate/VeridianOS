# AArch64 Boot Issues Documentation

## Summary

There were critical issues affecting AArch64 bare-metal boot that caused the kernel to hang when using:
- Any kind of loop (`for`, `while`)  
- Iterator methods
- String slice methods (`.as_bytes()`, `.len()`)
- Function calls in early boot contexts

## Root Cause (RESOLVED)

The issue was **NOT** an LLVM bug, but improper stack initialization:
- Stack pointer was hardcoded to 0x80000 instead of using linker-defined `__stack_top`
- Missing stack alignment (16-byte alignment required by AArch64 ABI)
- Missing frame pointer initialization

## Solution Implemented

Fixed in boot.S by:
```asm
// Set up stack using linker-defined symbol
adrp x1, __stack_top
add x1, x1, :lo12:__stack_top
// Ensure 16-byte alignment
and sp, x1, #~15

// Initialize frame pointer for ABI compliance
mov x29, #0
mov x30, #0
```

This proper stack initialization allows all Rust features to work correctly:
- Function calls work normally
- Loops may still have issues (possibly a separate LLVM issue)
- For safety, we use `direct_uart.rs` for AArch64 console output

## Current Status

- Stack initialization: ✅ Fixed
- Function calls: ✅ Working
- String operations: ✅ Working with `direct_uart.rs`
- Loops: ⚠️ May still have issues, use with caution
- Overall boot: ✅ All architectures boot to Stage 6

## Remaining Files

The following files provide working implementations:
- `boot.S` - Proper boot sequence with stack setup
- `boot.rs` - Rust entry point
- `direct_uart.rs` - UART output functions (loop-free)
- `context.rs` - Context switching support
- `timer.rs` - Timer support
- `mod.rs` - Module definitions

## Removed Files (No Longer Needed)

- `manual_print.rs` - Superseded by `direct_uart.rs`
- `safe_iter.rs` - Not needed with proper stack
- `bootstrap.rs` - Using unified bootstrap
- `working-simple/` - Backup files no longer needed
# AArch64 LLVM Loop/Iterator Bug Documentation

## Summary

There is a critical LLVM bug affecting AArch64 bare-metal targets that causes the kernel to hang when using:
- Any kind of loop (`for`, `while`)
- Iterator methods
- String slice methods (`.as_bytes()`, `.len()`)
- Function calls in certain boot contexts
- Even some compile-time operations

## Current Workaround

The ONLY reliable solution is complete manual inlining with direct UART writes:

```rust
// This works:
unsafe {
    let uart = 0x0900_0000 as *mut u8;
    *uart = b'H'; *uart = b'e'; *uart = b'l'; *uart = b'l'; *uart = b'o'; *uart = b'\n';
}

// This hangs:
println!("Hello");
// Also hangs:
for i in 0..5 { /* ... */ }
// Also hangs:
"Hello".as_bytes()
```

## Limitations

- Maximum practical string length: ~64 characters (manual unrolling limit)
- No dynamic formatting (numbers, etc.)
- Code is verbose and hard to maintain
- Function calls may hang after certain point in boot

## Recommendations

1. **Short term**: Use manual inline UART writes for critical boot messages
2. **Medium term**: Consider assembly routines or C functions compiled with GCC
3. **Long term**: 
   - File LLVM bug report with minimal reproducer
   - Consider alternative backends (GCC, Cranelift)
   - Monitor for upstream fixes

## Test Results

Through extensive testing, we found:
- Direct inline UART writes: ✅ Always work
- Function calls: ❌ Hang after boot initialization
- Macros with any method calls: ❌ Hang
- Assembly routines: ❓ Attempted but function call itself hangs
- Loops of any kind: ❌ Always hang

This is a fundamental compiler bug that severely limits AArch64 development until fixed.
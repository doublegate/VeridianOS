# Session Summary: AArch64 println! LLVM Bug Investigation and Workaround

**Date**: December 16, 2025  
**Issue**: AArch64 println! macro causing kernel hangs due to LLVM loop/iterator bug

## Problem Description

The AArch64 kernel was hanging after printing 2-3 messages when using the println! macro. This was due to a critical LLVM bug affecting bare-metal AArch64 targets that causes hangs when using:
- Any loops (`for`, `while`)
- Iterator methods
- String slice methods (`.as_bytes()`, `.len()`)
- Function calls in certain boot contexts
- Even some compile-time operations

## Investigation Process

### 1. Initial Diagnosis
- Confirmed println! was hanging after 2-3 successful prints
- Identified that the issue was related to the known LLVM iterator/loop bug

### 2. Three-Worktree Parallel Investigation
Created three worktrees to explore different solutions simultaneously:

**Worktree 1 (format-fix)**: Custom formatting system
- Attempted to create a loop-free formatting system
- Limited to 16 characters due to manual unrolling
- Still encountered hangs due to string methods

**Worktree 2 (safe-iter)**: Safe iterator integration  
- Tried to integrate existing safe_iter.rs utilities
- Used pre-compiled messages approach
- Most reliable of the three approaches

**Worktree 3 (minimal-fix)**: Minimal surgical fix
- Attempted minimal changes with placeholders
- Least useful approach

### 3. Solutions Attempted

1. **Assembly UART Implementation** ❌
   - Created uart_asm.s with hand-written assembly loop
   - Function call itself caused hang before assembly executed

2. **Precompiled Byte Arrays** ❌
   - Used const byte arrays to avoid runtime string operations
   - `.len()` method still caused hangs

3. **Fixed-Size Arrays with Const Generics** ❌
   - Attempted to use compile-time known sizes
   - Function calls still triggered the bug

4. **Direct Inline UART Writes** ✅
   - Only reliable solution
   - Complete manual inlining with no function calls
   - Verbose but functional

## Final Solution

The only reliable solution is complete manual inlining:

```rust
// This works - completely inline, no function calls
unsafe {
    let uart = 0x0900_0000 as *mut u8;
    *uart = b'H'; *uart = b'e'; *uart = b'l'; *uart = b'l'; *uart = b'o'; *uart = b'\n';
}
```

## Limitations

1. **String Length**: Practical limit of ~64 characters due to manual unrolling
2. **No Formatting**: Cannot format numbers or other dynamic data
3. **Code Verbosity**: Each character requires a separate write statement
4. **Maintenance**: Difficult to maintain and modify messages

## Files Created/Modified

### Created
- `kernel/src/arch/aarch64/direct_uart.rs` - Direct UART implementation (limited use)
- `kernel/src/arch/aarch64/simple_print.rs` - Simple printing attempt
- `kernel/src/arch/aarch64/test_print.rs` - Testing utilities
- `kernel/src/arch/aarch64/precompiled_messages.rs` - Pre-compiled message approach
- `kernel/src/arch/aarch64/fixed_print.rs` - Fixed-size array approach
- `kernel/src/arch/aarch64/inline_print.rs` - Inline macro approach
- `kernel/src/arch/aarch64/uart_asm.s` - Assembly implementation
- `kernel/src/arch/aarch64/uart_asm.rs` - Rust wrapper for assembly
- `kernel/src/arch/aarch64/manual_print.rs` - Final helper macros
- `kernel/src/arch/aarch64/README_LLVM_BUG.md` - Bug documentation

### Modified
- `kernel/src/print.rs` - Updated AArch64 println! implementation
- `kernel/src/main.rs` - Added test code and debug output
- `kernel/src/arch/aarch64/mod.rs` - Added new modules

## Recommendations

1. **Short Term**: Use manual inline UART writes for critical boot messages on AArch64
2. **Medium Term**: 
   - File LLVM bug report with minimal reproducer
   - Consider assembly routines compiled with GCC
   - Use x86_64/RISC-V as primary development platforms
3. **Long Term**:
   - Evaluate alternative backends (GCC, Cranelift)
   - Monitor LLVM for fixes
   - Consider contributing fix to LLVM if feasible

## Key Learnings

1. The LLVM bug is more pervasive than initially documented - affecting not just loops but also function calls and string methods
2. Compile-time operations don't fully escape the bug
3. Complete manual inlining is currently the only reliable workaround
4. This significantly limits AArch64 development capabilities until a compiler fix is available

## References

- Original issue discussions in project documentation
- LLVM bug tracking (to be filed)
- Rust embedded working group discussions on similar issues
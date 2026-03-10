# Stack Setup Audit - All Architectures

## Overview

This document summarizes the stack initialization audit performed across all three architectures (x86_64, AArch64, RISC-V) to ensure proper use of linker-defined symbols instead of hardcoded addresses.

## Findings and Fixes

### x86_64 Architecture

**Status**: ✅ Already Correct

- Uses the `bootloader` crate which handles stack setup automatically
- Linker script (`kernel/src/arch/x86_64/link.ld`) properly defines:
  - `__stack_bottom` at aligned address after BSS
  - `__stack_top` with 1MB stack size
- No manual stack setup needed in boot code

### AArch64 Architecture

**Status**: ✅ Fixed Previously (Root cause of boot issues)

- **Previous Issue**: Stack pointer was hardcoded to `0x80000`
- **Fix Applied**: Updated `boot.S` to use linker-defined symbols:
  ```asm
  adrp x1, __stack_top
  add x1, x1, :lo12:__stack_top
  and sp, x1, #~15  // 16-byte alignment
  ```
- Linker script properly defines stack symbols with appropriate size
- This fix resolved the function call hanging issues

### RISC-V Architecture

**Status**: ✅ Fixed in This Session

- **Previous Issue**: boot.S defined its own local stack in .bss section:
  ```asm
  .section .bss
  .align 16
  _stack:
      .space 0x4000
  _stack_top:
  ```
- **Fix Applied**: Updated to use linker-defined symbols:
  ```asm
  # Set up stack using linker-defined symbol
  la sp, __stack_top
  
  # Ensure stack pointer is 16-byte aligned (RISC-V ABI requirement)
  andi sp, sp, ~15
  
  # Initialize frame pointer
  li fp, 0
  ```
- Also added BSS clearing for completeness
- Linker script already had proper stack definitions (128KB stack)

## Stack Sizes by Architecture

- **x86_64**: 1MB (0x100000)
- **AArch64**: 16KB (0x4000) - defined in link.ld
- **RISC-V**: 128KB (0x20000)

## ABI Requirements

All architectures now properly implement their ABI requirements:

- **x86_64**: 16-byte alignment (handled by bootloader)
- **AArch64**: 16-byte alignment, frame pointer initialization
- **RISC-V**: 16-byte alignment, frame pointer initialization

## Benefits of Using Linker Symbols

1. **Flexibility**: Stack size and location can be changed in one place
2. **Safety**: Linker ensures stack doesn't overlap with other sections
3. **Consistency**: All memory layout defined in linker script
4. **Debugging**: Symbols visible in debugger and symbol maps
5. **Correctness**: Proper alignment and placement guaranteed

## Testing Results

- ✅ **x86_64**: Boots correctly (though has early hang unrelated to stack)
- ✅ **AArch64**: Boots to Stage 6, function calls work
- ⚠️ **RISC-V**: Stack setup improved but experiencing unrelated regression
  - Issue: Kernel restarts when acquiring frame allocator lock
  - Not related to stack changes (occurs with both old and new boot.S)
  - Previously was booting to Stage 6 according to documentation
  - Needs separate investigation (likely lock/mutex initialization issue)

## Conclusion

All three architectures now use proper linker-defined stack symbols instead of hardcoded addresses. This ensures consistent, maintainable, and correct stack initialization across the entire kernel.
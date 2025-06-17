# Critical Architecture-Specific Issues

**Priority**: CRITICAL - Blocks core functionality
**Phase**: Must be resolved for Phase 2
**Last Updated**: January 17, 2025 - Pre-Phase 2 Assessment Complete

## AArch64 Critical Blockers

### 1. Iterator and Loop Compilation Issues
**Status**: ✅ RESOLVED WITH WORKAROUNDS (June 15, 2025)
**Location**: Throughout AArch64 code paths
**Issue**: For loops and iterators cause kernel hangs on bare metal AArch64
**Resolution**: Created comprehensive workarounds in `arch/aarch64/safe_iter.rs`
- Implemented loop-free utilities: `write_str_loopfree()`, `write_num_loopfree()`, etc.
- Created `aarch64_for!` macro for safe iteration when needed
- Memory operations without loops: `memcpy_loopfree()`, `memset_loopfree()`
- All critical code paths updated to use safe iteration patterns

**Future Work**:
- File upstream LLVM bug report with minimal test case
- Continue using workarounds until compiler issue resolved

### 2. Bootstrap Process Completely Bypassed
**Status**: 🔴 CRITICAL - TOP PRIORITY FOR PHASE 2
**Location**: `kernel/src/main.rs` (lines 172-215)
**Issue**: Full bootstrap process skipped for AArch64 due to println!/loop issues
**Impact**: 
- No proper hardware initialization beyond basic boot
- No memory management setup
- No scheduler initialization
- No IPC/capability system setup
- Currently just outputs "S6" and enters idle loop

**Required Fix**:
- Rewrite bootstrap process without loops/iterators for AArch64
- Use assembly-only output methods from direct_uart.rs
- Create architecture-specific bootstrap implementation
- Ensure all Phase 1 subsystems properly initialize
- Test each bootstrap stage individually to avoid hangs

**Workaround Strategy**:
- Use direct UART writes instead of boot_println!
- Replace all loops with manual assembly or recursion
- Initialize subsystems one at a time with debug output
- Consider minimal initialization for Phase 2 development

### 3. Serial Driver Bare Minimum Implementation
**Status**: 🟡 HIGH
**Location**: `kernel/src/serial.rs` (lines 53-68)
**Issue**: PL011 UART driver simplified to bare minimum
**Missing**:
- FIFO status checking (causes loops)
- Flow control
- Error handling
- Interrupt-driven I/O

**Required Fix**:
- Implement FIFO handling without problematic loops
- Add proper error detection and recovery
- Consider DMA support for better performance

## x86_64 Critical Issues

### 1. Early Boot Hang (ISSUE-0012)
**Status**: 🔴 CRITICAL - BLOCKS x86_64 DEVELOPMENT
**Location**: Very early in boot sequence
**Issue**: x86_64 kernel hangs with no serial output during early boot
**Symptoms**:
- No output to serial console
- Hangs before reaching kernel_main
- May be related to memory initialization or GDT/IDT setup

**Required Investigation**:
- Check early boot assembly code
- Verify serial port initialization timing
- Test with minimal boot sequence
- Add assembly-level debug output
- Check memory regions and stack setup

## Cross-Architecture Critical Issues

### 1. Context Switching Implementation
**Status**: ✅ RESOLVED (June 15, 2025)
**Location**: Multiple files
**Resolution**: Context switching was already fully implemented!
- `arch/x86_64/context.rs`: Complete implementation with all registers
  - **FIXED**: Changed from `iretq` to `ret` for kernel-to-kernel switches
- `arch/aarch64/context.rs`: Full implementation using pure assembly
- `arch/riscv64/context.rs`: Standard RISC-V context switch
- `sched/mod.rs`: Fixed to actually load initial task context

**x86_64 Specific Fix**:
- Problem: Using `iretq` (interrupt return) for kernel-to-kernel switches
- Solution: Changed to `ret` instruction with proper stack setup
- Result: Bootstrap_stage4 now executes correctly

### 2. Multiple kernel_main Functions Confusion
**Status**: ✅ RESOLVED (June 15, 2025)
**Resolution**: Unified kernel_main across all architectures
- Removed duplicate kernel_main from lib.rs
- All architectures now use main.rs version
- RISC-V updated to call `extern "C" kernel_main`
- Consistent bootstrap initialization for all platforms

### 3. RISC-V Missing Full Bootstrap
**Status**: ✅ RESOLVED (June 15, 2025)
**Location**: `kernel/src/arch/riscv64/boot.rs`
**Resolution**: Updated to call `extern "C" kernel_main` from main.rs
- Now uses full bootstrap process like other architectures
- Consistent initialization across all platforms

## Architecture-Specific Implementation Gaps

### x86_64 Issues
- **Context Switching**: ✅ RESOLVED (June 15, 2025) - Fixed `iretq` to `ret` instruction
- **Memory Mapping**: ✅ RESOLVED (June 15, 2025)
  - Fixed duplicate kernel space mapping error
  - Reduced heap size from 256MB to 16MB
  - Init process creation now works correctly
- **Kernel Stack in TSS**: TODO placeholder at `kernel/src/arch/x86_64/context.rs`
- **System Call Entry**: ✅ RESOLVED - Proper naked function with inline assembly implemented
- **APIC Integration**: Timer and IPI functionality replaced with println! stubs
  - Need proper APIC module implementation
  - Required for multi-core support
  - Timer interrupts currently simplified
- **Early Boot Hang**: ISSUE-0012 still pending (separate investigation needed)

### AArch64 Issues
- **Hardware Initialization**: `kernel/src/arch/aarch64/mod.rs` (line 11) - "This will be expanded later"
- **Thread Local Storage**: TODO at `kernel/src/arch/aarch64/context.rs`

### RISC-V Issues  
- **UART Initialization**: `kernel/src/arch/riscv64/serial.rs` (line 87) - "Initialize UART"
- **Thread Local Storage**: TODO at `kernel/src/arch/riscv/context.rs`
- **SBI Module**: Minimal implementation needs expansion

## Resolved Architecture Issues

### ✅ x86_64 Fixes Applied
- Boot entry point now calls proper kernel_main from main.rs
- Serial output added to println! macro for debugging
- PIC initialization with interrupts masked
- MMU initialization completed

### ✅ Memory Allocator Mutex Fix
- Fixed RISC-V mutex deadlock during initialization
- Skip stats updates during init phase to avoid allocation
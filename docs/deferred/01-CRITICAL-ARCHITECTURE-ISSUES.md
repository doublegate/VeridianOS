# Critical Architecture-Specific Issues

**Priority**: CRITICAL - Blocks core functionality
**Phase**: Must be resolved for Phase 2

## AArch64 Critical Blockers

### 1. Iterator and Loop Compilation Issues
**Status**: 🔴 CRITICAL BLOCKER
**Location**: Throughout AArch64 code paths
**Issue**: For loops and iterators cause kernel hangs on bare metal AArch64
**Current Workarounds**:
- `kernel/src/lib.rs` (lines 74-114): kernel_main uses direct UART writes character by character
- `kernel/src/main.rs` (lines 84-136): Manual character writes instead of string loops
- `kernel/src/serial.rs` (lines 53-68): Pl011Uart::write_str uses while loops with manual indexing

**Required Fix**:
- Investigate root cause of iterator codegen issues on AArch64
- May be related to LLVM optimizations or missing memory barriers
- Consider custom iterator implementations for bare metal
- Test with different optimization levels and LLVM versions

### 2. Bootstrap Process Completely Bypassed
**Status**: 🔴 CRITICAL
**Location**: `kernel/src/main.rs` (lines 172-215)
**Issue**: Full bootstrap process skipped for AArch64 due to println!/loop issues
**Impact**: 
- No proper hardware initialization beyond basic boot
- No memory management setup
- No scheduler initialization
- No IPC/capability system setup

**Required Fix**:
- Rewrite bootstrap process without loops/iterators for AArch64
- Create architecture-specific bootstrap implementation
- Ensure all Phase 1 subsystems properly initialize

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

## Cross-Architecture Critical Issues

### 1. Context Switching Not Implemented
**Status**: 🔴 CRITICAL BLOCKER
**Location**: Multiple files
**Critical Gap**: No actual CPU context switching despite scheduler infrastructure
**Current Workaround**: `kernel/src/sched/mod.rs` (lines 575-590) - idle loop instead

**Required Implementation**:
- `arch/x86_64/context.rs`: Actual assembly for context save/restore
- `arch/aarch64/context.rs`: Context switch implementation
- `arch/riscv64/context.rs`: Context switch implementation
- `sched/scheduler.rs`: switch_to() must call arch-specific implementations
- Proper task state preservation across switches
- FPU/vector state handling

### 2. Multiple kernel_main Functions Confusion
**Status**: 🟡 HIGH
**Locations**: 
- `kernel/src/lib.rs`: Simplified test version
- `kernel/src/main.rs`: Full version with bootstrap

**Issues**:
- Different architectures call different versions
- RISC-V still using simplified version
- Inconsistent initialization across architectures

**Required Fix**:
- Remove lib.rs kernel_main
- Ensure all boot code calls main.rs version
- Standardize entry point across architectures

### 3. RISC-V Missing Full Bootstrap
**Status**: 🟡 HIGH
**Location**: `kernel/src/arch/riscv64/boot.rs`
**Issue**: Calls crate::kernel_main() which is the simplified version
**Fix**: Update to call extern "C" kernel_main like x86_64

## Architecture-Specific Implementation Gaps

### x86_64 Issues
- **Kernel Stack in TSS**: TODO placeholder at `kernel/src/arch/x86_64/context.rs`
- **System Call Entry**: ✅ RESOLVED - Proper naked function with inline assembly implemented
- **APIC Integration**: Timer and IPI functionality replaced with println! stubs
  - Need proper APIC module implementation
  - Required for multi-core support
  - Timer interrupts currently simplified

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
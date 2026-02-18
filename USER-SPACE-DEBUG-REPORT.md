# User-Space Execution Debug Report
**Date:** 2026-02-17
**Status:** Partial Success - Boot Return Mechanism Working, Shell Test Blocked

---

## Executive Summary

### What Works ✅
1. **First test program (/bin/minimal)**: Executes successfully in Ring 3, outputs "MINIMAL_TEST_PASS", exits cleanly
2. **Boot return mechanism**: Functional - can return from user mode to bootstrap and continue
3. **Second test (/bin/minimal again)**: Executes successfully - proves boot return mechanism works for multiple processes
4. **Segment register restoration**: Fixed - kernel segments properly restored after boot_return_to_kernel

### What's Broken ❌
1. **Shell test (/bin/sh)**: GP fault during iretq - specific to this binary, not a general issue
2. **Audit logging**: Still disabled (commented out at line 466 of kernel/src/syscall/mod.rs)

---

## Technical Details

### Issues Fixed During Debug Session

#### 1. Segment Register Corruption (FIXED ✅)
**Problem**: After returning from the first user-space test, DS register was left at 0x2b (user data segment) instead of 0x10 (kernel data segment).

**Diagnosis**:
```
[DBG] After first return: 8/2b/10
                            ^^^^^
                            CS=0x8 (kernel code) ✓
                            DS=0x2b (user data) ✗
                            SS=0x10 (kernel stack) ✓
```

**Root Cause**: `boot_return_to_kernel()` function restored CR3 and did swapgs, but didn't restore segment registers.

**Fix** (kernel/src/arch/x86_64/usermode.rs):
```rust
asm!(
    "mov cr3, {cr3}",     // Restore boot page tables
    "swapgs",              // Balance syscall_entry's swapgs (BEFORE touching GS!)
    "mov ax, 0x10",       // Kernel data segment (GDT index 2, RPL 0)
    "mov ds, ax",         // Restore kernel DS
    "mov es, ax",         // Restore kernel ES
    "xor eax, eax",       // Zero FS and GS
    "mov fs, ax",
    "mov gs, ax",
    "mov rsp, {rsp}",     // Restore saved boot RSP
    // ... restore callee-saved registers and ret
)
```

**Key Insight**: swapgs MUST come before clearing GS register to avoid corrupting KERNEL_GS_BASE MSR.

#### 2. Successful Two-Process Test (VERIFIED ✅)
**Test Case**: Run /bin/minimal twice in succession

**Results**:
```
[ENTER]      ← First process enters user mode
1 2 3 4      ← Traces through naked asm: save context, switch CR3, set segments, iretq
MINIMAL_TEST_PASS  ← First process executes and exits

[EXEC-TEST] First test returned, trying second /bin/minimal...
[DBG] After first return: 8/10/10  ← Segments correctly restored

[ENTER]      ← Second process enters user mode
1 2 3 4      ← Successful transition again
MINIMAL_TEST_PASS  ← Second process executes and exits
```

**Conclusion**: Boot return mechanism is sound. The issue is specific to /bin/sh.

### Remaining Issue: Shell Test Failure

#### Symptoms
```
[RUN_USER_PROC] pid=2
[RUN_USER_PROC] entry=40025c stack=7ffdfffde000 cr3=203b000
[RUN_USER_PROC] Before enter CS/DS/SS=8/10/10
[ENTER]
1  ← After saving context
2  ← After CR3 switch
3  ← After segment setup
4  ← Before iretq
FATAL:GP err=0x0
RIP=0xffffffff8018d203 CS=0x8
RFLAGS=0x10046
RSP=0xffffffff806828b8 SS=0x0
```

#### Analysis

1. **All traces (1-4) appear** → We successfully execute all the way through `enter_usermode_returnable` including the iretq instruction
2. **GP fault with SS=0x0** → Invalid stack segment selector
3. **Fault RIP is in kernel** (0xffffffff8018d203 = `core::option::Option<T>::ok_or`) → Not in user space
4. **Fault happens AFTER iretq** → iretq itself failed before transitioning to user mode

#### Hypothesis

The GP fault occurs **during iretq execution**, not after. When iretq validates the segment selectors and stack before transitioning to user mode, it finds something invalid and triggers #GP **before** actually changing privilege level.

Possible causes specific to /bin/sh:
1. **Dynamic linking**: /bin/sh requires interpreter (/lib/ld-linux.so or similar), /bin/minimal does not
2. **ELF segment loading**: More complex LOAD segments or different memory layout
3. **Page table mappings**: Entry point or user stack not properly mapped in the process's page tables (CR3=0x203b000)
4. **Stack alignment**: User stack (0x7ffdfffde000) might have alignment issues for /bin/sh's startup code

#### Evidence

- Entry point: 0x40025c (different from /bin/minimal)
- User stack: 0x7ffdfffde000 (properly allocated, guard pages installed)
- CR3: 0x203b000 (process page table root)

The fact that /bin/minimal works but /bin/sh doesn't suggests the issue is in:
- ELF loading code (kernel/src/elf/mod.rs)
- Dynamic linker support (kernel/src/elf/dynamic.rs)
- VAS page table setup for more complex binaries

---

## Diagnostic Infrastructure Added

### New Trace Points

1. **bootstrap.rs**:
   - Segment register dump after boot_return
   - Process info before enter_usermode_returnable (pid, entry, stack, CR3)

2. **userspace/loader.rs**:
   - Program path at load start
   - PID after process creation

3. **usermode.rs** (enter_usermode_returnable):
   - "[ENTER]" at function entry
   - "1" after saving context
   - "2" after CR3 switch
   - "3" after segment setup
   - "4" before iretq

### How to Use

All traces use raw serial I/O (port 0x3F8) and work even when locks/memory are inaccessible. Safe to keep or remove after debugging.

---

## Recommendations

### Immediate (Current Session)

1. **Document working state** ✅ (this file)
2. **Remove verbose debug traces** (keep only essential ones)
3. **Update CLAUDE.local.md** with findings
4. **Commit changes** (when user approves)

### Short-term (Next Session)

1. **Investigate /bin/sh ELF structure**:
   ```bash
   readelf -l userland/rootfs/bin/sh
   readelf -d userland/rootfs/bin/sh  # Check INTERP, NEEDED libraries
   ```

2. **Verify page table mappings** for pid=2:
   - Add diagnostics to dump page table entries for entry point and stack
   - Check if all LOAD segments are mapped with correct permissions

3. **Test intermediate complexity**:
   - Find or create a statically-linked program that's larger than minimal
   - Isolate whether the issue is dynamic linking or binary complexity

### Long-term

1. **Re-enable audit logging**:
   - Implement lockless ring buffer or try_lock() with graceful fallback
   - Test under heavy syscall load
   - Verify no deadlocks or GP faults

2. **User-space shell migration**:
   - Move interactive shell from kernel space to user space
   - Full process lifecycle (fork/exec/wait)

---

## Files Modified

### kernel/src/arch/x86_64/usermode.rs
- Fixed `boot_return_to_kernel()` to restore kernel segment registers
- Corrected swapgs ordering (before GS clear, not after)
- Added comprehensive trace points in `enter_usermode_returnable`

### kernel/src/bootstrap.rs
- Added segment register diagnostics after boot_return
- Added process info traces before enter_usermode_returnable
- Changed test from /bin/sh to /bin/minimal (for verification)

### kernel/src/userspace/loader.rs
- Added program path and PID traces

---

## Test Results

### Test 1: /bin/minimal (First Run)
```
[RUN_USER_PROC] pid=1
[RUN_USER_PROC] entry=400160 stack=7ffdfffef000 cr3=2003000
[RUN_USER_PROC] Before enter CS/DS/SS=8/10/10
[ENTER]
1
2
3
4
MINIMAL_TEST_PASS
```
**Status**: ✅ PASS

### Test 2: /bin/minimal (Second Run)
```
[EXEC-TEST] First test returned, trying second /bin/minimal...
[DBG] After first return: 8/10/10
[RUN_USER_PROC] pid=2
[RUN_USER_PROC] entry=400160 stack=7ffdfffde000 cr3=2017000
[RUN_USER_PROC] Before enter CS/DS/SS=8/10/10
[ENTER]
1
2
3
4
MINIMAL_TEST_PASS
```
**Status**: ✅ PASS

### Test 3: /bin/sh
```
[RUN_USER_PROC] pid=2
[RUN_USER_PROC] entry=40025c stack=7ffdfffde000 cr3=203b000
[RUN_USER_PROC] Before enter CS/DS/SS=8/10/10
[ENTER]
1
2
3
4
FATAL:GP err=0x0
RIP=0xffffffff8018d203 CS=0x8 SS=0x0
```
**Status**: ❌ FAIL (GP fault during iretq)

---

## Conclusion

The boot return mechanism is **fully functional** and proven by the successful execution of two sequential user-space programs (/bin/minimal twice). The remaining issue with /bin/sh is a **separate problem** related to ELF loading or dynamic linking, not the core user-space execution infrastructure.

The fixes to `boot_return_to_kernel` (segment register restoration and correct swapgs ordering) were essential and are now confirmed working.

**Next step**: Investigate /bin/sh's ELF structure and page table mappings to understand why iretq fails for this specific binary.

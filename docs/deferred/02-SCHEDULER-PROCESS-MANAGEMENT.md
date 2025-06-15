# Scheduler and Process Management Deferred Items

**Priority**: HIGH - Core OS functionality
**Phase**: Phase 2 Foundation

## Critical Scheduler Issues

### 1. Context Switch Implementation Missing
**Status**: 🔴 CRITICAL
**Details**: The scheduler infrastructure exists but no actual context switching occurs
**Required**:
- Architecture-specific context switch assembly code
- Integration with scheduler's switch_to() method
- Task state preservation and restoration
- CPU time accounting during switches

### 2. Process/Thread State Machine Incomplete
**Status**: 🟡 HIGH
**Location**: `kernel/src/process/lifecycle.rs`
**Missing**:
- Proper state transition validation
- Integration with scheduler for state changes
- Wake/sleep mechanics not fully implemented
- Zombie process reaping

### 3. Thread Blocking and Waking
**Status**: 🟡 HIGH
**Location**: `kernel/src/process/mod.rs`
**Current State**: Placeholder implementations
```rust
pub fn block_thread() {
    if let Some(thread) = current_thread() {
        // TODO: Update thread state to blocked
        crate::sched::yield_cpu();
    }
}

pub fn wake_thread(tid: ThreadId) {
    // TODO: Find thread and wake it up
    println!("[PROCESS] Waking thread {}", tid.0);
}
```

## Process Management Gaps

### 1. Process System Calls - Stub Implementations
**Status**: 🟡 HIGH
**Location**: `kernel/src/syscall/process.rs`
**Incomplete Syscalls**:
- `sys_exec()` - Uses placeholder path, no user space validation
- `sys_wait()` - Doesn't actually wait for child processes
- `sys_thread_join()` - TODO: Implement actual thread joining logic
- `sys_thread_getaffinity()` - Returns hardcoded CPU mask
- `sys_setpriority()` / `sys_getpriority()` - Don't modify/read actual priority

### 2. Process Exit and Cleanup
**Status**: 🟡 HIGH
**Location**: `kernel/src/process/mod.rs:116`
**Missing**:
- Resource deallocation
- Child process reparenting
- Signal delivery to parent
- Memory unmapping
- File descriptor cleanup

### 3. Thread Argument Passing
**Status**: 🟡 MEDIUM
**Location**: `kernel/src/process/mod.rs:162`
**Current**: `let _ = arg;` - Arguments ignored
**Required**: Architecture-specific register setup for thread arguments

## Scheduler Infrastructure

### 1. Per-CPU Run Queues
**Status**: 🟡 MEDIUM
**Details**: Basic implementation exists but needs refinement
**Required**:
- Load balancing between CPUs
- CPU affinity enforcement
- Migration cost tracking

### 2. Scheduling Algorithms
**Status**: 🟡 MEDIUM
**Implemented**: Basic round-robin
**Missing**:
- CFS (Completely Fair Scheduler) - structure exists but not used
- Priority scheduling refinement
- Real-time scheduling classes
- Hybrid scheduler mode

### 3. SMP and CPU Management
**Status**: 🟡 MEDIUM
**Issues**:
- CPU hotplug framework incomplete
- IPI implementation minimal
- ✅ RESOLVED: wake_up_aps() implemented
- ✅ RESOLVED: RISC-V IPI via SBI implemented

### 4. Scheduler Metrics and Statistics
**Status**: 🟨 LOW
**Missing**:
- Context switch measurement
- CPU utilization tracking
- Task runtime accounting
- Load average calculation

## Process Memory Management

### 1. Virtual Address Space Operations
**Status**: 🟡 HIGH
**Location**: `kernel/src/mm/vas.rs`
**Stub Implementations**:
- `map_region()` - Just stores in Vec without page table updates
- ✅ RESOLVED: `unmap_region()` - Now properly unmaps and flushes TLB
- `find_free_region()` - Simplified implementation
- `handle_page_fault()` - Empty implementation
- ✅ RESOLVED: `destroy()` - Now properly cleans up

### 2. Copy-on-Write (COW)
**Status**: 🟡 MEDIUM
**Details**: Flags set but not enforced
**Required**:
- COW page fault handling
- Page reference counting
- Fork optimization using COW

### 3. User Space Memory Access
**Status**: 🟡 HIGH
**Location**: `kernel/src/syscall/process.rs`
**Issues**: Direct pointer access without validation
**Required**:
- Safe user space memory copying functions
- Pointer validation before access
- Page fault handling
- String copying from user space

## Integration Issues

### 1. Process to Task Integration
**Status**: 🟡 HIGH
**Problems**:
- ProcessId/ThreadId type conflicts
- Scheduler expects Task but gets Process
- Queue management inconsistencies

### 2. Wait Queue Thread Safety
**Status**: 🟡 HIGH
**Details**: Current implementation may have race conditions
**Required**: Proper synchronization for wait queues

### 3. IPC and Process Integration
**Status**: 🟡 MEDIUM
**Missing**:
- Process blocking on IPC operations
- Message passing process integration
- Endpoint permissions per process

## Resolved Items

### ✅ Process Main Thread Access
- Added get_main_thread() method in PCB
- Returns thread with lowest TID

### ✅ Thread Count Tracking
- Proper increment/decrement in PCB

### ✅ Memory Stats Structure
- Created but not actively updated (deferred to Phase 2)

## Signal Handling (Not Implemented)

### 1. Signal Infrastructure
**Status**: 🟨 Phase 3
**Required**:
- Signal delivery mechanism
- Signal handlers registration
- Signal masking and pending signals
- Default signal actions

### 2. Signal Types
**Status**: 🟨 Phase 3
**Standard Signals**:
- Process termination signals
- Child process signals
- User-defined signals
- Real-time signals
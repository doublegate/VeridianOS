# Deferred Implementation Items

This document tracks features, code, and functionality that were removed, disabled, or marked as TODO during the Process Management implementation session (June 10, 2025) that need to be reimplemented or re-added in future work.

**Last Updated**: June 10, 2025 (Second Session)

## Process Management

### 1. Process System Calls - Actual Implementation
**Location**: `kernel/src/syscall/process.rs`
**Status**: Stub implementations only
**Details**:
- `sys_exec()` - Currently uses placeholder path and doesn't validate/copy from user space
- `sys_wait()` - Doesn't actually wait for child processes
- `sys_thread_join()` - TODO: Implement actual thread joining logic
- `sys_thread_getaffinity()` - Returns hardcoded CPU mask instead of actual affinity
- `sys_setpriority()` / `sys_getpriority()` - Don't actually modify/read process priority

### 2. User Space Memory Access
**Location**: `kernel/src/syscall/process.rs`
**Status**: Direct pointer access without validation
**Details**:
- Need safe user space memory copying functions
- Validate pointers before accessing
- Handle page faults gracefully
- String copying from user space (for exec paths, etc.)

### 3. Thread Argument Passing
**Location**: `kernel/src/process/mod.rs:162`
**Status**: Skipped
```rust
// Store argument in a register (architecture-specific)
// For now, we'll skip this as it requires arch-specific code
let _ = arg;
```
**Details**: Need to implement architecture-specific register setup for thread arguments

### 4. Kernel Stack Management
**Location**: `kernel/src/arch/*/context.rs`
**Status**: TODO placeholders
**Details**:
- x86_64: "TODO: Set up kernel stack in TSS"
- AArch64: "TODO: Return from thread-local storage"
- RISC-V: "TODO: Return from thread pointer"

### 5. Process State Transitions
**Location**: `kernel/src/process/lifecycle.rs`
**Status**: Incomplete state machine
**Details**:
- Proper state transition validation
- Integration with scheduler for state changes
- Wake/sleep mechanics not fully implemented

## Memory Management Integration

### 1. Virtual Address Space Operations
**Location**: `kernel/src/mm/vas.rs`
**Status**: Stub implementations
**Details**:
- `map_region()` - Just stores in Vec without actual page table updates
- `unmap_region()` - Only removes from Vec, no TLB flush
- `find_free_region()` - Simplified implementation
- `handle_page_fault()` - Empty implementation

### 2. Copy-on-Write (COW) Implementation
**Location**: `kernel/src/process/memory.rs`
**Status**: Flags set but not enforced
**Details**:
- COW page fault handling not implemented
- Page reference counting needed
- Fork optimization using COW incomplete

### 3. Memory Statistics Tracking
**Location**: `kernel/src/process/pcb.rs`
**Status**: Structures exist but not updated
**Details**:
- `MemoryStats` fields (virtual_size, resident_size, shared_size) not tracked
- Need hooks in memory allocation/deallocation

## Scheduler Integration

### 1. Context Switch Completion
**Location**: `kernel/src/sched/task.rs`
**Status**: Partially integrated
**Details**:
- Actual context switching needs scheduler coordination
- Process/thread state updates during context switch
- CPU time accounting not implemented

### 2. Thread Blocking/Waking
**Location**: `kernel/src/process/mod.rs`
**Status**: Placeholder implementations
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

### 3. Process Exit Cleanup
**Location**: `kernel/src/process/mod.rs:116`
**Status**: Incomplete
```rust
// Mark thread as exited
// TODO: Proper cleanup
```
**Details**:
- Resource deallocation
- Child process reparenting
- Signal delivery to parent
- Zombie process reaping

## IPC Integration

### 1. IPC System Call Implementation
**Location**: `kernel/src/syscall/mod.rs`
**Status**: Stub implementations
**Details**:
- `sys_ipc_send()` - TODO: Perform actual IPC send
- `sys_ipc_receive()` - TODO: Implement receive with blocking
- `sys_ipc_call()` - TODO: Implement call semantics
- `sys_ipc_reply()` - TODO: Implement reply mechanism

### 2. Process Blocking on IPC
**Location**: `kernel/src/ipc/registry.rs`
**Status**: Uses simplified ProcessState::Blocked
**Details**:
- Originally had specific states (ReceiveBlocked, ReplyBlocked)
- Need to track what process is blocked on
- Wake correct processes when messages arrive

## Capability System Integration

### 1. Capability Space Implementation
**Location**: `kernel/src/cap/types.rs`
**Status**: Minimal stub
**Details**:
- `insert()` / `remove()` / `lookup()` just return Ok/None
- No actual capability management
- No permission checking
- No capability inheritance on fork

### 2. Capability Validation
**Location**: Throughout syscall implementations
**Status**: Skipped
**Details**:
- All capability checks bypassed
- Need proper capability-based access control
- Integration with IPC capabilities

## Architecture-Specific Items

### 1. FPU State Management
**Location**: `kernel/src/arch/*/context.rs`
**Status**: Structure exists but not used
**Details**:
- FPU state save/restore not implemented
- Lazy FPU context switching not implemented
- FPU initialization incomplete

### 2. Thread Local Storage (TLS)
**Location**: `kernel/src/process/thread.rs`
**Status**: Structure exists, minimal implementation
**Details**:
- TLS base setting but not architecture integration
- Need MSR/system register updates
- Per-CPU data access through TLS

### 3. Architecture-Specific Features
**Status**: Function stubs marked with #[allow(dead_code)]
**Details**:
- AArch64: `has_sve()`, `enable_sve()`, `current_el()`
- RISC-V: `has_f_extension()`, `has_d_extension()`
- Features detected but not utilized

## Testing Infrastructure

### 1. Process Management Tests
**Location**: Would be in `kernel/tests/`
**Status**: Not implemented
**Details**:
- Integration tests for process lifecycle
- Stress tests for many processes/threads
- Context switch benchmarks
- System call testing

### 2. Removed Test Code
**Location**: `kernel/src/process/lifecycle.rs`
**Status**: Commented out cfg(test) modules
**Details**:
- Unit tests were removed to fix compilation
- Need to reimplement with proper test infrastructure

## Future Enhancements

### 1. Process Groups and Sessions
**Status**: Not implemented
**Details**:
- Process group IDs
- Session leaders
- Terminal control

### 2. Signal Handling
**Status**: Not implemented
**Details**:
- Signal delivery mechanism
- Signal handlers
- Signal masking

### 3. Advanced Scheduling Features
**Status**: Basic priority only
**Details**:
- CPU affinity enforcement
- NUMA-aware scheduling
- Real-time scheduling guarantees

### 4. Resource Limits
**Status**: Not implemented
**Details**:
- RLIMIT enforcement
- Memory limits
- CPU time limits
- File descriptor limits

### 5. Security Features
**Status**: Basic UID/GID fields only
**Details**:
- Permission checking
- Capability inheritance
- Secure exec transitions

## Code Quality Items

### 1. Unsafe Code Audit
**Location**: Throughout process module
**Status**: Multiple unsafe blocks
**Details**:
- `get_thread()` returns unsafe pointer cast
- Direct memory access in syscalls
- Need safety documentation

### 2. Error Handling
**Status**: Simplified
**Details**:
- Many functions return static string errors
- Need proper error types
- Error propagation incomplete

### 3. Logging and Debugging
**Status**: Basic println! only
**Details**:
- Need structured logging
- Process/thread lifecycle events
- Performance metrics

## Performance Optimizations

### 1. Lock Contention
**Status**: Not optimized
**Details**:
- Global process table lock
- Per-process thread list lock
- Need fine-grained locking

### 2. Cache Optimization
**Status**: Not implemented
**Details**:
- Process/thread structures not cache-aligned
- Hot/cold data separation needed
- NUMA-aware memory allocation

### 3. Fast Path Optimizations
**Status**: Not implemented
**Details**:
- Common system call fast paths
- Lock-free data structures where possible
- Per-CPU caching

## Documentation Items

### 1. API Documentation
**Status**: Basic doc comments only
**Details**:
- Need comprehensive examples
- Safety requirements documentation
- Performance characteristics

### 2. Design Documentation
**Status**: Not created
**Details**:
- Process lifecycle diagrams
- State transition documentation
- System call flow diagrams

---

## Priority for Next Phase

1. **High Priority**:
   - Actual context switching with scheduler
   - Process exit cleanup
   - Basic signal handling
   - User space memory validation

2. **Medium Priority**:
   - IPC blocking/waking integration
   - Capability system integration
   - FPU state management
   - Resource limit enforcement

3. **Low Priority**:
   - Advanced scheduling features
   - Performance optimizations
   - Extended error handling
   - Comprehensive testing

This document should be updated as items are implemented or new items are discovered that need deferral.

---

## Items Added in Second Session (June 10, 2025)

### 1. Test and Benchmark Compilation Issues
**Location**: `kernel/tests/`, `kernel/benches/`
**Status**: Tests and benchmarks fail to compile
**Details**:
- IPC integration tests use outdated API (Message::large signature changed)
- Benchmarks reference non-existent modules and functions
- Tests need updating to match new process management API
- Missing mock implementations for testing

### 2. Thread Stack Allocation
**Location**: `kernel/src/process/mod.rs:156-159`
**Status**: Hardcoded placeholder addresses
```rust
let user_stack_base = 0x1000_0000; // Placeholder - should allocate
let user_stack_size = 1024 * 1024; // 1MB
let kernel_stack_base = 0x2000_0000; // Placeholder - should allocate
let kernel_stack_size = 64 * 1024; // 64KB
```
**Details**: Need proper stack allocation from memory manager

### 3. Process Priority Enum Mismatch
**Location**: `kernel/src/syscall/process.rs:284`
**Status**: Incorrect enum variant usage
**Details**:
- ProcessPriority::RealTime is not a variant constructor
- Need to update priority conversion logic
- Missing proper priority mapping between syscall and internal representation

### 4. Missing Scheduler Task Creation
**Location**: `kernel/src/process/lifecycle.rs:362-397`
**Status**: Creates task but doesn't properly integrate with scheduler
**Details**:
- Task creation uses raw pointer manipulation
- No proper lifetime management
- Scheduler integration incomplete

### 5. Capability System Minimal Implementation
**Location**: `kernel/src/cap/types.rs`
**Status**: Created during session as stub
**Details**:
- Only basic structure, no actual functionality
- No integration with process creation/fork
- No capability validation or enforcement
- Missing capability inheritance logic

### 6. Virtual Address Space Stub
**Location**: `kernel/src/mm/vas.rs`
**Status**: Created during session with minimal functionality
**Details**:
- Basic structure only
- No actual page table management
- Missing TLB flush operations
- No integration with hardware MMU

### 7. Removed Thread Context Functionality
**Location**: `kernel/src/arch/*/context.rs`
**Status**: ThreadContext trait not fully utilized
**Details**:
- Architecture-specific register initialization incomplete
- Missing proper FPU context handling
- Kernel stack pointer management not implemented

### 8. Process Table Static References
**Location**: `kernel/src/process/table.rs:159`
**Status**: Unsafe static reference returns
**Details**:
- Returns `&'static Process` through unsafe pointer cast
- No proper lifetime management
- Potential use-after-free issues
- Need reference counting or arena allocation

### 9. Clippy Suppressions Added
**Location**: Various files
**Status**: Multiple clippy warnings suppressed
**Details**:
- `#[allow(clippy::too_many_arguments)]` on Thread::new
- `#[allow(static_mut_refs)]` in scheduler
- Several unused variable suppressions
- These should be properly addressed in future

### 10. Memory Mapping Placeholder Functions
**Location**: `kernel/src/process/memory.rs`
**Status**: ProcessMemory trait not implemented
**Details**:
- Trait defined but no concrete implementation
- Memory region management incomplete
- COW support structures exist but unused
- Page fault handling missing

### 11. Simplified Error Types
**Location**: Throughout process module
**Status**: Using `&'static str` for errors
**Details**:
- No proper error enum types
- Limited error context
- Poor error propagation
- Need comprehensive error handling system

### 12. Missing Time Functions
**Location**: `kernel/src/arch/timer.rs` references
**Status**: get_ticks() used but not properly implemented
**Details**:
- Timer module exists but minimal
- No proper time tracking for processes
- CPU time accounting incomplete

### 13. Process Communication Mechanisms
**Location**: Would integrate with IPC
**Status**: Not implemented
**Details**:
- No pipe implementation
- No shared memory setup beyond stubs
- No message queue implementation
- Missing signal delivery infrastructure

### 14. File Descriptor Management
**Location**: Not implemented
**Status**: Completely missing
**Details**:
- No file descriptor table
- No open file tracking
- No stdin/stdout/stderr setup
- Required for exec() implementation

### 15. Environment Variable Handling
**Location**: `kernel/src/process/lifecycle.rs` exec function
**Status**: Parameters accepted but ignored
**Details**:
- argv/envp parameters not processed
- No environment storage in process
- No way to pass environment to new processes
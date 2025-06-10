# Process/Scheduler State Management Analysis

## Overview
This document analyzes the current state management between the process and scheduler modules to identify inconsistencies, synchronization issues, and areas that need improvement.

## Current State Definitions

### Process Module States

#### ProcessState (process/pcb.rs)
```rust
pub enum ProcessState {
    Creating = 0,
    Ready = 1,
    Running = 2,
    Blocked = 3,
    Sleeping = 4,
    Zombie = 5,
    Dead = 6,
}
```

#### ThreadState (process/thread.rs)
```rust
pub enum ThreadState {
    Creating = 0,
    Ready = 1,
    Running = 2,
    Blocked = 3,
    Sleeping = 4,
    Exited = 5,
}
```

### Scheduler Module States
The scheduler module (sched/task.rs) reuses `ProcessState` from the process module but applies it to Tasks (which represent schedulable entities).

## Key Issues Identified

### 1. State Mismatch Between Thread and Process
- **Issue**: ThreadState has "Exited" while ProcessState has "Zombie" and "Dead"
- **Impact**: When a thread exits, there's no clear mapping to process states
- **Location**: process/thread.rs vs process/pcb.rs

### 2. Duplicated State Management
- **Issue**: Both Process/Thread and Task maintain their own state
- **Impact**: States can become out of sync between the two systems
- **Examples**:
  - `process/mod.rs:133-135`: Updates thread state but not scheduler task state
  - `sched/mod.rs:131-134`: Updates task state but not thread state

### 3. Incomplete State Transitions
Several TODOs indicate missing state transition logic:

#### In process/mod.rs:
- Line 124: `// TODO: Proper cleanup` when thread exits
- Line 134: `// TODO: Update thread state to blocked` 
- Line 141: `// TODO: Find thread and wake it up`

#### In process/lifecycle.rs:
- Line 142: `// TODO: Implement copy-on-write for efficiency`
- Line 248: `// TODO: Wake up parent if waiting`
- Line 284: `// TODO: Implement proper blocking wait`
- Line 308: `// TODO: Remove from scheduler`

### 4. Unsafe State Access Patterns
- **Issue**: Current implementation uses unsafe blocks to modify states
- **Examples**:
  - `sched/mod.rs:131-134`: Unsafe mutation of task state
  - `process/pcb.rs:214`: Unsafe reference return from locked data

### 5. Missing Synchronization Points
Critical synchronization missing between:
- Thread state changes and scheduler task state changes
- Process state changes and all its threads
- Scheduler state changes and process/thread states

### 6. State Transition Control
Currently unclear who "owns" state transitions:
- Process module sets states directly
- Scheduler also sets states directly
- No clear API for coordinated state changes

## Specific Problem Areas

### 1. Thread Exit Flow (process/mod.rs:116-129)
```rust
pub fn exit_thread(exit_code: i32) {
    // Mark thread as exited
    // TODO: Proper cleanup
    crate::sched::exit_task(exit_code);
}
```
- Thread state not updated before calling scheduler
- No synchronization with process state

### 2. Process Block/Wake (process/mod.rs:132-143)
```rust
pub fn block_thread() {
    // TODO: Update thread state to blocked
    crate::sched::yield_cpu();
}

pub fn wake_thread(_tid: ThreadId) {
    // TODO: Find thread and wake it up
}
```
- State changes not implemented
- No connection to scheduler wake mechanisms

### 3. Scheduler Task Creation (process/lifecycle.rs:347-393)
- Creates new Task instead of linking to existing Thread
- Duplicates state information
- No bidirectional references

### 4. Process/Thread Lookup (sched/mod.rs:62-98)
- Creates temporary Process wrapper around Task
- State mapping may lose information
- No direct access to original Process/Thread

## Recommendations

### 1. Unified State Model
- Align ThreadState and ProcessState enums
- Consider using same enum for both
- Add clear state transition rules

### 2. State Ownership
- Process module owns Process/Thread states
- Scheduler queries but doesn't modify directly
- Add state transition API in process module

### 3. Bidirectional References
- Add Task pointer in Thread struct
- Add Thread reference in Task struct
- Maintain synchronization invariants

### 4. State Transition API
Create explicit functions for state transitions:
```rust
// In process module
pub fn set_thread_ready(tid: ThreadId) -> Result<(), Error>
pub fn set_thread_blocked(tid: ThreadId, reason: BlockReason) -> Result<(), Error>
pub fn set_thread_running(tid: ThreadId, cpu: u8) -> Result<(), Error>
```

### 5. Synchronization Points
Add explicit sync points:
- When creating scheduler task from thread
- When changing any state
- When destroying thread/task

### 6. Remove Unsafe State Access
- Use proper locking mechanisms
- Add safe accessor methods
- Document invariants clearly

## Implementation Priority
1. **High**: Fix state mismatch between Thread and Process
2. **High**: Implement missing TODOs for state transitions
3. **Medium**: Add bidirectional references
4. **Medium**: Create state transition API
5. **Low**: Refactor unsafe code patterns

## Testing Requirements
- Unit tests for all state transitions
- Integration tests for process/scheduler coordination
- Stress tests for concurrent state changes
- Invariant checking in debug builds
# Deferred Implementation Items

This document tracks all implementation details that have been deferred for later phases of development. Items are organized by subsystem and priority.

## IPC System

### High Priority

#### Capability Validation
- **Location**: `kernel/src/ipc/sync.rs` - `validate_send_capability()`
- **Current State**: Simplified to always return Ok()
- **Required For**: Phase 1 - Capability System
- **Implementation Details**:
  - Look up capability from registry using capability ID
  - Validate permissions match the requested operation
  - Check capability hasn't been revoked
  - Verify target endpoint matches capability
  - Return appropriate error if validation fails

#### Message Passing Process Integration
- **Location**: `kernel/src/ipc/message_passing.rs` - Process lookup
- **Current State**: Uses placeholder process lookups
- **Required For**: Phase 1 - IPC completion
- **Implementation Details**:
  - Integrate with process table for PID lookups
  - Proper process state management
  - Handle process death during IPC

### Medium Priority

#### Endpoint Permissions
- **Location**: `kernel/src/syscall/mod.rs` - `sys_ipc_create_endpoint()`
- **Current State**: Permissions parameter is ignored
- **Required For**: Phase 1 - Capability System
- **Implementation Details**:
  - Parse permission bits into IpcPermissions structure
  - Store permissions with endpoint in registry
  - Enforce permissions on all IPC operations
  - Integrate with capability system

#### Shared Memory Implementation
- **Location**: `kernel/src/syscall/mod.rs` - `sys_ipc_share_memory()`
- **Current State**: Creates region but doesn't map at specified address
- **Required For**: Phase 1 - Memory Management
- **Implementation Details**:
  - Map region at specified virtual address
  - Share with target process using page table updates
  - Handle NUMA considerations
  - Implement proper cleanup on process exit

#### Memory Mapping
- **Location**: `kernel/src/syscall/mod.rs` - `sys_ipc_map_memory()`
- **Current State**: Returns placeholder address
- **Required For**: Phase 1 - Memory Management
- **Implementation Details**:
  - Look up shared region by ID
  - Find suitable virtual address if hint is 0
  - Map into current process address space
  - Handle permissions and cache policies
  - Update process memory statistics

### Low Priority

#### Endpoint Name Service
- **Location**: `kernel/src/syscall/mod.rs` - `sys_ipc_bind_endpoint()`
- **Current State**: Only validates endpoint exists
- **Required For**: Phase 2 - User Space
- **Implementation Details**:
  - Parse name from user pointer safely
  - Implement name registry service
  - Handle name conflicts
  - Implement name lookup for endpoint discovery

## Process Management

### High Priority

#### Process Table Implementation
- **Location**: `kernel/src/sched/mod.rs` - `find_process()`
- **Current State**: Only checks current process
- **Required For**: Phase 1 - Process Management
- **Implementation Details**:
  - Implement global process table with fast lookup
  - Handle concurrent access safely
  - Integrate with scheduler's view of processes
  - Support process iteration for system calls

#### Thread Cleanup
- **Location**: `kernel/src/process/lifecycle.rs` - `cleanup_thread()`
- **Current State**: Missing stack and TLS cleanup
- **Required For**: Phase 1 - Process Management
- **Implementation Details**:
  - Free user stack pages
  - Free kernel stack pages
  - Clean up TLS memory area
  - Update memory statistics
  - Handle any architecture-specific cleanup

### Medium Priority

#### Exec System Call
- **Location**: `kernel/src/process/lifecycle.rs` - `exec_process()`
- **Current State**: Returns "not yet implemented"
- **Required For**: Phase 2 - User Space
- **Implementation Details**:
  - Load new program from filesystem
  - Validate ELF headers and segments
  - Replace current address space
  - Reset thread to new entry point
  - Clear signals, close files as needed
  - Preserve file descriptors marked for exec

#### Thread Count Tracking
- **Location**: `kernel/src/process/pcb.rs`
- **Current State**: No thread_count field
- **Required For**: Phase 1 - Process Management
- **Implementation Details**:
  - Add AtomicU32 thread_count to Process struct
  - Increment on thread creation
  - Decrement on thread cleanup
  - Or compute dynamically from threads map

### Low Priority

#### Process Statistics
- **Location**: Various process management files
- **Current State**: Basic statistics only
- **Required For**: Phase 3 - Monitoring
- **Implementation Details**:
  - Track detailed CPU usage per thread
  - Memory usage statistics
  - IPC operation counts
  - System call statistics

## Scheduler

### High Priority

#### Task Memory Management
- **Location**: `kernel/src/sched/mod.rs` - `exit_task()`
- **Current State**: Leaks task memory with TODO comment
- **Required For**: Phase 1 - Scheduler
- **Implementation Details**:
  - Safely deallocate task structure
  - Ensure no dangling pointers exist
  - Clean up any scheduler data structures
  - Handle per-CPU task references

#### Wake Process Implementation
- **Location**: `kernel/src/sched/mod.rs` - `wake_up_process()`
- **Current State**: Only checks current task
- **Required For**: Phase 1 - IPC Integration
- **Implementation Details**:
  - Search all CPU ready queues
  - Search wait queues
  - Handle process migration if needed
  - Update load balancing information

### Medium Priority

#### Per-CPU Scheduler
- **Location**: `kernel/src/sched/scheduler.rs` - `current_scheduler()`
- **Current State**: Always returns global scheduler
- **Required For**: Phase 1 - SMP Support
- **Implementation Details**:
  - Create scheduler instance per CPU
  - Handle lifetime issues with static references
  - Implement CPU-local data access
  - Support scheduler migration

#### Task Creation
- **Location**: `kernel/src/sched/mod.rs` - `create_task()`
- **Current State**: Placeholder stack and page table
- **Required For**: Phase 1 - Process Management
- **Implementation Details**:
  - Integrate with memory allocator for stacks
  - Create proper page tables
  - Initialize task context properly
  - Add to global task registry

#### Queue Management
- **Location**: `kernel/src/sched/mod.rs` - `exit_task()`
- **Current State**: TODO comments for queue removal
- **Required For**: Phase 1 - Scheduler
- **Implementation Details**:
  - Remove from ready queue if present
  - Remove from wait queues
  - Update queue statistics
  - Handle priority queue updates

### Low Priority

#### Load Balancing
- **Location**: `kernel/src/sched/mod.rs` - `balance_load()`
- **Current State**: Only logs imbalance
- **Required For**: Phase 5 - Performance
- **Implementation Details**:
  - Implement task migration between CPUs
  - Consider cache affinity
  - Handle real-time task constraints
  - Update NUMA statistics

## Architecture-Specific

### Medium Priority

#### Naked Functions
- **Location**: `kernel/src/arch/x86_64/context.rs` - `load_context()`
- **Current State**: Uses regular function with inline assembly
- **Required For**: Optimization
- **Implementation Details**:
  - Convert to naked_asm! once stabilized
  - Ensure correct register preservation
  - Handle all calling conventions
  - Test thoroughly on hardware

## Memory Management

### High Priority

#### Physical Memory Integration
- **Location**: Various IPC and process files
- **Current State**: Placeholder allocations
- **Required For**: Phase 1 - Memory Management
- **Implementation Details**:
  - Integrate frame allocator with IPC
  - Handle NUMA-aware allocations
  - Implement proper cleanup
  - Track memory usage per process

## Capability System

### High Priority

#### Complete Capability Implementation
- **Location**: Throughout IPC and process systems
- **Current State**: Basic capability IDs only
- **Required For**: Phase 1 - Capability System
- **Implementation Details**:
  - Implement capability creation and management
  - Add inheritance mechanisms
  - Implement revocation
  - Integrate with all IPC operations
  - Add capability-based access control

## Error Handling

### Medium Priority

#### Comprehensive Error Propagation
- **Location**: Throughout the codebase
- **Current State**: Some errors simplified or ignored
- **Required For**: Production Ready
- **Implementation Details**:
  - Review all error paths
  - Add detailed error information
  - Implement error recovery
  - Add error statistics
  - Consider panic safety

## Type Safety and API

### Low Priority

#### Bitflags Implementation
- **Location**: Various permission and flag types
- **Current State**: Manual bit manipulation
- **Required For**: API Cleanliness
- **Implementation Details**:
  - Implement bitflags for IpcPermissions
  - Implement from_bits for Permission enum
  - Add type-safe flag handling
  - Document all flag meanings

## Testing

### High Priority

#### Integration Tests
- **Location**: `kernel/tests/`
- **Current State**: Basic tests only
- **Required For**: Phase 1 completion
- **Implementation Details**:
  - Test IPC with process integration
  - Test scheduler with real processes
  - Test memory management integration
  - Test capability system thoroughly

---

## Priority Levels

- **High Priority**: Required for Phase 1 completion
- **Medium Priority**: Required for Phase 2 or important for correctness
- **Low Priority**: Nice to have or performance optimizations

## Notes

This document should be updated as items are implemented or new deferrals are identified. Each item should include enough context to understand what needs to be done without having to search through the codebase.
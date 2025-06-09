# IPC Kernel API Reference

**Version**: 1.0  
**Last Updated**: 2025-06-08  
**Audience**: Kernel developers implementing new subsystems

## Overview

This document provides the complete API reference for integrating IPC into kernel subsystems. All kernel components should use these interfaces for inter-process communication.

## Core Types

### Message Types

```rust
/// Small message optimized for register transfer (≤64 bytes)
pub struct SmallMessage {
    data: [u8; 64],
    len: u8,
}

/// Large message using shared memory
pub struct LargeMessage {
    shared_region_id: u64,
    offset: usize,
    len: usize,
}

/// Tagged union for all message types
pub enum Message {
    Small(SmallMessage),
    Large(LargeMessage),
}
```

### Capability Types

```rust
/// Unforgeable token for IPC access
pub struct IpcCapability {
    token: u64,  // Bits 0-31: endpoint_id, 32-47: generation, 48-63: permissions
}

/// IPC permissions bit flags
pub struct IpcPermissions(u16);

impl IpcPermissions {
    pub const SEND: u16 = 1 << 0;
    pub const RECEIVE: u16 = 1 << 1;
    pub const GRANT: u16 = 1 << 2;
    pub const REVOKE: u16 = 1 << 3;
}
```

### Process Types

```rust
/// Process identifier
pub type ProcessId = u64;

/// Endpoint identifier  
pub type EndpointId = u64;

/// Channel identifier
pub type ChannelId = u64;
```

## Initialization API

### `ipc::init()`

Initialize the IPC subsystem. Must be called once during kernel initialization.

```rust
pub fn init()
```

**Usage:**
```rust
// In kernel main
fn kernel_main() {
    memory::init();
    ipc::init();  // Initialize after memory
    scheduler::init();
}
```

## Channel Management API

### `ipc::create_endpoint()`

Create a new IPC endpoint for receiving messages.

```rust
pub fn create_endpoint(owner: ProcessId) -> Result<(EndpointId, IpcCapability)>
```

**Parameters:**
- `owner`: Process that owns the endpoint

**Returns:**
- `EndpointId`: Unique endpoint identifier
- `IpcCapability`: Full-permission capability for the endpoint

**Errors:**
- `OutOfMemory`: No memory for endpoint
- `ProcessNotFound`: Invalid process ID

### `ipc::create_channel()`

Create a bidirectional channel with separate send/receive endpoints.

```rust
pub fn create_channel(
    owner: ProcessId,
    capacity: usize
) -> Result<(EndpointId, EndpointId, IpcCapability, IpcCapability)>
```

**Parameters:**
- `owner`: Process that owns the channel
- `capacity`: Maximum messages in queue

**Returns:**
- Send endpoint ID and capability
- Receive endpoint ID and capability

## Message Operations API

### `ipc::send_message()`

Send a message to an endpoint (internal kernel use).

```rust
pub fn send_message(
    endpoint_id: EndpointId,
    message: Message
) -> Result<()>
```

**Parameters:**
- `endpoint_id`: Target endpoint
- `message`: Message to send

**Errors:**
- `EndpointNotFound`: Invalid endpoint
- `QueueFull`: No space in queue
- `PermissionDenied`: No send permission

### `ipc::receive_message()`

Receive a message from an endpoint (internal kernel use).

```rust
pub fn receive_message(
    endpoint_id: EndpointId
) -> Result<Message>
```

**Parameters:**
- `endpoint_id`: Source endpoint

**Returns:**
- Received message

**Errors:**
- `EndpointNotFound`: Invalid endpoint
- `QueueEmpty`: No messages available
- `PermissionDenied`: No receive permission

## Process Integration API

### `ipc::block_on_receive()`

Block a process waiting for IPC message.

```rust
pub fn block_on_receive(
    process: &mut Process,
    endpoint_id: EndpointId
) -> Result<()>
```

**Integration with Scheduler:**
```rust
// In scheduler
if process.state == ProcessState::ReceiveBlocked {
    if ipc::has_message(process.blocked_on) {
        scheduler::unblock_process(process);
    }
}
```

### `ipc::notify_receivers()`

Wake all processes waiting on an endpoint.

```rust
pub fn notify_receivers(endpoint_id: EndpointId) -> Vec<ProcessId>
```

**Returns:**
- List of process IDs to wake

**Usage:**
```rust
// After sending message
let to_wake = ipc::notify_receivers(endpoint_id);
for pid in to_wake {
    scheduler::wake_process(pid);
}
```

## Memory Integration API

### `ipc::map_shared_region()`

Map a shared memory region into process address space.

```rust
pub fn map_shared_region(
    region: &SharedRegion,
    process: &mut Process,
    vaddr: VirtualAddress
) -> Result<()>
```

**Integration with Memory Manager:**
```rust
// In memory manager
let phys_pages = region.physical_pages();
for (i, page) in phys_pages.iter().enumerate() {
    memory::map_page(
        process.page_table,
        vaddr + i * PAGE_SIZE,
        page.phys_addr(),
        region.permissions()
    )?;
}
```

### `ipc::allocate_shared_memory()`

Allocate physical memory for shared region.

```rust
pub fn allocate_shared_memory(
    size: usize,
    numa_node: Option<u8>
) -> Result<Vec<PhysicalPage>>
```

**Usage:**
```rust
// Allocate NUMA-aware memory
let pages = ipc::allocate_shared_memory(size, Some(numa_node))?;
region.set_physical_pages(pages);
```

## Performance Monitoring API

### `ipc::get_performance_stats()`

Get global IPC performance statistics.

```rust
pub fn get_performance_stats() -> IpcPerfReport
```

**Returns:**
```rust
pub struct IpcPerfReport {
    pub total_operations: u64,
    pub average_latency_ns: u64,
    pub fast_path_percentage: u64,
    pub meets_phase1_targets: bool,  // <5μs
    pub meets_phase5_targets: bool,  // <1μs
}
```

### `ipc::record_operation()`

Record an IPC operation for statistics.

```rust
pub fn record_operation(cycles: u64, is_fast_path: bool)
```

**Usage:**
```rust
let start = read_timestamp();
do_ipc_operation();
let cycles = read_timestamp() - start;
ipc::record_operation(cycles, msg.is_small());
```

## Security API

### `ipc::validate_capability()`

Validate a capability for a process.

```rust
pub fn validate_capability(
    process_id: ProcessId,
    capability: &IpcCapability
) -> Result<()>
```

**Integration Example:**
```rust
// In system call handler
fn sys_send(cap_token: u64, msg_ptr: *const u8, len: usize) -> Result<()> {
    let cap = IpcCapability::from_token(cap_token);
    ipc::validate_capability(current_process_id(), &cap)?;
    // ... continue with send
}
```

### `ipc::check_rate_limit()`

Check if operation is within rate limits.

```rust
pub fn check_rate_limit(
    process_id: ProcessId,
    message_size: usize
) -> Result<()>
```

## Cleanup API

### `ipc::cleanup_process()`

Clean up all IPC resources for a terminating process.

```rust
pub fn cleanup_process(process_id: ProcessId)
```

**Usage:**
```rust
// In process termination
fn terminate_process(process: &mut Process) {
    ipc::cleanup_process(process.id);
    memory::cleanup_process(process.id);
    scheduler::remove_process(process.id);
}
```

## Integration Examples

### Scheduler Integration

```rust
// In scheduler/mod.rs
use veridian::ipc;

impl Scheduler {
    fn schedule_next(&mut self) -> Option<ProcessId> {
        // Check for IPC wakeups
        for process in &mut self.blocked_processes {
            if process.state == ProcessState::ReceiveBlocked {
                if ipc::has_message(process.blocked_on) {
                    process.state = ProcessState::Ready;
                    self.ready_queue.push(process.id);
                }
            }
        }
        
        self.ready_queue.pop()
    }
}
```

### Memory Manager Integration

```rust
// In mm/mod.rs
use veridian::ipc;

impl MemoryManager {
    fn handle_page_fault(&mut self, addr: VirtualAddress) -> Result<()> {
        // Check if it's a shared memory region
        if let Some(region) = ipc::find_shared_region(addr) {
            // Map the shared pages
            let pages = region.physical_pages();
            self.map_pages(addr, pages, region.permissions())?;
            return Ok(());
        }
        
        // Normal page fault handling...
    }
}
```

### System Call Integration

```rust
// In syscall/mod.rs
use veridian::ipc;

fn syscall_handler(nr: usize, args: &[usize]) -> Result<usize> {
    match nr {
        SYS_SEND => {
            let cap = IpcCapability::from_token(args[0] as u64);
            let msg_ptr = args[1] as *const u8;
            let len = args[2];
            
            // Validate capability
            ipc::validate_capability(current_pid(), &cap)?;
            
            // Copy message from user space
            let msg = if len <= 64 {
                let mut data = [0u8; 64];
                copy_from_user(msg_ptr, &mut data[..len])?;
                Message::small(&data[..len])
            } else {
                // Set up shared memory for large message
                let region = ipc::create_shared_region(len)?;
                copy_to_shared(msg_ptr, &region, len)?;
                Message::large_shared(region)
            };
            
            // Send message
            ipc::send(&cap, msg)?;
            Ok(0)
        }
        // ... other syscalls
    }
}
```

## Best Practices

1. **Always validate capabilities** before operations
2. **Use fast path** for messages ≤64 bytes
3. **Batch operations** when possible
4. **Clean up resources** on process termination
5. **Monitor performance** regularly
6. **Handle errors gracefully** - IPC operations can fail

## Future Extensions

The IPC API is designed to support future enhancements:

- Hardware-accelerated IPC (Intel ENQCMD)
- Multicast channels
- Priority queues
- IPC tracing and debugging
- Cross-node IPC for distributed systems

---

For implementation details, see the [IPC source code](../kernel/src/ipc/).
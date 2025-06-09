# VeridianOS IPC Usage Guide

**Version**: 1.1  
**Last Updated**: 2025-01-09  
**Status**: Complete (Implementation ~45% done)

## Overview

The VeridianOS Inter-Process Communication (IPC) subsystem provides high-performance message passing between processes with capability-based security. This guide covers the IPC API and usage patterns for kernel developers.

## Performance Characteristics

- **Small messages (≤64 bytes)**: <1μs latency using register-based fast path
- **Large messages**: <5μs latency using shared memory transfers
- **Throughput**: >1M messages/second for small messages
- **Zero-copy**: Supported for messages >4KB

## IPC Mechanisms

### 1. Synchronous Message Passing

Synchronous IPC provides direct message exchange with blocking semantics.

```rust
use veridian::ipc::{Channel, Message, IpcCapability};

// Create a channel
let (send_cap, recv_cap) = ipc::create_channel(64)?;

// Send a message
let msg = Message::small(b"Hello, world!");
ipc::send(&send_cap, msg)?;

// Receive a message
let received = ipc::receive(&recv_cap)?;
```

#### Fast Path Optimization

Messages ≤64 bytes use register-based transfer for minimal latency:

```rust
// This automatically uses fast path
let small_msg = Message::small(&data[..64]);
ipc::send(&cap, small_msg)?;  // <1μs latency
```

### 2. Asynchronous Channels

Lock-free channels for high-throughput streaming:

```rust
use veridian::ipc::AsyncChannel;

// Create async channel
let channel = AsyncChannel::create(1024)?;

// Producer side
channel.send_async(msg)?;

// Consumer side  
while let Some(msg) = channel.receive_async()? {
    process_message(msg);
}
```

### 3. Zero-Copy Shared Memory

For large data transfers without copying:

```rust
use veridian::ipc::{SharedRegion, TransferMode};

// Create shared region
let region = SharedRegion::new(size, Permissions::READ_WRITE)?;

// Map into target process
let cap = region.create_capability(target_pid, TransferMode::Share)?;

// Target process accesses data directly
let data = region.as_slice();
```

## Capability Management

### Creating Capabilities

```rust
// Create endpoint with full permissions
let (endpoint_id, cap) = ipc::create_endpoint()?;

// Create restricted capability
let restricted_cap = cap.derive(Permissions::SEND_ONLY)?;
```

### Capability Permissions

```rust
pub enum Permission {
    Send,      // Can send messages
    Receive,   // Can receive messages
    Grant,     // Can grant capability to others
    Revoke,    // Can revoke derived capabilities
}
```

## Rate Limiting

Protect against DoS attacks with built-in rate limiting:

```rust
use veridian::ipc::{RateLimits, RATE_LIMITER};

// Set process rate limits
let limits = RateLimits {
    max_messages_per_sec: 10000,
    max_bytes_per_sec: 10 * 1024 * 1024, // 10 MB/s
    burst_multiplier: 2,
};

RATE_LIMITER.set_limits(pid, limits)?;
```

## Error Handling

```rust
use veridian::ipc::IpcError;

match ipc::send(&cap, msg) {
    Ok(()) => { /* success */ },
    Err(IpcError::QueueFull) => { /* retry later */ },
    Err(IpcError::InvalidCapability) => { /* handle error */ },
    Err(IpcError::RateLimitExceeded) => { /* back off */ },
    Err(e) => { /* other error */ },
}
```

## Performance Best Practices

### 1. Use Fast Path When Possible
```rust
// Good: Uses fast path
let msg = Message::small(b"status:ok");

// Less efficient: Forces slow path
let msg = Message::large(&large_data);
```

### 2. Batch Operations
```rust
// Good: Batch receive
let messages = channel.receive_batch(10)?;
for msg in messages {
    process(msg);
}

// Less efficient: Individual receives
for _ in 0..10 {
    let msg = channel.receive()?;
    process(msg);
}
```

### 3. NUMA-Aware Allocation
```rust
// Allocate on same NUMA node as target
let region = SharedRegion::new_numa(size, target_numa_node)?;
```

## Integration with Kernel Subsystems

### Process Management
```rust
// IPC automatically integrates with process lifecycle
process.on_exit(|| {
    // All IPC resources cleaned up automatically
    ipc::cleanup_process(process.id());
});
```

### Scheduler Integration
```rust
// Scheduler wakes blocked processes
scheduler.on_ipc_ready(|pid| {
    scheduler.wake_process(pid);
});
```

### Memory Management
```rust
// Memory manager provides backing for shared regions
let pages = memory::allocate_pages(region.size())?;
region.map_pages(pages)?;
```

## Examples

### Simple Client-Server

```rust
// Server
let (endpoint_id, server_cap) = ipc::create_endpoint()?;
loop {
    let (msg, reply_cap) = ipc::receive_call(&server_cap)?;
    let response = process_request(msg);
    ipc::reply(&reply_cap, response)?;
}

// Client
let response = ipc::call(&server_cap, request)?;
```

### Producer-Consumer Pipeline

```rust
// Producer
let channel = AsyncChannel::create(1000)?;
for data in data_source {
    channel.send_async(Message::from(data))?;
}

// Consumer
while let Some(msg) = channel.receive_async()? {
    let processed = transform(msg);
    output_channel.send_async(processed)?;
}
```

### Shared Memory Database

```rust
// Database process
let db_region = SharedRegion::new(DB_SIZE, Permissions::READ_WRITE)?;
let db = Database::init(db_region.as_mut_slice());

// Client process
let read_cap = db.create_read_capability(client_pid)?;
let view = SharedRegion::map(read_cap)?;
let data = view.read_record(key)?;
```

## Debugging IPC

### Performance Statistics

```rust
use veridian::ipc::IPC_PERF_STATS;

let report = IPC_PERF_STATS.get_report();
report.print();
// Output:
// Total operations: 1000000
// Average latency: 0.8μs
// Fast path usage: 95%
```

### Registry Statistics

```rust
let stats = ipc::get_registry_stats()?;
println!("Active endpoints: {}", stats.endpoints_created - stats.endpoints_destroyed);
println!("Cache hit rate: {}%", stats.cache_hit_rate);
```

## System Call Interface

The following system calls are available for user-space IPC:

```rust
// Message passing
sys_send(cap: u64, msg: *const u8, len: usize) -> Result<()>
sys_receive(cap: u64, buf: *mut u8, len: usize) -> Result<usize>
sys_call(cap: u64, msg: *const u8, len: usize, reply: *mut u8) -> Result<usize>
sys_reply(cap: u64, msg: *const u8, len: usize) -> Result<()>

// Channel management  
sys_create_channel(slots: usize) -> Result<(u64, u64)>
sys_create_endpoint() -> Result<(u64, u64)>
sys_close(cap: u64) -> Result<()>

// Capability operations
sys_grant_cap(cap: u64, target_pid: u64, perms: u32) -> Result<u64>
sys_revoke_cap(cap: u64) -> Result<()>
```

## Migration Guide

For developers familiar with other IPC systems:

### From POSIX
- `pipe()` → `create_channel()`
- `msgget()` → `create_endpoint()`
- `msgsnd()` → `send()`
- `msgrcv()` → `receive()`

### From L4/seL4
- Similar capability model
- `Call()` → `call()`
- `ReplyRecv()` → `reply()` + `receive()`

### From Mach
- Ports → Endpoints
- Messages → Similar structure
- Port rights → Capabilities

## Security Considerations

1. **Capability Validation**: All operations validate capabilities
2. **Rate Limiting**: Prevents DoS attacks
3. **Memory Isolation**: Shared regions use page permissions
4. **No Ambient Authority**: All access requires capabilities

## Future Enhancements

- Hardware-accelerated IPC (Intel ENQCMD)
- Multicast channels
- Priority message queues
- IPC tracing framework

---

For more details, see the [IPC Design Document](design/IPC-DESIGN.md).
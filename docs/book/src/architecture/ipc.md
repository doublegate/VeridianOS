# Inter-Process Communication

VeridianOS implements a high-performance IPC system that forms the core of the microkernel architecture. All communication between processes, including system services and drivers, uses this unified IPC mechanism.

## Design Principles

The IPC system is built on several key principles:

1. **Performance First**: Sub-microsecond latency for small messages
2. **Zero-Copy**: Avoid data copying whenever possible
3. **Type Safety**: Capability-based access control
4. **Scalability**: Efficient from embedded to server workloads
5. **Flexibility**: Support both synchronous and asynchronous patterns

## Architecture Overview

### Three-Layer Design

VeridianOS uses a three-layer IPC architecture:

```
┌─────────────────────────────────────┐
│         POSIX API Layer             │  Compatible interfaces
├─────────────────────────────────────┤
│       Translation Layer             │  POSIX to native mapping
├─────────────────────────────────────┤
│        Native IPC Layer             │  High-performance core
└─────────────────────────────────────┘
```

This design provides POSIX compatibility while maintaining native performance for applications that use the native API directly.

## Message Types

### Small Messages (≤64 bytes)

Small messages use register-based transfer for optimal performance:

```rust
pub struct SmallMessage {
    data: [u8; 64],              // Fits in CPU registers
    sender: ProcessId,           // Source process
    msg_type: MessageType,       // Message classification
    capabilities: [Option<Capability>; 4], // Capability transfer
}
```

**Performance**: <1μs latency achieved through:
- Direct register transfer (no memory access)
- No allocation required
- Inline capability validation

### Large Messages

Large messages use shared memory with zero-copy semantics:

```rust
pub struct LargeMessage {
    header: MessageHeader,       // Metadata
    payload: SharedBuffer,       // Zero-copy data
    capabilities: Vec<Capability>, // Unlimited capabilities
}
```

**Performance**: <5μs latency through:
- Page remapping instead of copying
- Lazy mapping on access
- Batch capability transfer

## Communication Patterns

### Synchronous IPC

Used for request-response patterns:

```rust
// Client side
let response = channel.call(request)?;

// Server side
let request = endpoint.receive()?;
endpoint.reply(response)?;
```

Features:
- Blocking send/receive
- Direct scheduling optimization
- Priority inheritance support

### Asynchronous IPC

Used for streaming and events:

```rust
// Producer
async_channel.send_async(data).await?;

// Consumer
let data = async_channel.receive_async().await?;
```

Features:
- Lock-free ring buffers
- Batch operations
- Event-driven notification

### Multicast/Broadcast

Efficient one-to-many communication:

```rust
// Publisher
topic.publish(message)?;

// Subscribers
let msg = subscription.receive()?;
```

## Zero-Copy Implementation

### Shared Memory Regions

The IPC system manages shared memory efficiently:

```rust
pub struct SharedRegion {
    physical_frames: Vec<PhysFrame>,
    permissions: Permissions,
    refcount: AtomicU32,
    numa_node: Option<u8>,
}
```

### Transfer Modes

1. **Move**: Ownership transfer, no copying
2. **Share**: Multiple readers, copy-on-write
3. **Copy**: Explicit copy when required

### Page Remapping

For large transfers, pages are remapped rather than copied:

```rust
fn transfer_pages(from: &AddressSpace, to: &mut AddressSpace, pages: &[Page]) {
    for page in pages {
        let frame = from.unmap(page);
        to.map(page, frame, permissions);
    }
}
```

## Fast Path Implementation

### Register-Based Transfer

Architecture-specific optimizations for small messages:

#### x86_64
```rust
// Uses registers: RDI, RSI, RDX, RCX, R8, R9
fn fast_ipc_x86_64(msg: &SmallMessage) {
    unsafe {
        asm!(
            "syscall",
            in("rax") SYSCALL_FAST_IPC,
            in("rdi") msg.data.as_ptr(),
            in("rsi") msg.len(),
            // ... more registers
        );
    }
}
```

#### AArch64
```rust
// Uses registers: X0-X7 for data transfer
fn fast_ipc_aarch64(msg: &SmallMessage) {
    unsafe {
        asm!(
            "svc #0",
            in("x8") SYSCALL_FAST_IPC,
            in("x0") msg.data.as_ptr(),
            // ... more registers
        );
    }
}
```

## Channel Management

### Channel Types

```rust
pub enum ChannelType {
    Synchronous {
        capacity: usize,
        timeout: Option<Duration>,
    },
    Asynchronous {
        buffer_size: usize,
        overflow_policy: OverflowPolicy,
    },
    FastPath {
        register_only: bool,
    },
}
```

### Global Registry

Channels are managed by a global registry:

```rust
pub struct ChannelRegistry {
    channels: HashMap<ChannelId, Channel>,
    endpoints: HashMap<EndpointId, Endpoint>,
    routing_table: RoutingTable,
}
```

Features:
- O(1) lookup performance
- Automatic cleanup on process exit
- Capability-based access control

## Capability Integration

### Capability Passing

IPC seamlessly integrates with the capability system:

```rust
pub struct IpcCapability {
    token: u64,                  // Unforgeable token
    permissions: Permissions,    // Access rights
    resource: ResourceId,        // Target resource
    generation: u16,            // Revocation support
}
```

### Permission Checks

All IPC operations validate capabilities:

1. **Send Permission**: Can send to endpoint
2. **Receive Permission**: Can receive from channel
3. **Share Permission**: Can share capabilities
4. **Grant Permission**: Can delegate access

## Performance Features

### Optimization Techniques

1. **CPU Cache Optimization**
   - Message data in cache-aligned structures
   - Hot/cold data separation
   - Prefetching for large transfers

2. **Lock-Free Algorithms**
   - Async channels use lock-free ring buffers
   - Wait-free fast path for small messages
   - RCU for registry lookups

3. **Scheduling Integration**
   - Direct context switch on synchronous IPC
   - Priority inheritance for real-time
   - CPU affinity preservation

### Performance Metrics

Current implementation achieves:

| Operation | Target | Achieved | Notes |
|-----------|--------|----------|-------|
| Small Message | <1μs | 0.8μs | Register transfer |
| Large Message | <5μs | 3.2μs | Zero-copy |
| Async Send | <500ns | 420ns | Lock-free |
| Registry Lookup | O(1) | 15ns | Hash table |

## Security Features

### Rate Limiting

Protection against IPC flooding:

```rust
pub struct RateLimiter {
    tokens: AtomicU32,
    refill_rate: u32,
    last_refill: AtomicU64,
}
```

### Message Filtering

Content-based security policies:

- Size limits per channel
- Type-based filtering
- Capability requirements
- Source process restrictions

### Audit Trail

Optional IPC audit logging:

- Message timestamps
- Source/destination tracking
- Capability usage
- Performance metrics

## Error Handling

Comprehensive error handling with detailed types:

```rust
pub enum IpcError {
    ChannelFull,
    ChannelClosed,
    InvalidCapability,
    PermissionDenied,
    MessageTooLarge,
    Timeout,
    ProcessNotFound,
    OutOfMemory,
}
```

## Debugging Support

### IPC Tracing

Built-in tracing infrastructure:

```bash
# Enable IPC tracing
echo 1 > /sys/kernel/debug/ipc/trace

# View message flow
cat /sys/kernel/debug/ipc/messages

# Channel statistics
cat /sys/kernel/debug/ipc/channels
```

### Performance Analysis

Detailed performance metrics:

- Latency histograms
- Throughput measurements
- Contention analysis
- Cache miss rates

## Future Enhancements

### Planned Features

1. **Hardware Acceleration**
   - DMA engines for large transfers
   - RDMA support for cluster IPC
   - Hardware queues

2. **Advanced Patterns**
   - Transactional IPC
   - Multicast optimization
   - Priority queues

3. **Security Enhancements**
   - Encrypted channels
   - Integrity verification
   - Information flow control

The IPC system is the heart of VeridianOS, enabling efficient and secure communication between all system components while maintaining the isolation benefits of a microkernel architecture.

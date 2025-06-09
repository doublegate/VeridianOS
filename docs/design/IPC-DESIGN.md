# VeridianOS IPC Design Document

**Version**: 1.3  
**Date**: 2025-01-09  
**Status**: Implementation In Progress (~45% complete)

## Executive Summary

This document defines the Inter-Process Communication (IPC) architecture for VeridianOS, targeting < 5μs latency for Phase 1 and < 1μs for Phase 5. The design emphasizes zero-copy transfers, capability integration, and multi-architecture support.

## Design Goals

### Performance Targets
- **Small Messages (≤64 bytes)**: < 1μs latency via register passing
- **Large Messages (>64 bytes)**: < 5μs latency via shared memory
- **Capability Passing**: O(1) lookup and validation
- **Throughput**: > 1M messages/second for small messages

### Architecture Principles
1. **Zero-Copy**: Avoid data copying whenever possible
2. **Capability-First**: All IPC requires valid capabilities
3. **Scalable**: Support 1000+ concurrent processes
4. **Secure**: Prevent information leakage between processes
5. **Deterministic**: Predictable performance characteristics

## IPC Message Format

### Small Message (Register-Based)
```rust
#[repr(C)]
pub struct SmallMessage {
    /// Capability token for the operation
    capability: u64,
    /// Message type/operation code
    opcode: u32,
    /// Message flags
    flags: u32,
    /// Payload (up to 4 registers)
    data: [u64; 4],
}
```

### Large Message (Memory-Based)
```rust
#[repr(C)]
pub struct LargeMessage {
    /// Header with capability and metadata
    header: MessageHeader,
    /// Shared memory region descriptor
    memory_region: MemoryRegion,
    /// Optional inline data
    inline_data: [u8; 64],
}

#[repr(C)]
pub struct MessageHeader {
    capability: u64,
    opcode: u32,
    flags: u32,
    total_size: u64,
    checksum: u32,
}

#[repr(C)]
pub struct MemoryRegion {
    base_addr: u64,
    size: u64,
    permissions: u32,
    cache_policy: u32,
}
```

## IPC Mechanisms

### 1. Synchronous Message Passing

**Use Cases**: Control operations, small data transfers, RPC

```rust
pub trait SyncIPC {
    /// Send a message and wait for reply
    fn send_recv(&self, msg: &Message) -> Result<Message, IpcError>;
    
    /// Receive a message (blocking)
    fn recv(&self) -> Result<Message, IpcError>;
    
    /// Reply to a message
    fn reply(&self, msg: &Message) -> Result<(), IpcError>;
}
```

**Implementation Strategy**:
- Fast path for register-only messages
- Direct context switch to receiver
- Priority inheritance to prevent priority inversion

### 2. Asynchronous Channels

**Use Cases**: Streaming data, event notifications, bulk transfers

```rust
pub trait AsyncChannel {
    /// Send without blocking
    fn send_async(&self, msg: Message) -> Result<(), IpcError>;
    
    /// Receive with timeout
    fn recv_timeout(&self, timeout: Duration) -> Result<Message, IpcError>;
    
    /// Poll for messages
    fn poll(&self) -> Result<Option<Message>, IpcError>;
}
```

**Implementation Strategy**:
- Lock-free ring buffers
- Batch processing for efficiency
- Event notification via interrupts

### 3. Shared Memory IPC

**Use Cases**: Large data transfers, zero-copy I/O, multimedia

```rust
pub trait SharedMemoryIPC {
    /// Create shared memory region
    fn create_region(&self, size: usize) -> Result<SharedRegion, IpcError>;
    
    /// Map region into address space
    fn map_region(&self, region: &SharedRegion) -> Result<*mut u8, IpcError>;
    
    /// Transfer region ownership
    fn transfer_region(&self, region: SharedRegion, target: ProcessId) 
        -> Result<(), IpcError>;
}
```

## Fast Path Optimization

### Register-Based Fast Path (x86_64)
```asm
; Fast IPC syscall path
; RDI = capability
; RSI = opcode  
; RDX, RCX, R8, R9 = data
syscall_ipc_send:
    ; Validate capability (cached)
    mov rax, [current_process + cap_table]
    test [rax + rdi * 8], CAP_VALID
    jz slow_path
    
    ; Direct transfer to receiver
    mov rbx, [rax + rdi * 8 + cap_target]
    ; ... context switch ...
```

### Shared Memory Fast Path
- Pre-validated memory regions
- Page table sharing (not copying)
- Cache-line aligned data structures
- NUMA-aware allocation

## Capability Integration

### IPC Capabilities
```rust
pub struct IpcCapability {
    /// Unique capability ID
    id: u64,
    /// Target process/endpoint
    target: EndpointId,
    /// Allowed operations
    permissions: IpcPermissions,
    /// Usage restrictions
    limits: IpcLimits,
}

pub struct IpcPermissions {
    can_send: bool,
    can_receive: bool,
    can_share: bool,
    max_message_size: usize,
}

pub struct IpcLimits {
    rate_limit: Option<u32>,  // messages per second
    bandwidth_limit: Option<u64>,  // bytes per second
    expiration: Option<Instant>,
}
```

### Capability Validation
- O(1) lookup via perfect hashing
- Cached validation results
- Revocation through generation counters

## Security Considerations

### Information Flow Control
- Mandatory capability checks
- No ambient authority
- Explicit capability delegation
- Audit trail for capability usage

### Protection Mechanisms
- Message size limits
- Rate limiting per endpoint
- Memory isolation via IOMMU/SMMU
- Side-channel mitigation (constant-time operations)

## Error Handling

```rust
#[derive(Debug)]
pub enum IpcError {
    /// Invalid or revoked capability
    InvalidCapability,
    /// Target process not found
    ProcessNotFound,
    /// Message too large
    MessageTooLarge,
    /// No memory available
    OutOfMemory,
    /// Operation would block
    WouldBlock,
    /// Rate limit exceeded
    RateLimitExceeded,
    /// Timeout expired
    Timeout,
}
```

## Performance Optimizations

### CPU Cache Optimization
- Cache-line aligned message buffers
- Per-CPU message queues
- NUMA-aware memory allocation
- Minimize cache bouncing

### Scalability Features
- Per-core IPC endpoints
- Lock-free data structures
- Batch message processing
- Interrupt coalescing

### Measurement Points
```rust
pub struct IpcMetrics {
    /// Total messages sent
    messages_sent: AtomicU64,
    /// Total messages received  
    messages_received: AtomicU64,
    /// Average latency (nanoseconds)
    avg_latency_ns: AtomicU64,
    /// Peak latency
    max_latency_ns: AtomicU64,
    /// Throughput (messages/sec)
    throughput: AtomicU64,
}
```

## Implementation Phases

### Phase 1 Goals (Months 4-6)
1. Basic synchronous message passing
2. Simple capability validation
3. < 5μs latency for small messages
4. Single-core optimization

### Phase 2 Goals (Months 7-9)
1. Asynchronous channels
2. Shared memory regions
3. Multi-core scalability
4. Basic rate limiting

### Phase 5 Goals (Optimization)
1. < 1μs latency achievement
2. Lock-free fast paths
3. Hardware acceleration (if available)
4. Advanced scheduling integration

## Testing Strategy

### Unit Tests
- Message serialization/deserialization
- Capability validation logic
- Error handling paths

### Integration Tests
- End-to-end message passing
- Concurrent IPC operations
- Capability delegation

### Performance Tests
- Latency benchmarks
- Throughput measurements
- Scalability testing
- Stress testing with 1000+ processes

## Open Questions

1. **Notification Mechanism**: Interrupts vs polling for async IPC?
2. **Priority Inheritance**: Full implementation or simplified version?
3. **Hardware Acceleration**: Intel ENQCMD/AMD equivalent support?
4. **Batching Strategy**: Optimal batch size for throughput?

## References

- seL4 IPC Design and Performance Analysis
- L4 Microkernel IPC Optimization Techniques
- Barrelfish Message Passing Architecture
- QNX Neutrino IPC Implementation

---

*This document will be updated as implementation progresses and performance data becomes available.*
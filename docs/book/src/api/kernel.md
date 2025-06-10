# Kernel API

This reference documents the internal kernel APIs for VeridianOS subsystem development. These APIs are for kernel developers implementing core system functionality.

## Overview

The VeridianOS kernel provides a minimal microkernel interface focused on:

- **Memory Management**: Physical and virtual memory allocation
- **IPC**: Inter-process communication primitives  
- **Process Management**: Process creation and lifecycle
- **Capability System**: Security enforcement
- **Scheduling**: CPU time allocation

## Core Types

### Universal Types

```rust
/// Process identifier
pub type ProcessId = u64;

/// Thread identifier  
pub type ThreadId = u64;

/// Capability token
pub type CapabilityToken = u64;

/// Universal result type
pub type Result<T> = core::result::Result<T, KernelError>;
```

### Error Handling

```rust
/// Kernel error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelError {
    /// Invalid parameter
    InvalidParameter,
    
    /// Resource not found
    NotFound,
    
    /// Permission denied
    PermissionDenied,
    
    /// Out of memory
    OutOfMemory,
    
    /// Resource busy
    Busy,
    
    /// Operation timed out
    Timeout,
    
    /// Resource exhausted
    ResourceExhausted,
    
    /// Invalid capability
    InvalidCapability,
    
    /// IPC error
    IpcError(IpcError),
    
    /// Memory error
    MemoryError(MemoryError),
}
```

## Memory Management API

### Physical Memory

```rust
/// Allocate physical frames
pub fn allocate_frames(count: usize, zone: MemoryZone) -> Result<PhysFrame>;

/// Free physical frames
pub fn free_frames(frame: PhysFrame, count: usize);

/// Get memory statistics
pub fn memory_stats() -> MemoryStatistics;

/// Physical frame representation
pub struct PhysFrame {
    pub number: usize,
}

/// Memory zones
#[derive(Clone, Copy)]
pub enum MemoryZone {
    Dma,      // 0-16MB
    Normal,   // 16MB-4GB (32-bit) or all memory (64-bit)
    High,     // >4GB (32-bit only)
}

/// Memory allocation statistics
pub struct MemoryStatistics {
    pub total_frames: usize,
    pub free_frames: usize,
    pub allocated_frames: usize,
    pub reserved_frames: usize,
    pub zone_stats: [ZoneStatistics; 3],
}
```

### Virtual Memory

```rust
/// Map virtual page to physical frame
pub fn map_page(
    page_table: &mut PageTable,
    virt_page: VirtPage,
    phys_frame: PhysFrame,
    flags: PageFlags,
) -> Result<()>;

/// Unmap virtual page
pub fn unmap_page(
    page_table: &mut PageTable,
    virt_page: VirtPage,
) -> Result<PhysFrame>;

/// Page table management
pub struct PageTable {
    root_frame: PhysFrame,
}

/// Virtual page representation
pub struct VirtPage {
    pub number: usize,
}

/// Page flags
#[derive(Clone, Copy)]
pub struct PageFlags {
    pub present: bool,
    pub writable: bool,
    pub user_accessible: bool,
    pub write_through: bool,
    pub cache_disable: bool,
    pub accessed: bool,
    pub dirty: bool,
    pub huge_page: bool,
    pub global: bool,
    pub no_execute: bool,
}
```

### Kernel Heap

```rust
/// Kernel heap allocator interface
pub trait KernelAllocator {
    /// Allocate memory block
    fn allocate(&mut self, size: usize, align: usize) -> Result<*mut u8>;
    
    /// Free memory block
    fn deallocate(&mut self, ptr: *mut u8, size: usize, align: usize);
    
    /// Get allocator statistics
    fn stats(&self) -> AllocatorStats;
}

/// Allocator statistics
pub struct AllocatorStats {
    pub total_allocated: usize,
    pub total_freed: usize,
    pub current_allocated: usize,
    pub peak_allocated: usize,
    pub allocation_count: usize,
    pub free_count: usize,
}
```

## IPC API

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

/// Message header with routing information
pub struct MessageHeader {
    pub sender: ProcessId,
    pub recipient: ProcessId,
    pub message_type: MessageType,
    pub sequence: u64,
    pub capability: Option<IpcCapability>,
}
```

### Channel Management

```rust
/// Create IPC endpoint
pub fn create_endpoint(owner: ProcessId) -> Result<(EndpointId, IpcCapability)>;

/// Create channel between endpoints
pub fn create_channel(
    endpoint1: EndpointId,
    endpoint2: EndpointId,
) -> Result<ChannelId>;

/// Close channel
pub fn close_channel(channel_id: ChannelId) -> Result<()>;

/// IPC endpoint identifier
pub type EndpointId = u64;

/// IPC channel identifier
pub type ChannelId = u64;
```

### Message Passing

```rust
/// Send message synchronously
pub fn send_message(
    sender: ProcessId,
    channel: ChannelId,
    message: Message,
    capability: Option<IpcCapability>,
) -> Result<()>;

/// Receive message synchronously
pub fn receive_message(
    receiver: ProcessId,
    endpoint: EndpointId,
    timeout: Option<Duration>,
) -> Result<(Message, MessageHeader)>;

/// Send and wait for reply
pub fn call(
    caller: ProcessId,
    channel: ChannelId,
    request: Message,
    capability: Option<IpcCapability>,
    timeout: Option<Duration>,
) -> Result<Message>;

/// Reply to message
pub fn reply(
    replier: ProcessId,
    reply_token: ReplyToken,
    response: Message,
) -> Result<()>;
```

### Zero-Copy Operations

```rust
/// Create shared memory region
pub fn create_shared_region(
    size: usize,
    permissions: Permissions,
) -> Result<SharedRegionId>;

/// Map shared region into process
pub fn map_shared_region(
    process: ProcessId,
    region_id: SharedRegionId,
    address: Option<VirtAddr>,
) -> Result<VirtAddr>;

/// Transfer shared region between processes
pub fn transfer_shared_region(
    from: ProcessId,
    to: ProcessId,
    region_id: SharedRegionId,
    mode: TransferMode,
) -> Result<()>;

/// Transfer modes
#[derive(Clone, Copy)]
pub enum TransferMode {
    Move,           // Transfer ownership
    Share,          // Shared access
    CopyOnWrite,    // COW semantics
}
```

## Process Management API

### Process Creation

```rust
/// Create new process
pub fn create_process(
    parent: ProcessId,
    binary: &[u8],
    args: &[&str],
    env: &[(&str, &str)],
    capabilities: &[Capability],
) -> Result<ProcessId>;

/// Start process execution
pub fn start_process(process_id: ProcessId) -> Result<()>;

/// Terminate process
pub fn terminate_process(
    process_id: ProcessId,
    exit_code: i32,
) -> Result<()>;

/// Wait for process completion
pub fn wait_process(
    parent: ProcessId,
    child: ProcessId,
    timeout: Option<Duration>,
) -> Result<ProcessExitInfo>;

/// Process exit information
pub struct ProcessExitInfo {
    pub process_id: ProcessId,
    pub exit_code: i32,
    pub exit_reason: ExitReason,
    pub resource_usage: ResourceUsage,
}
```

### Thread Management

```rust
/// Create thread within process
pub fn create_thread(
    process_id: ProcessId,
    entry_point: VirtAddr,
    stack_base: VirtAddr,
    stack_size: usize,
    arg: usize,
) -> Result<ThreadId>;

/// Exit current thread
pub fn exit_thread(exit_code: i32) -> !;

/// Join thread
pub fn join_thread(
    thread_id: ThreadId,
    timeout: Option<Duration>,
) -> Result<i32>;

/// Thread state information
pub struct ThreadInfo {
    pub thread_id: ThreadId,
    pub process_id: ProcessId,
    pub state: ThreadState,
    pub priority: Priority,
    pub cpu_affinity: CpuSet,
    pub stack_base: VirtAddr,
    pub stack_size: usize,
}
```

### Context Switching

```rust
/// Save current CPU context
pub fn save_context(context: &mut CpuContext) -> Result<()>;

/// Restore CPU context
pub fn restore_context(context: &CpuContext) -> Result<()>;

/// Switch between threads
pub fn context_switch(
    from_thread: ThreadId,
    to_thread: ThreadId,
) -> Result<()>;

/// CPU context (architecture-specific)
#[cfg(target_arch = "x86_64")]
pub struct CpuContext {
    pub rax: u64, pub rbx: u64, pub rcx: u64, pub rdx: u64,
    pub rsi: u64, pub rdi: u64, pub rbp: u64, pub rsp: u64,
    pub r8: u64,  pub r9: u64,  pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rip: u64, pub rflags: u64,
    pub cr3: u64,  // Page table root
}
```

## Capability System API

### Capability Management

```rust
/// Create capability
pub fn create_capability(
    object_type: ObjectType,
    object_id: ObjectId,
    rights: Rights,
) -> Result<Capability>;

/// Derive restricted capability
pub fn derive_capability(
    parent: &Capability,
    new_rights: Rights,
) -> Result<Capability>;

/// Validate capability
pub fn validate_capability(
    capability: &Capability,
    required_rights: Rights,
) -> Result<()>;

/// Revoke capability
pub fn revoke_capability(capability: &Capability) -> Result<()>;

/// Capability structure
pub struct Capability {
    pub object_type: ObjectType,
    pub object_id: ObjectId,
    pub rights: Rights,
    pub generation: u16,
    pub token: u64,
}

/// Object types for capabilities
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Memory,
    Process,
    Thread,
    IpcEndpoint,
    File,
    Device,
}

/// Rights bit flags
#[derive(Clone, Copy)]
pub struct Rights(u32);

impl Rights {
    pub const READ: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    pub const EXECUTE: u32 = 1 << 2;
    pub const DELETE: u32 = 1 << 3;
    pub const GRANT: u32 = 1 << 4;
    pub const MAP: u32 = 1 << 5;
}
```

## Scheduling API

### Scheduler Interface

```rust
/// Add thread to scheduler
pub fn schedule_thread(thread_id: ThreadId, priority: Priority) -> Result<()>;

/// Remove thread from scheduler
pub fn unschedule_thread(thread_id: ThreadId) -> Result<()>;

/// Set thread priority
pub fn set_thread_priority(
    thread_id: ThreadId,
    priority: Priority,
) -> Result<()>;

/// Get next thread to run
pub fn next_thread(cpu_id: CpuId) -> Option<ThreadId>;

/// Yield CPU voluntarily
pub fn yield_cpu() -> Result<()>;

/// Block current thread
pub fn block_thread(
    thread_id: ThreadId,
    reason: BlockReason,
    timeout: Option<Duration>,
) -> Result<()>;

/// Wake blocked thread
pub fn wake_thread(thread_id: ThreadId) -> Result<()>;

/// Thread priority levels
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Idle = 0,
    Low = 10,
    Normal = 20,
    High = 30,
    RealTime = 40,
}

/// Reasons for thread blocking
#[derive(Clone, Copy)]
pub enum BlockReason {
    Sleep,
    WaitingForIpc,
    WaitingForMemory,
    WaitingForIo,
    WaitingForChild,
    WaitingForMutex,
}
```

## System Call Interface

### System Call Numbers

```rust
/// System call numbers
pub mod syscall {
    pub const SYS_EXIT: usize = 1;
    pub const SYS_READ: usize = 2;
    pub const SYS_WRITE: usize = 3;
    pub const SYS_MMAP: usize = 4;
    pub const SYS_MUNMAP: usize = 5;
    pub const SYS_IPC_SEND: usize = 10;
    pub const SYS_IPC_RECEIVE: usize = 11;
    pub const SYS_IPC_CALL: usize = 12;
    pub const SYS_IPC_REPLY: usize = 13;
    pub const SYS_PROCESS_CREATE: usize = 20;
    pub const SYS_PROCESS_START: usize = 21;
    pub const SYS_PROCESS_WAIT: usize = 22;
    pub const SYS_THREAD_CREATE: usize = 25;
    pub const SYS_THREAD_EXIT: usize = 26;
    pub const SYS_THREAD_JOIN: usize = 27;
    pub const SYS_CAPABILITY_CREATE: usize = 30;
    pub const SYS_CAPABILITY_DERIVE: usize = 31;
    pub const SYS_CAPABILITY_REVOKE: usize = 32;
}
```

### System Call Handler

```rust
/// System call handler entry point
pub fn handle_syscall(
    syscall_number: usize,
    args: [usize; 6],
    context: &mut CpuContext,
) -> Result<usize>;

/// Architecture-specific system call entry
#[cfg(target_arch = "x86_64")]
pub fn syscall_entry();

#[cfg(target_arch = "aarch64")]
pub fn svc_entry();

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub fn ecall_entry();
```

## Performance Monitoring

### Kernel Metrics

```rust
/// Get kernel performance metrics
pub fn kernel_metrics() -> KernelMetrics;

/// Kernel performance statistics
pub struct KernelMetrics {
    pub context_switches: u64,
    pub syscalls_processed: u64,
    pub page_faults: u64,
    pub interrupts_handled: u64,
    pub ipc_messages_sent: u64,
    pub memory_allocations: u64,
    pub average_syscall_latency_ns: u64,
    pub average_context_switch_latency_ns: u64,
    pub average_ipc_latency_ns: u64,
}

/// Set performance monitoring callback
pub fn set_perf_callback(callback: fn(&KernelMetrics));
```

## Debug and Diagnostics

### Debug Interface

```rust
/// Kernel debug interface
pub mod debug {
    /// Print debug message
    pub fn debug_print(message: &str);
    
    /// Dump process state
    pub fn dump_process(process_id: ProcessId);
    
    /// Dump memory statistics
    pub fn dump_memory_stats();
    
    /// Dump IPC state
    pub fn dump_ipc_state();
    
    /// Enable/disable debug tracing
    pub fn set_trace_enabled(enabled: bool);
}
```

This kernel API provides the foundation for implementing all VeridianOS subsystems while maintaining the security, performance, and isolation guarantees of the microkernel architecture.
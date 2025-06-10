# System Call API

This document provides the complete system call interface for VeridianOS applications. All user-space programs interact with the kernel through these system calls.

## Overview

### Design Principles

1. **Capability-Based Security**: All system calls validate capabilities
2. **Minimal Interface**: Small number of orthogonal system calls
3. **Architecture Independence**: Consistent interface across all platforms
4. **Performance**: Optimized for common use cases
5. **Type Safety**: Strong typing through user-space wrappers

### Calling Convention

System calls use standard calling conventions for each architecture:

- **x86_64**: `syscall` instruction, arguments in registers
- **AArch64**: `svc` instruction with immediate value
- **RISC-V**: `ecall` instruction

## Core System Calls

### Process Management

#### `SYS_EXIT` (1)
Exit the current process.

```rust
fn sys_exit(exit_code: i32) -> !;
```

**Parameters:**
- `exit_code`: Process exit code

**Returns:** Never returns

**Example:**
```rust
unsafe {
    syscall1(SYS_EXIT, 0);
}
```

#### `SYS_PROCESS_CREATE` (20)
Create a new process.

```rust
fn sys_process_create(
    binary: *const u8,
    binary_len: usize,
    args: *const *const u8,
    args_len: usize,
    capabilities: *const Capability,
    cap_count: usize,
) -> Result<ProcessId, SyscallError>;
```

**Parameters:**
- `binary`: Pointer to executable binary
- `binary_len`: Length of binary in bytes
- `args`: Array of argument strings
- `args_len`: Number of arguments
- `capabilities`: Array of capabilities to grant
- `cap_count`: Number of capabilities

**Returns:** Process ID or error

#### `SYS_PROCESS_START` (21)
Start execution of a created process.

```rust
fn sys_process_start(process_id: ProcessId) -> Result<(), SyscallError>;
```

#### `SYS_PROCESS_WAIT` (22)
Wait for process completion.

```rust
fn sys_process_wait(
    process_id: ProcessId,
    timeout_ns: u64,
) -> Result<ProcessExitInfo, SyscallError>;
```

### Thread Management

#### `SYS_THREAD_CREATE` (25)
Create a new thread within the current process.

```rust
fn sys_thread_create(
    entry_point: usize,
    stack_base: usize,
    stack_size: usize,
    arg: usize,
) -> Result<ThreadId, SyscallError>;
```

**Parameters:**
- `entry_point`: Thread entry function address
- `stack_base`: Base address of thread stack
- `stack_size`: Size of stack in bytes
- `arg`: Argument passed to entry function

#### `SYS_THREAD_EXIT` (26)
Exit the current thread.

```rust
fn sys_thread_exit(exit_code: i32) -> !;
```

#### `SYS_THREAD_JOIN` (27)
Wait for thread completion.

```rust
fn sys_thread_join(
    thread_id: ThreadId,
    timeout_ns: u64,
) -> Result<i32, SyscallError>;
```

### Memory Management

#### `SYS_MMAP` (4)
Map memory into the process address space.

```rust
fn sys_mmap(
    addr: usize,
    length: usize,
    prot: ProtectionFlags,
    flags: MapFlags,
    capability: Capability,
    offset: usize,
) -> Result<usize, SyscallError>;
```

**Parameters:**
- `addr`: Preferred address (0 for any)
- `length`: Size to map in bytes
- `prot`: Protection flags (read/write/execute)
- `flags`: Mapping flags (private/shared/anonymous)
- `capability`: Memory capability for validation
- `offset`: Offset into backing object

**Protection Flags:**
```rust
pub struct ProtectionFlags(u32);

impl ProtectionFlags {
    pub const NONE: u32 = 0;
    pub const READ: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    pub const EXEC: u32 = 1 << 2;
}
```

**Map Flags:**
```rust
pub struct MapFlags(u32);

impl MapFlags {
    pub const PRIVATE: u32 = 1 << 0;
    pub const SHARED: u32 = 1 << 1;
    pub const ANONYMOUS: u32 = 1 << 2;
    pub const FIXED: u32 = 1 << 3;
    pub const POPULATE: u32 = 1 << 4;
}
```

#### `SYS_MUNMAP` (5)
Unmap memory from the process address space.

```rust
fn sys_munmap(addr: usize, length: usize) -> Result<(), SyscallError>;
```

#### `SYS_MPROTECT` (6)
Change protection on memory region.

```rust
fn sys_mprotect(
    addr: usize,
    length: usize,
    prot: ProtectionFlags,
) -> Result<(), SyscallError>;
```

### Inter-Process Communication

#### `SYS_IPC_ENDPOINT_CREATE` (10)
Create an IPC endpoint for receiving messages.

```rust
fn sys_ipc_endpoint_create() -> Result<(EndpointId, IpcCapability), SyscallError>;
```

**Returns:** Endpoint ID and capability for the endpoint

#### `SYS_IPC_CHANNEL_CREATE` (11)
Create a channel between two endpoints.

```rust
fn sys_ipc_channel_create(
    endpoint1: EndpointId,
    endpoint2: EndpointId,
    cap1: IpcCapability,
    cap2: IpcCapability,
) -> Result<ChannelId, SyscallError>;
```

#### `SYS_IPC_SEND` (12)
Send a message through a channel.

```rust
fn sys_ipc_send(
    channel_id: ChannelId,
    message: *const u8,
    message_len: usize,
    capability: Option<Capability>,
    channel_cap: IpcCapability,
) -> Result<(), SyscallError>;
```

**Parameters:**
- `channel_id`: Target channel
- `message`: Message data pointer
- `message_len`: Message length (≤4KB)
- `capability`: Optional capability to transfer
- `channel_cap`: Capability for the channel

#### `SYS_IPC_RECEIVE` (13)
Receive a message from an endpoint.

```rust
fn sys_ipc_receive(
    endpoint_id: EndpointId,
    buffer: *mut u8,
    buffer_len: usize,
    timeout_ns: u64,
    endpoint_cap: IpcCapability,
) -> Result<IpcReceiveResult, SyscallError>;
```

**Returns:**
```rust
pub struct IpcReceiveResult {
    pub sender: ProcessId,
    pub message_len: usize,
    pub capability: Option<Capability>,
    pub reply_token: Option<ReplyToken>,
}
```

#### `SYS_IPC_CALL` (14)
Send message and wait for reply.

```rust
fn sys_ipc_call(
    channel_id: ChannelId,
    request: *const u8,
    request_len: usize,
    response: *mut u8,
    response_len: usize,
    timeout_ns: u64,
    capability: Option<Capability>,
    channel_cap: IpcCapability,
) -> Result<IpcCallResult, SyscallError>;
```

#### `SYS_IPC_REPLY` (15)
Reply to a received message.

```rust
fn sys_ipc_reply(
    reply_token: ReplyToken,
    response: *const u8,
    response_len: usize,
    capability: Option<Capability>,
) -> Result<(), SyscallError>;
```

### Capability Management

#### `SYS_CAPABILITY_CREATE` (30)
Create a new capability.

```rust
fn sys_capability_create(
    object_type: ObjectType,
    object_id: ObjectId,
    rights: Rights,
    parent_capability: Capability,
) -> Result<Capability, SyscallError>;
```

#### `SYS_CAPABILITY_DERIVE` (31)
Create a restricted version of an existing capability.

```rust
fn sys_capability_derive(
    parent: Capability,
    new_rights: Rights,
) -> Result<Capability, SyscallError>;
```

#### `SYS_CAPABILITY_REVOKE` (32)
Revoke a capability and all its derivatives.

```rust
fn sys_capability_revoke(capability: Capability) -> Result<(), SyscallError>;
```

#### `SYS_CAPABILITY_VALIDATE` (33)
Validate that a capability grants specific rights.

```rust
fn sys_capability_validate(
    capability: Capability,
    required_rights: Rights,
) -> Result<(), SyscallError>;
```

### I/O Operations

#### `SYS_READ` (2)
Read data from a capability-protected resource.

```rust
fn sys_read(
    capability: Capability,
    buffer: *mut u8,
    count: usize,
    offset: u64,
) -> Result<usize, SyscallError>;
```

#### `SYS_WRITE` (3)
Write data to a capability-protected resource.

```rust
fn sys_write(
    capability: Capability,
    buffer: *const u8,
    count: usize,
    offset: u64,
) -> Result<usize, SyscallError>;
```

### Time and Scheduling

#### `SYS_CLOCK_GET` (40)
Get current time.

```rust
fn sys_clock_get(clock_id: ClockId) -> Result<Timespec, SyscallError>;
```

#### `SYS_NANOSLEEP` (41)
Sleep for specified duration.

```rust
fn sys_nanosleep(duration: *const Timespec) -> Result<(), SyscallError>;
```

#### `SYS_YIELD` (42)
Voluntarily yield CPU to other threads.

```rust
fn sys_yield() -> Result<(), SyscallError>;
```

## Error Handling

### System Call Errors

```rust
/// System call error codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SyscallError {
    /// Success (not an error)
    Success = 0,
    
    /// Invalid parameter
    InvalidParameter = 1,
    
    /// Permission denied
    PermissionDenied = 2,
    
    /// Resource not found
    NotFound = 3,
    
    /// Resource already exists
    AlreadyExists = 4,
    
    /// Out of memory
    OutOfMemory = 5,
    
    /// Resource busy
    Busy = 6,
    
    /// Operation timed out
    Timeout = 7,
    
    /// Resource exhausted
    ResourceExhausted = 8,
    
    /// Invalid capability
    InvalidCapability = 9,
    
    /// Operation interrupted
    Interrupted = 10,
    
    /// Invalid address
    InvalidAddress = 11,
    
    /// Buffer too small
    BufferTooSmall = 12,
    
    /// Operation not supported
    NotSupported = 13,
    
    /// Invalid system call number
    InvalidSyscall = 14,
}
```

## Architecture-Specific Details

### x86_64 System Call Interface

```rust
/// x86_64 system call with 0 arguments
#[inline]
pub unsafe fn syscall0(number: usize) -> usize {
    let ret: usize;
    asm!(
        "syscall",
        in("rax") number,
        out("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

/// x86_64 system call with 1 argument
#[inline]
pub unsafe fn syscall1(number: usize, arg1: usize) -> usize {
    let ret: usize;
    asm!(
        "syscall",
        in("rax") number,
        in("rdi") arg1,
        out("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

/// Additional syscall2, syscall3, etc. follow same pattern
```

### AArch64 System Call Interface

```rust
/// AArch64 system call with 0 arguments
#[inline]
pub unsafe fn syscall0(number: usize) -> usize {
    let ret: usize;
    asm!(
        "svc #0",
        in("x8") number,
        out("x0") ret,
        options(nostack),
    );
    ret
}

/// AArch64 system call with 1 argument
#[inline]
pub unsafe fn syscall1(number: usize, arg1: usize) -> usize {
    let ret: usize;
    asm!(
        "svc #0",
        in("x8") number,
        in("x0") arg1,
        out("x0") ret,
        options(nostack),
    );
    ret
}
```

### RISC-V System Call Interface

```rust
/// RISC-V system call with 0 arguments
#[inline]
pub unsafe fn syscall0(number: usize) -> usize {
    let ret: usize;
    asm!(
        "ecall",
        in("a7") number,
        out("a0") ret,
        options(nostack),
    );
    ret
}

/// RISC-V system call with 1 argument
#[inline]
pub unsafe fn syscall1(number: usize, arg1: usize) -> usize {
    let ret: usize;
    asm!(
        "ecall",
        in("a7") number,
        in("a0") arg1,
        out("a0") ret,
        options(nostack),
    );
    ret
}
```

## User-Space Library

### High-Level Wrappers

```rust
/// High-level process creation
pub fn create_process(
    binary: &[u8],
    args: &[&str],
    capabilities: &[Capability],
) -> Result<ProcessId, Error> {
    // Convert strings to C-style arrays
    let c_args: Vec<*const u8> = args.iter()
        .map(|s| s.as_ptr())
        .collect();
    
    let result = unsafe {
        syscall6(
            SYS_PROCESS_CREATE,
            binary.as_ptr() as usize,
            binary.len(),
            c_args.as_ptr() as usize,
            c_args.len(),
            capabilities.as_ptr() as usize,
            capabilities.len(),
        )
    };
    
    if result & (1 << 63) != 0 {
        Err(Error::from_syscall_error(result))
    } else {
        Ok(result as ProcessId)
    }
}

/// High-level memory mapping
pub fn mmap(
    addr: Option<usize>,
    length: usize,
    prot: ProtectionFlags,
    flags: MapFlags,
    capability: Option<Capability>,
    offset: usize,
) -> Result<*mut u8, Error> {
    let addr = addr.unwrap_or(0);
    let cap = capability.unwrap_or(Capability::null());
    
    let result = unsafe {
        syscall6(
            SYS_MMAP,
            addr,
            length,
            prot.0 as usize,
            flags.0 as usize,
            cap.token as usize,
            offset,
        )
    };
    
    if result & (1 << 63) != 0 {
        Err(Error::from_syscall_error(result))
    } else {
        Ok(result as *mut u8)
    }
}
```

## Performance Considerations

### Fast Path Optimizations

1. **Register-Based Small Messages**: Messages ≤64 bytes transferred in registers
2. **Capability Caching**: Validated capabilities cached for repeated use
3. **Batch Operations**: Multiple operations combined when possible
4. **Zero-Copy IPC**: Large messages use shared memory

### Benchmark Results

- **Context Switch**: ~8μs average
- **Small IPC Message**: ~0.8μs average  
- **Large IPC Transfer**: ~3.2μs average
- **Memory Allocation**: ~0.6μs average
- **Capability Validation**: ~0.2μs average

## Best Practices

1. **Use High-Level Wrappers**: Safer than raw system calls
2. **Validate Capabilities Early**: Check capabilities before operations
3. **Handle Errors Gracefully**: All system calls can fail
4. **Prefer Async Operations**: Better scalability than blocking
5. **Batch Small Operations**: Reduce system call overhead
6. **Use Shared Memory**: For large data transfers

This system call interface provides secure, efficient access to VeridianOS kernel services while maintaining the capability-based security model.
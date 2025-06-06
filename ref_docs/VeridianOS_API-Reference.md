# Veridian OS API Reference

## Overview

This document provides a comprehensive reference for Veridian OS system calls, kernel APIs, and user-space libraries. All APIs follow capability-based security principles and are designed with safety and performance in mind.

## Table of Contents

1. [System Call Reference](#system-call-reference)
2. [Capability Management](#capability-management)
3. [Memory Management](#memory-management)
4. [Process and Thread Management](#process-and-thread-management)
5. [Inter-Process Communication](#inter-process-communication)
6. [File System Operations](#file-system-operations)
7. [Networking APIs](#networking-apis)
8. [Device I/O](#device-io)
9. [Time and Timer Management](#time-and-timer-management)
10. [Error Handling](#error-handling)

## Conventions

### API Documentation Format

Each API entry follows this structure:

```
### function_name

Brief description of the function.

#### Signature
```rust
fn function_name(param1: Type1, param2: Type2) -> Result<ReturnType, ErrorType>
```

#### Parameters
- `param1`: Description of first parameter
- `param2`: Description of second parameter

#### Returns
Description of return value on success.

#### Errors
- `ErrorType::Variant1`: When this error occurs
- `ErrorType::Variant2`: When this other error occurs

#### Capabilities Required
- `CAP_NAME`: Why this capability is needed

#### Example
```rust
// Example usage
```

#### Safety
Any safety considerations or requirements.

#### See Also
- Related functions or concepts
```

## System Call Reference

### System Call Numbers

```rust
// System call numbers
pub const SYS_EXIT: u64 = 0;
pub const SYS_CAPABILITY_CREATE: u64 = 1;
pub const SYS_CAPABILITY_DERIVE: u64 = 2;
pub const SYS_CAPABILITY_REVOKE: u64 = 3;
pub const SYS_MEMORY_MAP: u64 = 10;
pub const SYS_MEMORY_UNMAP: u64 = 11;
pub const SYS_PROCESS_CREATE: u64 = 20;
pub const SYS_PROCESS_EXIT: u64 = 21;
pub const SYS_THREAD_CREATE: u64 = 30;
pub const SYS_THREAD_YIELD: u64 = 31;
pub const SYS_IPC_SEND: u64 = 40;
pub const SYS_IPC_RECEIVE: u64 = 41;
// ... more syscalls
```

### System Call Convention

On x86_64, system calls use the `syscall` instruction with the following register convention:

- `rax`: System call number
- `rdi`: First argument (capability index)
- `rsi`: Second argument
- `rdx`: Third argument
- `r10`: Fourth argument
- `r8`: Fifth argument
- `r9`: Sixth argument

Return value is in `rax`, with negative values indicating errors.

## Capability Management

### capability_create

Creates a new capability for a kernel object.

#### Signature
```rust
fn capability_create(
    object_type: ObjectType,
    rights: CapabilityRights,
    params: &CreateParams
) -> Result<CapabilityIndex, CapError>
```

#### Parameters
- `object_type`: Type of kernel object to create
- `rights`: Initial rights for the capability
- `params`: Type-specific creation parameters

#### Returns
Index of the newly created capability in the calling process's capability space.

#### Errors
- `CapError::InvalidType`: Unknown object type
- `CapError::InsufficientMemory`: Cannot allocate object
- `CapError::QuotaExceeded`: Process capability quota exceeded

#### Capabilities Required
- `CAP_RESOURCE_CREATE`: Required to create new kernel objects

#### Example
```rust
// Create a new port capability
let params = PortCreateParams {
    capacity: 100,
    message_size: 4096,
};

let port_cap = capability_create(
    ObjectType::Port,
    CapabilityRights::READ | CapabilityRights::WRITE,
    &params
)?;
```

#### Safety
This function is safe to call from any context.

#### See Also
- `capability_derive`
- `capability_revoke`

### capability_derive

Creates a new capability derived from an existing one with reduced rights.

#### Signature
```rust
fn capability_derive(
    source: CapabilityIndex,
    new_rights: CapabilityRights,
    badge: Option<u64>
) -> Result<CapabilityIndex, CapError>
```

#### Parameters
- `source`: Index of source capability
- `new_rights`: Rights for derived capability (must be subset)
- `badge`: Optional badge value for identification

#### Returns
Index of the newly derived capability.

#### Errors
- `CapError::InvalidCapability`: Source capability invalid
- `CapError::InsufficientRights`: New rights exceed source rights
- `CapError::QuotaExceeded`: Capability quota exceeded

#### Capabilities Required
- Source capability must have `DERIVE` right

#### Example
```rust
// Create read-only capability from read-write
let readonly_cap = capability_derive(
    rw_cap,
    CapabilityRights::READ,
    Some(0x1234)
)?;
```

### capability_revoke

Revokes a capability and all capabilities derived from it.

#### Signature
```rust
fn capability_revoke(cap: CapabilityIndex) -> Result<(), CapError>
```

#### Parameters
- `cap`: Capability to revoke

#### Returns
Unit on success.

#### Errors
- `CapError::InvalidCapability`: Capability doesn't exist
- `CapError::InsufficientRights`: Missing REVOKE right

#### Capabilities Required
- Capability must have `REVOKE` right

#### Example
```rust
// Revoke a capability and all derived
capability_revoke(temp_cap)?;
```

## Memory Management

### memory_map

Maps a memory object into the process address space.

#### Signature
```rust
fn memory_map(
    mem_cap: CapabilityIndex,
    vaddr: Option<VirtAddr>,
    size: usize,
    flags: MapFlags
) -> Result<VirtAddr, MemError>
```

#### Parameters
- `mem_cap`: Capability for memory object
- `vaddr`: Preferred virtual address (None for any)
- `size`: Size to map in bytes
- `flags`: Mapping flags (READ, WRITE, EXECUTE)

#### Returns
Virtual address where memory was mapped.

#### Errors
- `MemError::InvalidCapability`: Invalid memory capability
- `MemError::InvalidAddress`: Requested address unavailable
- `MemError::OutOfMemory`: No virtual memory available
- `MemError::AccessDenied`: Capability lacks required rights

#### Capabilities Required
- Memory capability with appropriate rights matching flags

#### Example
```rust
// Map shared memory as read-write
let addr = memory_map(
    shared_mem_cap,
    None,
    4096,
    MapFlags::READ | MapFlags::WRITE
)?;
```

#### Safety
Mapped memory is guaranteed to be non-overlapping with existing mappings.

### memory_unmap

Unmaps previously mapped memory.

#### Signature
```rust
fn memory_unmap(vaddr: VirtAddr, size: usize) -> Result<(), MemError>
```

#### Parameters
- `vaddr`: Starting virtual address
- `size`: Size to unmap in bytes

#### Returns
Unit on success.

#### Errors
- `MemError::InvalidAddress`: Address not mapped
- `MemError::InvalidSize`: Size doesn't match mapping

#### Example
```rust
// Unmap previously mapped memory
memory_unmap(addr, 4096)?;
```

### memory_allocate

Allocates anonymous memory.

#### Signature
```rust
fn memory_allocate(
    size: usize,
    alignment: usize,
    flags: AllocFlags
) -> Result<CapabilityIndex, MemError>
```

#### Parameters
- `size`: Size in bytes to allocate
- `alignment`: Required alignment (must be power of 2)
- `flags`: Allocation flags (ZERO_FILL, EXECUTABLE, etc.)

#### Returns
Capability for allocated memory object.

#### Errors
- `MemError::OutOfMemory`: Insufficient memory
- `MemError::InvalidAlignment`: Alignment not power of 2
- `MemError::QuotaExceeded`: Memory quota exceeded

#### Example
```rust
// Allocate 1MB of zeroed memory
let mem_cap = memory_allocate(
    1024 * 1024,
    4096,
    AllocFlags::ZERO_FILL
)?;
```

## Process and Thread Management

### process_create

Creates a new process.

#### Signature
```rust
fn process_create(
    name: &str,
    entry_point: VirtAddr,
    initial_caps: &[CapabilityIndex]
) -> Result<ProcessHandle, ProcError>
```

#### Parameters
- `name`: Human-readable process name
- `entry_point`: Entry point address
- `initial_caps`: Capabilities to grant to new process

#### Returns
Handle to the newly created process.

#### Errors
- `ProcError::InvalidAddress`: Invalid entry point
- `ProcError::QuotaExceeded`: Process limit reached
- `ProcError::OutOfMemory`: Cannot allocate process

#### Capabilities Required
- `CAP_PROCESS_CREATE`: Required to create processes

#### Example
```rust
// Create a new process
let proc = process_create(
    "worker",
    entry_addr,
    &[memory_cap, port_cap]
)?;
```

### thread_create

Creates a new thread in the current process.

#### Signature
```rust
fn thread_create(
    entry: extern "C" fn(usize) -> !,
    arg: usize,
    stack_size: usize
) -> Result<ThreadId, ThreadError>
```

#### Parameters
- `entry`: Thread entry point function
- `arg`: Argument passed to entry function
- `stack_size`: Size of thread stack

#### Returns
ID of newly created thread.

#### Errors
- `ThreadError::OutOfMemory`: Cannot allocate stack
- `ThreadError::QuotaExceeded`: Thread limit reached

#### Example
```rust
// Create a worker thread
let tid = thread_create(worker_thread, 42, 64 * 1024)?;
```

### thread_yield

Yields CPU to another thread.

#### Signature
```rust
fn thread_yield() -> Result<(), ThreadError>
```

#### Returns
Always succeeds.

#### Example
```rust
// Yield to scheduler
thread_yield()?;
```

## Inter-Process Communication

### port_send

Sends a message through a port.

#### Signature
```rust
fn port_send(
    port: CapabilityIndex,
    message: &[u8],
    caps: &[CapabilityIndex]
) -> Result<(), IpcError>
```

#### Parameters
- `port`: Port capability
- `message`: Message data
- `caps`: Capabilities to transfer

#### Returns
Unit on success.

#### Errors
- `IpcError::InvalidPort`: Invalid port capability
- `IpcError::PortFull`: Port queue is full
- `IpcError::MessageTooLarge`: Message exceeds port limit

#### Capabilities Required
- Port capability with `WRITE` right

#### Example
```rust
// Send message with capability
let msg = b"Hello";
port_send(port_cap, msg, &[shared_mem_cap])?;
```

### port_receive

Receives a message from a port.

#### Signature
```rust
fn port_receive(
    port: CapabilityIndex,
    buffer: &mut [u8],
    caps: &mut Vec<CapabilityIndex>,
    timeout: Option<Duration>
) -> Result<usize, IpcError>
```

#### Parameters
- `port`: Port capability
- `buffer`: Buffer for message data
- `caps`: Vector to receive capabilities
- `timeout`: Optional timeout

#### Returns
Number of bytes received.

#### Errors
- `IpcError::InvalidPort`: Invalid port capability
- `IpcError::Timeout`: Operation timed out
- `IpcError::BufferTooSmall`: Buffer cannot hold message

#### Capabilities Required
- Port capability with `READ` right

#### Example
```rust
// Receive message with timeout
let mut buf = [0u8; 1024];
let mut caps = Vec::new();
let len = port_receive(
    port_cap,
    &mut buf,
    &mut caps,
    Some(Duration::from_secs(5))
)?;
```

### channel_create

Creates a bidirectional communication channel.

#### Signature
```rust
fn channel_create() -> Result<(CapabilityIndex, CapabilityIndex), IpcError>
```

#### Returns
Tuple of (endpoint0, endpoint1) capabilities.

#### Errors
- `IpcError::QuotaExceeded`: Channel limit reached
- `IpcError::OutOfMemory`: Cannot allocate channel

#### Example
```rust
// Create channel
let (ep0, ep1) = channel_create()?;
// Transfer ep1 to another process
```

## File System Operations

### file_open

Opens a file.

#### Signature
```rust
fn file_open(
    path: &str,
    flags: OpenFlags,
    mode: FileMode
) -> Result<CapabilityIndex, FsError>
```

#### Parameters
- `path`: File path
- `flags`: Open flags (READ, WRITE, CREATE, etc.)
- `mode`: Permissions for new files

#### Returns
File capability.

#### Errors
- `FsError::NotFound`: File doesn't exist
- `FsError::PermissionDenied`: Insufficient permissions
- `FsError::AlreadyExists`: File exists with O_EXCL

#### Capabilities Required
- Appropriate directory capabilities in path

#### Example
```rust
// Open file for reading
let file = file_open(
    "/data/config.toml",
    OpenFlags::READ,
    FileMode::empty()
)?;
```

### file_read

Reads from a file.

#### Signature
```rust
fn file_read(
    file: CapabilityIndex,
    buffer: &mut [u8],
    offset: Option<u64>
) -> Result<usize, FsError>
```

#### Parameters
- `file`: File capability
- `buffer`: Buffer to read into
- `offset`: Optional offset (None uses current position)

#### Returns
Number of bytes read.

#### Errors
- `FsError::InvalidFile`: Invalid file capability
- `FsError::IoError`: I/O error occurred

#### Capabilities Required
- File capability with `READ` right

#### Example
```rust
let mut buf = [0u8; 1024];
let n = file_read(file_cap, &mut buf, None)?;
```

### file_write

Writes to a file.

#### Signature
```rust
fn file_write(
    file: CapabilityIndex,
    data: &[u8],
    offset: Option<u64>
) -> Result<usize, FsError>
```

#### Parameters
- `file`: File capability
- `data`: Data to write
- `offset`: Optional offset (None uses current position)

#### Returns
Number of bytes written.

#### Errors
- `FsError::InvalidFile`: Invalid file capability
- `FsError::NoSpace`: Filesystem full
- `FsError::IoError`: I/O error occurred

#### Capabilities Required
- File capability with `WRITE` right

#### Example
```rust
let data = b"Hello, World!";
let n = file_write(file_cap, data, Some(0))?;
```

## Networking APIs

### socket_create

Creates a network socket.

#### Signature
```rust
fn socket_create(
    domain: AddressFamily,
    sock_type: SocketType,
    protocol: Protocol
) -> Result<CapabilityIndex, NetError>
```

#### Parameters
- `domain`: Address family (AF_INET, AF_INET6, etc.)
- `sock_type`: Socket type (STREAM, DGRAM, etc.)
- `protocol`: Protocol (TCP, UDP, etc.)

#### Returns
Socket capability.

#### Errors
- `NetError::InvalidDomain`: Unsupported address family
- `NetError::InvalidType`: Unsupported socket type
- `NetError::OutOfMemory`: Cannot allocate socket

#### Capabilities Required
- `CAP_NET_CREATE`: Required to create sockets

#### Example
```rust
// Create TCP socket
let sock = socket_create(
    AddressFamily::Inet,
    SocketType::Stream,
    Protocol::Tcp
)?;
```

### socket_bind

Binds a socket to an address.

#### Signature
```rust
fn socket_bind(
    socket: CapabilityIndex,
    addr: &SocketAddr
) -> Result<(), NetError>
```

#### Parameters
- `socket`: Socket capability
- `addr`: Address to bind to

#### Returns
Unit on success.

#### Errors
- `NetError::InvalidSocket`: Invalid socket capability
- `NetError::AddressInUse`: Address already bound
- `NetError::PermissionDenied`: Cannot bind to port

#### Capabilities Required
- Socket capability with `BIND` right
- `CAP_NET_BIND_PRIVILEGED` for ports < 1024

#### Example
```rust
// Bind to port 8080
let addr = "127.0.0.1:8080".parse()?;
socket_bind(sock_cap, &addr)?;
```

### socket_connect

Connects a socket.

#### Signature
```rust
fn socket_connect(
    socket: CapabilityIndex,
    addr: &SocketAddr
) -> Result<(), NetError>
```

#### Parameters
- `socket`: Socket capability
- `addr`: Address to connect to

#### Returns
Unit on success.

#### Errors
- `NetError::InvalidSocket`: Invalid socket capability
- `NetError::ConnectionRefused`: Connection refused
- `NetError::Timeout`: Connection timed out

#### Example
```rust
// Connect to server
let addr = "192.168.1.1:80".parse()?;
socket_connect(sock_cap, &addr)?;
```

## Device I/O

### device_open

Opens a device.

#### Signature
```rust
fn device_open(
    path: &str,
    flags: DeviceFlags
) -> Result<CapabilityIndex, DevError>
```

#### Parameters
- `path`: Device path
- `flags`: Access flags

#### Returns
Device capability.

#### Errors
- `DevError::NotFound`: Device doesn't exist
- `DevError::Busy`: Device already in use
- `DevError::PermissionDenied`: Insufficient permissions

#### Capabilities Required
- `CAP_DEVICE_ACCESS`: Required for device access

#### Example
```rust
// Open serial port
let serial = device_open(
    "/dev/ttyS0",
    DeviceFlags::READ | DeviceFlags::WRITE
)?;
```

### device_ioctl

Performs device-specific control operation.

#### Signature
```rust
fn device_ioctl(
    device: CapabilityIndex,
    request: u32,
    arg: *mut c_void
) -> Result<i32, DevError>
```

#### Parameters
- `device`: Device capability
- `request`: Device-specific request code
- `arg`: Request-specific argument

#### Returns
Device-specific return value.

#### Errors
- `DevError::InvalidDevice`: Invalid device capability
- `DevError::InvalidRequest`: Unknown request code
- `DevError::IoError`: I/O error occurred

#### Safety
Caller must ensure `arg` points to valid memory for the specific request.

#### Example
```rust
// Set serial baud rate
let mut termios = Termios::default();
device_ioctl(serial_cap, TCGETS, &mut termios as *mut _)?;
```

## Time and Timer Management

### time_get

Gets current system time.

#### Signature
```rust
fn time_get(clock: ClockId) -> Result<TimeSpec, TimeError>
```

#### Parameters
- `clock`: Clock to query (MONOTONIC, REALTIME, etc.)

#### Returns
Current time value.

#### Errors
- `TimeError::InvalidClock`: Unknown clock ID

#### Example
```rust
// Get monotonic time
let now = time_get(ClockId::Monotonic)?;
```

### timer_create

Creates a timer.

#### Signature
```rust
fn timer_create(
    clock: ClockId,
    notification: TimerNotification
) -> Result<CapabilityIndex, TimeError>
```

#### Parameters
- `clock`: Clock source for timer
- `notification`: How to notify on expiration

#### Returns
Timer capability.

#### Errors
- `TimeError::InvalidClock`: Unknown clock ID
- `TimeError::QuotaExceeded`: Timer limit reached

#### Example
```rust
// Create timer with port notification
let timer = timer_create(
    ClockId::Monotonic,
    TimerNotification::Port(port_cap)
)?;
```

### timer_set

Sets timer expiration.

#### Signature
```rust
fn timer_set(
    timer: CapabilityIndex,
    value: &TimerSpec,
    flags: TimerFlags
) -> Result<TimerSpec, TimeError>
```

#### Parameters
- `timer`: Timer capability
- `value`: New timer value
- `flags`: Timer flags (ABSTIME, etc.)

#### Returns
Previous timer value.

#### Errors
- `TimeError::InvalidTimer`: Invalid timer capability
- `TimeError::InvalidValue`: Invalid timer specification

#### Example
```rust
// Set timer for 1 second
let spec = TimerSpec {
    value: Duration::from_secs(1),
    interval: Duration::ZERO,
};
timer_set(timer_cap, &spec, TimerFlags::empty())?;
```

## Error Handling

### Error Types

All system calls return `Result<T, E>` where errors are strongly typed:

```rust
#[derive(Debug, Error)]
pub enum CapError {
    #[error("invalid capability")]
    InvalidCapability,
    
    #[error("insufficient rights")]
    InsufficientRights,
    
    #[error("quota exceeded")]
    QuotaExceeded,
    
    // ... more variants
}
```

### Error Conversion

User-space wrapper provides convenient error conversion:

```rust
impl From<i64> for SystemError {
    fn from(errno: i64) -> Self {
        match errno {
            -1 => SystemError::InvalidArgument,
            -2 => SystemError::OutOfMemory,
            -3 => SystemError::PermissionDenied,
            // ... more mappings
            _ => SystemError::Unknown(errno),
        }
    }
}
```

### Error Handling Best Practices

```rust
// Use ? operator for propagation
fn process_file(path: &str) -> Result<String, Error> {
    let file = file_open(path, OpenFlags::READ, FileMode::empty())?;
    let mut buffer = String::new();
    file_read_to_string(file, &mut buffer)?;
    file_close(file)?;
    Ok(buffer)
}

// Handle specific errors
match socket_connect(sock, &addr) {
    Ok(()) => println!("Connected"),
    Err(NetError::ConnectionRefused) => {
        println!("Server not running");
    }
    Err(NetError::Timeout) => {
        println!("Connection timed out");
    }
    Err(e) => return Err(e.into()),
}
```

## Appendix A: Capability Rights Reference

```rust
bitflags! {
    pub struct CapabilityRights: u64 {
        const READ         = 0x0000_0001;
        const WRITE        = 0x0000_0002;
        const EXECUTE      = 0x0000_0004;
        const MAP          = 0x0000_0008;
        const DUPLICATE    = 0x0000_0010;
        const TRANSFER     = 0x0000_0020;
        const DELETE       = 0x0000_0040;
        const DERIVE       = 0x0000_0080;
        const REVOKE       = 0x0000_0100;
        const SEAL         = 0x0000_0200;
        const GRANT        = 0x0000_0400;
        const CONNECT      = 0x0000_0800;
        const LISTEN       = 0x0000_1000;
        const ACCEPT       = 0x0000_2000;
        const BIND         = 0x0000_4000;
        const SHUTDOWN     = 0x0000_8000;
        const SIGNAL       = 0x0001_0000;
        const WAIT         = 0x0002_0000;
        const POLL         = 0x0004_0000;
        // ... more rights
    }
}
```

## Appendix B: System Limits

```rust
// System-wide limits
pub const MAX_PROCESSES: usize = 65536;
pub const MAX_THREADS_PER_PROCESS: usize = 4096;
pub const MAX_CAPABILITIES_PER_PROCESS: usize = 65536;
pub const MAX_OPEN_FILES_PER_PROCESS: usize = 1024;
pub const MAX_MEMORY_PER_PROCESS: usize = 128 * 1024 * 1024 * 1024; // 128GB

// IPC limits
pub const MAX_MESSAGE_SIZE: usize = 65536;
pub const MAX_CAPABILITIES_PER_MESSAGE: usize = 16;
pub const MAX_PORT_QUEUE_SIZE: usize = 1000;

// Network limits
pub const MAX_SOCKETS_PER_PROCESS: usize = 1024;
pub const MAX_LISTEN_BACKLOG: usize = 128;
```

## Version History

- v0.1.0: Initial API design
- v0.2.0: Added capability system
- v0.3.0: Network API additions
- v0.4.0: Device I/O framework
- v0.5.0: Timer subsystem

---

This API reference is generated from the Veridian OS source code. For the most up-to-date information, consult the online documentation at https://docs.veridian-os.org/api/
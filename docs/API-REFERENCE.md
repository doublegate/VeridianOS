# VeridianOS API Reference

## System Call Interface

### Process Management

#### `sys_process_create`
Creates a new process.

```rust
fn sys_process_create(
    name: *const u8,
    name_len: usize,
    entry_point: usize,
) -> Result<Pid, Error>
```

**Parameters:**
- `name`: Process name (UTF-8)
- `name_len`: Length of name
- `entry_point`: Entry point address

**Returns:**
- `Ok(Pid)`: Process ID of created process
- `Err(Error)`: Error code

**Errors:**
- `EINVAL`: Invalid parameters
- `ENOMEM`: Out of memory
- `EACCES`: Permission denied

#### `sys_process_exit`
Terminates the current process.

```rust
fn sys_process_exit(code: i32) -> !
```

**Parameters:**
- `code`: Exit code

**Note:** This function never returns.

#### `sys_thread_create`
Creates a new thread in the current process.

```rust
fn sys_thread_create(
    entry: fn(usize),
    arg: usize,
    stack_size: usize,
) -> Result<ThreadId, Error>
```

**Parameters:**
- `entry`: Thread entry point
- `arg`: Argument passed to thread
- `stack_size`: Stack size in bytes

**Returns:**
- `Ok(ThreadId)`: Thread ID
- `Err(Error)`: Error code

### Memory Management

#### `sys_mmap`
Maps memory into the process address space.

```rust
fn sys_mmap(
    addr: Option<*mut u8>,
    len: usize,
    prot: Protection,
    flags: MapFlags,
) -> Result<*mut u8, Error>
```

**Parameters:**
- `addr`: Preferred address (None for any)
- `len`: Length in bytes
- `prot`: Memory protection flags
- `flags`: Mapping flags

**Protection Flags:**
```rust
bitflags! {
    pub struct Protection: u32 {
        const READ    = 0x1;
        const WRITE   = 0x2;
        const EXECUTE = 0x4;
    }
}
```

**Map Flags:**
```rust
bitflags! {
    pub struct MapFlags: u32 {
        const PRIVATE   = 0x1;
        const SHARED    = 0x2;
        const ANONYMOUS = 0x4;
        const FIXED     = 0x8;
    }
}
```

#### `sys_munmap`
Unmaps memory from the process address space.

```rust
fn sys_munmap(addr: *mut u8, len: usize) -> Result<(), Error>
```

**Parameters:**
- `addr`: Start address
- `len`: Length in bytes

**Returns:**
- `Ok(())`: Success
- `Err(Error)`: Error code

### Inter-Process Communication

#### `sys_endpoint_create`
Creates an IPC endpoint.

```rust
fn sys_endpoint_create(max_msg_size: usize) -> Result<EndpointId, Error>
```

**Parameters:**
- `max_msg_size`: Maximum message size

**Returns:**
- `Ok(EndpointId)`: Endpoint identifier
- `Err(Error)`: Error code

#### `sys_send`
Sends a message to an endpoint.

```rust
fn sys_send(
    endpoint: EndpointId,
    data: *const u8,
    len: usize,
    caps: *const CapabilityId,
    cap_count: usize,
) -> Result<(), Error>
```

**Parameters:**
- `endpoint`: Target endpoint
- `data`: Message data
- `len`: Data length
- `caps`: Capabilities to transfer
- `cap_count`: Number of capabilities

**Returns:**
- `Ok(())`: Message sent
- `Err(Error)`: Error code

#### `sys_receive`
Receives a message from an endpoint.

```rust
fn sys_receive(
    endpoint: EndpointId,
    data: *mut u8,
    len: usize,
    caps: *mut CapabilityId,
    cap_count: usize,
) -> Result<(usize, usize), Error>
```

**Parameters:**
- `endpoint`: Source endpoint
- `data`: Buffer for message data
- `len`: Buffer length
- `caps`: Buffer for capabilities
- `cap_count`: Capability buffer size

**Returns:**
- `Ok((data_len, cap_len))`: Actual lengths
- `Err(Error)`: Error code

### Capability Management

#### `sys_cap_create`
Creates a new capability.

```rust
fn sys_cap_create(
    resource: ResourceType,
    permissions: Permissions,
) -> Result<CapabilityId, Error>
```

**Resource Types:**
```rust
pub enum ResourceType {
    Memory { start: usize, size: usize },
    Endpoint(EndpointId),
    Process(Pid),
    Thread(ThreadId),
    Device(DeviceId),
}
```

**Permissions:**
```rust
bitflags! {
    pub struct Permissions: u32 {
        const READ    = 0x1;
        const WRITE   = 0x2;
        const EXECUTE = 0x4;
        const GRANT   = 0x8;
        const REVOKE  = 0x10;
    }
}
```

#### `sys_cap_derive`
Derives a new capability with reduced permissions.

```rust
fn sys_cap_derive(
    parent: CapabilityId,
    permissions: Permissions,
) -> Result<CapabilityId, Error>
```

**Parameters:**
- `parent`: Parent capability
- `permissions`: New permissions (must be subset)

**Returns:**
- `Ok(CapabilityId)`: Derived capability
- `Err(Error)`: Error code

### File System

#### `sys_open`
Opens a file.

```rust
fn sys_open(
    path: *const u8,
    path_len: usize,
    flags: OpenFlags,
    mode: Mode,
) -> Result<FileDescriptor, Error>
```

**Open Flags:**
```rust
bitflags! {
    pub struct OpenFlags: u32 {
        const READ     = 0x1;
        const WRITE    = 0x2;
        const CREATE   = 0x4;
        const EXCL     = 0x8;
        const TRUNC    = 0x10;
        const APPEND   = 0x20;
    }
}
```

#### `sys_read`
Reads from a file descriptor.

```rust
fn sys_read(
    fd: FileDescriptor,
    buf: *mut u8,
    count: usize,
) -> Result<usize, Error>
```

**Parameters:**
- `fd`: File descriptor
- `buf`: Buffer to read into
- `count`: Maximum bytes to read

**Returns:**
- `Ok(n)`: Number of bytes read
- `Err(Error)`: Error code

#### `sys_write`
Writes to a file descriptor.

```rust
fn sys_write(
    fd: FileDescriptor,
    buf: *const u8,
    count: usize,
) -> Result<usize, Error>
```

**Parameters:**
- `fd`: File descriptor
- `buf`: Data to write
- `count`: Number of bytes to write

**Returns:**
- `Ok(n)`: Number of bytes written
- `Err(Error)`: Error code

## Kernel Libraries

### Memory Allocator

```rust
use veridian_kernel::mm::{FrameAllocator, PageAllocator};

// Allocate physical frame
let frame = FRAME_ALLOCATOR.lock().allocate()?;

// Allocate virtual page
let page = PAGE_ALLOCATOR.lock().allocate()?;

// Map page to frame
page_table.map(page, frame, PageFlags::WRITABLE)?;
```

### Synchronization Primitives

#### Mutex
```rust
use veridian_kernel::sync::Mutex;

static COUNTER: Mutex<u32> = Mutex::new(0);

// Lock and increment
*COUNTER.lock() += 1;
```

#### RwLock
```rust
use veridian_kernel::sync::RwLock;

static CONFIG: RwLock<Config> = RwLock::new(Config::default());

// Read access
let config = CONFIG.read();

// Write access
CONFIG.write().update();
```

#### Semaphore
```rust
use veridian_kernel::sync::Semaphore;

let sem = Semaphore::new(5); // 5 resources

// Acquire
sem.acquire();

// Release
sem.release();
```

### Collections

#### B-Tree Map
```rust
use veridian_kernel::collections::BTreeMap;

let mut map = BTreeMap::new();
map.insert("key", "value");

if let Some(value) = map.get("key") {
    println!("Found: {}", value);
}
```

#### Vector
```rust
use veridian_kernel::collections::Vec;

let mut vec = Vec::new();
vec.push(42);
vec.extend_from_slice(&[1, 2, 3]);
```

## Driver Framework

### Driver Trait

All drivers must implement the `Driver` trait:

```rust
pub trait Driver: Send + Sync {
    /// Initialize the driver
    fn init(&mut self, device: DeviceInfo) -> Result<(), Error>;
    
    /// Handle interrupt
    fn handle_interrupt(&mut self, vector: u8);
    
    /// Handle control message
    fn handle_message(&mut self, msg: Message) -> Result<Response, Error>;
    
    /// Cleanup on shutdown
    fn cleanup(&mut self);
}
```

### Device Access

```rust
use veridian_driver::{MmioRegion, Port};

// Memory-mapped I/O
let mmio = MmioRegion::new(0x1000_0000, 0x1000)?;
let value: u32 = mmio.read(0x10);
mmio.write(0x20, 0x1234u32);

// Port I/O (x86)
let port = Port::<u16>::new(0x3F8);
port.write(0x41);
let status = port.read();
```

### DMA Operations

```rust
use veridian_driver::dma::{DmaBuffer, DmaDirection};

// Allocate DMA buffer
let buffer = DmaBuffer::new(4096)?;

// Get physical address for device
let phys_addr = buffer.physical_address();

// Access buffer
buffer.as_slice_mut().fill(0);

// Sync for device access
buffer.sync_for_device(DmaDirection::ToDevice)?;

// After device writes
buffer.sync_for_cpu(DmaDirection::FromDevice)?;
```

## User-Space Library

### Process Control

```rust
use veridian::process;

// Fork process
match process::fork() {
    Ok(pid) if pid == 0 => {
        // Child process
        process::exec("/bin/sh", &["sh", "-c", "echo hello"])?;
    }
    Ok(pid) => {
        // Parent process
        let status = process::wait(pid)?;
    }
    Err(e) => eprintln!("Fork failed: {}", e),
}
```

### Memory Management

```rust
use veridian::mem::{mmap, Protection, MapFlags};

// Allocate anonymous memory
let addr = mmap(
    None,
    4096,
    Protection::READ | Protection::WRITE,
    MapFlags::PRIVATE | MapFlags::ANONYMOUS,
)?;

// Use memory
unsafe {
    let slice = core::slice::from_raw_parts_mut(addr, 4096);
    slice.fill(0);
}
```

### IPC

```rust
use veridian::ipc::{Endpoint, Message};

// Create endpoint
let endpoint = Endpoint::create(1024)?;

// Send message
let msg = Message::new(b"Hello, IPC!");
endpoint.send(msg)?;

// Receive message
let received = endpoint.receive()?;
println!("Received: {:?}", received.data());
```

### File I/O

```rust
use veridian::fs::{File, OpenOptions};
use veridian::io::{Read, Write};

// Open file
let mut file = File::open("/etc/config.toml")?;

// Read contents
let mut contents = String::new();
file.read_to_string(&mut contents)?;

// Write file
let mut output = OpenOptions::new()
    .create(true)
    .write(true)
    .open("/tmp/output.txt")?;
    
output.write_all(b"Hello, World!\n")?;
```

### Networking

```rust
use veridian::net::{TcpListener, TcpStream};

// Server
let listener = TcpListener::bind("127.0.0.1:8080")?;
for stream in listener.incoming() {
    handle_client(stream?)?;
}

// Client
let mut stream = TcpStream::connect("127.0.0.1:8080")?;
stream.write_all(b"GET / HTTP/1.0\r\n\r\n")?;
```

## Error Handling

### Error Types

```rust
#[derive(Debug)]
pub enum Error {
    /// Invalid argument
    InvalidArgument,
    /// Resource not found
    NotFound,
    /// Permission denied
    PermissionDenied,
    /// Out of memory
    OutOfMemory,
    /// I/O error
    Io(IoError),
    /// Would block
    WouldBlock,
    /// System error with errno
    System(i32),
}

impl Error {
    pub fn errno(&self) -> i32 {
        match self {
            Error::InvalidArgument => EINVAL,
            Error::NotFound => ENOENT,
            Error::PermissionDenied => EACCES,
            Error::OutOfMemory => ENOMEM,
            Error::WouldBlock => EAGAIN,
            Error::System(errno) => *errno,
            _ => EIO,
        }
    }
}
```

### Error Constants

```rust
pub const EPERM: i32 = 1;      // Operation not permitted
pub const ENOENT: i32 = 2;     // No such file or directory
pub const ESRCH: i32 = 3;      // No such process
pub const EINTR: i32 = 4;      // Interrupted system call
pub const EIO: i32 = 5;        // I/O error
pub const ENXIO: i32 = 6;      // No such device or address
pub const E2BIG: i32 = 7;      // Argument list too long
pub const ENOEXEC: i32 = 8;    // Exec format error
pub const EBADF: i32 = 9;      // Bad file descriptor
pub const ECHILD: i32 = 10;    // No child processes
pub const EAGAIN: i32 = 11;    // Try again
pub const ENOMEM: i32 = 12;    // Out of memory
pub const EACCES: i32 = 13;    // Permission denied
pub const EFAULT: i32 = 14;    // Bad address
pub const EBUSY: i32 = 16;     // Device or resource busy
pub const EEXIST: i32 = 17;    // File exists
pub const ENODEV: i32 = 19;    // No such device
pub const ENOTDIR: i32 = 20;   // Not a directory
pub const EISDIR: i32 = 21;    // Is a directory
pub const EINVAL: i32 = 22;    // Invalid argument
pub const ENFILE: i32 = 23;    // File table overflow
pub const EMFILE: i32 = 24;    // Too many open files
pub const ENOTTY: i32 = 25;    // Not a typewriter
pub const ETXTBSY: i32 = 26;   // Text file busy
pub const EFBIG: i32 = 27;     // File too large
pub const ENOSPC: i32 = 28;    // No space left on device
pub const ESPIPE: i32 = 29;    // Illegal seek
pub const EROFS: i32 = 30;     // Read-only file system
pub const EMLINK: i32 = 31;    // Too many links
pub const EPIPE: i32 = 32;     // Broken pipe
pub const EDOM: i32 = 33;      // Math argument out of domain
pub const ERANGE: i32 = 34;    // Math result not representable
```

## Performance Considerations

### System Call Overhead

Typical system call latencies:
- Simple syscall (getpid): ~100-200 cycles
- Memory allocation: ~500-1000 cycles  
- IPC message: ~1000-2000 cycles
- File I/O: ~5000+ cycles

### Best Practices

1. **Batch Operations**: Combine multiple operations when possible
2. **Use Async I/O**: Prefer io_uring for high-performance I/O
3. **Memory Pooling**: Reuse allocations to reduce syscall overhead
4. **Capability Caching**: Cache frequently used capabilities
5. **Zero-Copy IPC**: Use shared memory for large data transfers

## Version History

- **0.1.0**: Initial API design
- **0.2.0**: Added capability system
- **0.3.0**: Enhanced IPC mechanisms
- **0.4.0**: File system support
- **0.5.0**: Networking APIs
- **1.0.0**: Stable API release
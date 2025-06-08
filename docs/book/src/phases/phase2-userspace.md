# Phase 2: User Space Foundation

Phase 2 (Months 10-15) establishes the user space environment, transforming the microkernel into a usable operating system by implementing essential system services, user libraries, and foundational components.

## Overview

This phase creates the bridge between the microkernel and user applications through:
- **Init System**: Process management and service orchestration
- **Device Drivers**: User-space driver framework
- **Virtual File System**: Unified file system interface
- **Network Stack**: TCP/IP implementation
- **Standard Library**: POSIX-compatible C library in Rust
- **Basic Shell**: Interactive command environment

## Key Design Decisions

### POSIX Compatibility Strategy

VeridianOS implements a three-layer architecture for POSIX compatibility:

```
┌─────────────────────────────┐
│    POSIX API Layer         │  Standard POSIX functions
├─────────────────────────────┤
│   Translation Layer        │  POSIX → Capabilities
├─────────────────────────────┤
│   Native IPC Layer         │  Zero-copy VeridianOS IPC
└─────────────────────────────┘
```

This approach provides:
- **Compatibility**: Easy porting of existing software
- **Security**: Capability-based access control
- **Performance**: Native IPC for critical paths

### Process Model

VeridianOS uses `spawn()` instead of `fork()` for security:

```rust
// Traditional Unix pattern (NOT used)
pid_t pid = fork();
if (pid == 0) {
    execve(path, argv, envp);
}

// VeridianOS pattern
pid_t pid;
posix_spawn(&pid, path, NULL, NULL, argv, envp);
```

Benefits:
- No address space duplication
- Explicit capability inheritance
- Better performance and security

## Init System Architecture

### Service Manager

The init process (PID 1) manages all system services:

```rust
pub struct Service {
    name: String,
    path: String,
    dependencies: Vec<String>,
    restart_policy: RestartPolicy,
    capabilities: Vec<Capability>,
    state: ServiceState,
}

pub enum RestartPolicy {
    Never,        // Don't restart
    OnFailure,    // Restart only on failure
    Always,       // Always restart
}
```

### Service Configuration

Services are defined in TOML files:

```toml
[[services]]
name = "vfs"
path = "/sbin/vfs"
restart_policy = "always"
capabilities = ["CAP_FS_MOUNT", "CAP_IPC_CREATE"]

[[services]]
name = "netstack"
path = "/sbin/netstack"
depends_on = ["devmgr"]
restart_policy = "always"
capabilities = ["CAP_NET_ADMIN", "CAP_NET_RAW"]
```

## Device Driver Framework

### User-Space Drivers

All drivers run in user space for isolation:

```rust
pub trait Driver {
    /// Initialize with device information
    fn init(&mut self, device: DeviceInfo) -> Result<(), Error>;
    
    /// Handle hardware interrupt
    fn handle_interrupt(&mut self, vector: u8);
    
    /// Process control messages
    fn handle_message(&mut self, msg: Message) -> Result<Response, Error>;
}
```

### Device Manager

The device manager service:
1. Enumerates hardware (PCI, platform devices)
2. Matches devices with drivers
3. Loads appropriate drivers
4. Manages device lifecycles

```rust
// Device enumeration
for bus in 0..256 {
    for device in 0..32 {
        let vendor_id = pci_read_u16(bus, device, 0, 0x00);
        if vendor_id != 0xFFFF {
            // Device found, load driver
            load_driver_for_device(vendor_id, device_id)?;
        }
    }
}
```

## Virtual File System

### VFS Architecture

The VFS provides a unified interface to different file systems:

```rust
pub struct VNode {
    id: VNodeId,
    node_type: VNodeType,
    parent: Option<VNodeId>,
    children: BTreeMap<String, VNodeId>,
    fs: Option<FsId>,
}

pub enum VNodeType {
    Directory,
    RegularFile,
    SymbolicLink,
    Device,
    Pipe,
    Socket,
}
```

### File Operations

POSIX-compatible file operations:

```rust
// Open file
let fd = open("/etc/config.toml", O_RDONLY)?;

// Read data
let mut buffer = [0u8; 1024];
let n = read(fd, &mut buffer)?;

// Close file
close(fd)?;
```

### Supported File Systems

1. **tmpfs**: RAM-based temporary storage
2. **devfs**: Device file system (/dev)
3. **procfs**: Process information (/proc)
4. **ext2**: Basic persistent storage (Phase 3)

## Network Stack

### TCP/IP Implementation

Based on smoltcp for initial implementation:

```rust
pub struct NetworkStack {
    interfaces: Vec<NetworkInterface>,
    tcp_sockets: Slab<TcpSocket>,
    udp_sockets: Slab<UdpSocket>,
    routes: RoutingTable,
}

// Socket operations
let socket = socket(AF_INET, SOCK_STREAM, 0)?;
connect(socket, &addr)?;
send(socket, data, 0)?;
```

### Network Architecture

```
┌─────────────────────┐
│   Applications      │
├─────────────────────┤
│   BSD Socket API    │
├─────────────────────┤
│   TCP/UDP Layer     │
├─────────────────────┤
│   IP Layer          │
├─────────────────────┤
│   Ethernet Driver   │
└─────────────────────┘
```

## Standard Library

### libveridian Design

A POSIX-compatible C library written in Rust:

```rust
// Memory allocation
pub unsafe fn malloc(size: usize) -> *mut c_void {
    let layout = Layout::from_size_align(size, 8).unwrap();
    ALLOCATOR.alloc(layout) as *mut c_void
}

// File operations
pub fn open(path: *const c_char, flags: c_int) -> c_int {
    let path = unsafe { CStr::from_ptr(path) };
    match syscall::open(path.to_str().unwrap(), flags.into()) {
        Ok(fd) => fd as c_int,
        Err(_) => -1,
    }
}
```

### Implementation Priority

1. **Memory**: malloc, free, mmap
2. **I/O**: open, read, write, close
3. **Process**: spawn, wait, exit
4. **Threading**: pthread_create, mutex, condvar
5. **Network**: socket, connect, send, recv

## Basic Shell (vsh)

### Features

- Command execution
- Built-in commands (cd, pwd, export)
- Environment variables
- Command history
- Job control (basic)

```rust
// Shell main loop
loop {
    print!("{}> ", cwd);
    let input = read_line();
    
    match parse_command(input) {
        Command::Builtin(cmd) => execute_builtin(cmd),
        Command::External(cmd, args) => {
            let pid = spawn(cmd, args)?;
            wait(pid)?;
        }
    }
}
```

## Implementation Timeline

### Month 10-11: Foundation
- Init system and service management
- Device manager framework
- Basic driver loading

### Month 12: File Systems
- VFS core implementation
- tmpfs and devfs
- Basic file operations

### Month 13: Extended File Systems
- procfs implementation
- File system mounting
- Path resolution

### Month 14: Networking
- Network service architecture
- TCP/IP stack integration
- Socket API

### Month 15: User Environment
- Standard library completion
- Shell implementation
- Basic utilities

## Performance Targets

| Component | Metric | Target |
|-----------|--------|--------|
| Service startup | Time to start | <100ms |
| File open | Latency | <10μs |
| Network socket | Creation time | <50μs |
| Shell command | Launch time | <5ms |

## Testing Strategy

### Unit Tests
- Service dependency resolution
- VFS path lookup algorithms
- Network protocol correctness
- Library function compliance

### Integration Tests
- Multi-service interaction
- File system operations
- Network connectivity
- Shell command execution

### Stress Tests
- Service restart cycles
- Concurrent file access
- Network load testing
- Memory allocation patterns

## Success Criteria

1. **Stable Init**: Services start reliably with proper dependencies
2. **Driver Support**: Common hardware works (storage, network, serial)
3. **File System**: POSIX-compliant operations work correctly
4. **Networking**: Can establish TCP connections and transfer data
5. **User Experience**: Shell provides usable interactive environment
6. **Performance**: Meets or exceeds target metrics

## Challenges and Solutions

### Challenge: Driver Isolation
**Solution**: Capability-based hardware access with IOMMU protection

### Challenge: POSIX Semantics
**Solution**: Translation layer maps POSIX to capability model

### Challenge: Performance
**Solution**: Zero-copy IPC and efficient caching

## Next Phase Dependencies

Phase 3 (Security Hardening) requires:
- Stable user-space environment
- Working file system for policy storage
- Network stack for remote attestation
- Shell for administrative tasks
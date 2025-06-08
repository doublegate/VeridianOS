# VeridianOS System Services

This directory contains core system services that run in user space.

## Overview

System services provide essential OS functionality outside the microkernel:
- File systems and storage management
- Network protocol stacks
- Device management
- User session management

## Structure

```
services/
├── init/              # System initialization service
├── vfs/               # Virtual File System service
├── devmgr/            # Device manager service
├── netstack/          # Network stack service
├── storage/           # Storage management service
├── auth/              # Authentication service
├── logger/            # System logging service
└── common/            # Shared service utilities
```

## Service Architecture

Each service follows a common pattern:

```rust
pub trait Service: Send + Sync {
    /// Service identifier
    fn name(&self) -> &str;
    
    /// Start the service
    async fn start(&mut self) -> Result<(), ServiceError>;
    
    /// Handle incoming IPC messages
    async fn handle_message(&mut self, msg: Message) -> Result<Response, ServiceError>;
    
    /// Stop the service gracefully
    async fn stop(&mut self) -> Result<(), ServiceError>;
}
```

## Core Services

### Init Service
- First user process (PID 1)
- Starts other services
- Handles system shutdown/reboot
- Manages service dependencies

### VFS Service
- Provides POSIX-like file system interface
- Manages mount points
- Coordinates with storage drivers
- Implements file system caching

### Device Manager
- Enumerates hardware devices
- Loads appropriate drivers
- Manages device capabilities
- Handles hot-plug events

### Network Stack
- TCP/IP protocol implementation
- Socket API for applications
- Network interface management
- Routing and firewall rules

### Storage Service
- Logical volume management
- RAID functionality
- File system mounting
- Storage device abstraction

## IPC Protocols

Services communicate using defined protocols:

```rust
// Example: VFS protocol
pub enum VfsRequest {
    Open { path: Path, flags: OpenFlags },
    Read { fd: FileDescriptor, count: usize },
    Write { fd: FileDescriptor, data: Vec<u8> },
    Close { fd: FileDescriptor },
}

pub enum VfsResponse {
    Fd(FileDescriptor),
    Data(Vec<u8>),
    Count(usize),
    Error(VfsError),
}
```

## Capabilities

Services require various capabilities:
- `CAP_SERVICE`: Register as system service
- `CAP_IPC_CREATE`: Create IPC endpoints
- `CAP_SPAWN`: Create new processes
- Additional caps based on function

## Building Services

```bash
# Build all services
just build-services

# Build specific service
cd services/vfs && cargo build
```

## Configuration

Services are configured via:
- Command-line arguments
- Configuration files in `/etc/veridian/`
- Environment variables
- IPC configuration messages

## Testing

Each service includes:
- Unit tests for components
- Integration tests with mocks
- System tests in QEMU
- Stress tests for reliability

## Service Management

Services are managed by init:
- Automatic restart on failure
- Dependency ordering
- Resource limits
- Health monitoring

## Performance Targets

| Operation | Target | Notes |
|-----------|--------|-------|
| IPC round-trip | < 5μs | Critical path |
| Service startup | < 100ms | Including deps |
| File open | < 10μs | Cached |
| Network packet | < 20μs | Processing |

## Status

| Service | Status | Phase | Priority |
|---------|--------|-------|----------|
| Init | Planned | 2 | Critical |
| VFS | Planned | 2 | Critical |
| Device Manager | Planned | 2 | High |
| Network Stack | Planned | 3 | Medium |
| Storage | Planned | 3 | Medium |

## Development Guide

See:
- [Service Development Guide](../docs/service-guide.md)
- [IPC Protocol Design](../docs/ipc-design.md)
- [Capability Model](../docs/capabilities.md)
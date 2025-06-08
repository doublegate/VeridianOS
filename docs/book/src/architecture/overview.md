# Architecture Overview

VeridianOS is designed as a modern microkernel operating system with a focus on security, modularity, and performance. This chapter provides a comprehensive overview of the system architecture.

## Architecture Goals

- **Microkernel size**: < 15,000 lines of code
- **IPC latency**: < 1μs for small messages, < 5μs for large transfers
- **Context switch time**: < 10μs
- **Process support**: 1000+ concurrent processes
- **Memory allocation**: < 1μs latency
- **Capability lookup**: O(1) time complexity

## Core Design Principles

1. **Microkernel Architecture**: Minimal kernel with services in user space
2. **Capability-Based Security**: Unforgeable tokens for all resource access
3. **Memory Safety**: Written entirely in Rust with minimal unsafe code
4. **Zero-Copy Design**: Efficient data sharing without copying
5. **Hardware Abstraction**: Clean separation between architecture-specific and generic code
6. **Performance First**: Design decisions prioritize sub-microsecond operations

## System Layers

```
┌─────────────────────────────────────────────────────────────┐
│                    User Applications                        │
├─────────────────────────────────────────────────────────────┤
│                    System Services                          │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐       │
│  │   VFS   │  │ Network │  │ Display │  │  Audio  │       │
│  │ Service │  │  Stack  │  │ Server  │  │ Server  │       │
│  └─────────┘  └─────────┘  └─────────┘  └─────────┘       │
├─────────────────────────────────────────────────────────────┤
│                    User-Space Drivers                       │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐       │
│  │  Block  │  │   Net   │  │   GPU   │  │   USB   │       │
│  │ Drivers │  │ Drivers │  │ Drivers │  │ Drivers │       │
│  └─────────┘  └─────────┘  └─────────┘  └─────────┘       │
├─────────────────────────────────────────────────────────────┤
│                      Microkernel                            │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐       │
│  │ Memory  │  │  Task   │  │   IPC   │  │   Cap   │       │
│  │  Mgmt   │  │  Sched  │  │ System  │  │ System  │       │
│  └─────────┘  └─────────┘  └─────────┘  └─────────┘       │
└─────────────────────────────────────────────────────────────┘
```

## Microkernel Components

The microkernel contains only the essential components that must run in privileged mode:

### Memory Management
- Physical and virtual memory allocation
- Page table management
- Memory protection and isolation
- NUMA-aware allocation
- Hardware memory features (huge pages, CXL, memory tagging)

### Task Scheduling
- Process and thread management
- CPU scheduling with multi-level feedback queue
- Real-time scheduling support
- CPU affinity and NUMA optimization
- Power management integration

### Inter-Process Communication
- Synchronous message passing
- Asynchronous channels
- Shared memory regions
- Capability passing
- Zero-copy transfers

### Capability System
- Capability creation and validation
- Access control enforcement
- Hierarchical delegation
- Revocation support

## User-Space Architecture

All non-essential services run in user space for better isolation and reliability:

### System Services
- **Virtual File System**: Unified file access interface
- **Network Stack**: TCP/IP implementation with zero-copy
- **Display Server**: Wayland compositor with GPU acceleration
- **Audio Server**: Low-latency audio routing and mixing

### Device Drivers
- Run as isolated user processes
- Communicate via IPC with kernel
- Direct hardware access through capabilities
- Interrupt forwarding from kernel
- DMA buffer management

## Security Architecture

Security is built into every layer of the system:

1. **Hardware Security**: Support for Intel TDX, AMD SEV-SNP, ARM CCA
2. **Capability-Based Access**: All resources protected by capabilities
3. **Memory Safety**: Rust prevents memory corruption vulnerabilities
4. **Process Isolation**: Full address space isolation between processes
5. **Secure Boot**: Cryptographic verification of boot chain

## Performance Characteristics

VeridianOS is designed for high performance on modern hardware:

- **Lock-Free Algorithms**: Used throughout for scalability
- **Cache-Aware Design**: Data structures optimized for cache locality
- **NUMA Optimization**: Memory allocation considers NUMA topology
- **Zero-Copy IPC**: Data shared without copying
- **Fast Context Switching**: Minimal state saved/restored

## Platform Support

VeridianOS supports multiple hardware architectures:

- **x86_64**: Full support with all features
- **AArch64**: ARM 64-bit with security extensions
- **RISC-V**: RV64GC with standard extensions

Each platform has architecture-specific optimizations while sharing the majority of the codebase.

## Next Steps

- Learn about the [Microkernel Design](./microkernel.md) in detail
- Explore [Memory Management](./memory.md) architecture
- Understand the [IPC System](./ipc.md)
- Deep dive into [Capabilities](./capabilities.md)
# VeridianOS Architecture Overview

## Executive Summary

VeridianOS is a capability-based microkernel operating system designed for security, reliability, and performance. This document provides a comprehensive overview of the system architecture.

## Design Principles

1. **Minimal Kernel**: Only essential services in kernel space
2. **Capability-Based Security**: All access control via unforgeable tokens
3. **Fault Isolation**: Components isolated in separate address spaces
4. **Zero-Copy IPC**: Efficient communication without data copying
5. **Formal Verification**: Mathematical proof of critical properties

## System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        User Applications                    │
├─────────────────────────────────────────────────────────────┤
│                      System Services                        │
│  ┌─────────┐ ┌─────────┐ ┌──────────┐ ┌────────────┐        │
│  │   VFS   │ │ Network │ │ Device   │ │   Other    │        │
│  │ Service │ │  Stack  │ │ Manager  │ │  Services  │        │
│  └─────────┘ └─────────┘ └──────────┘ └────────────┘        │
├─────────────────────────────────────────────────────────────┤
│                      Device Drivers                         │
│  ┌─────────┐ ┌─────────┐ ┌──────────┐ ┌────────────┐        │
│  │ Storage │ │ Network │ │  Input   │ │   Other    │        │
│  │ Drivers │ │ Drivers │ │ Drivers  │ │  Drivers   │        │
│  └─────────┘ └─────────┘ └──────────┘ └────────────┘        │
├─────────────────────────────────────────────────────────────┤
│                    VeridianOS Microkernel                   │
│  ┌─────────┐ ┌─────────┐ ┌──────────┐ ┌────────────┐        │
│  │ Memory  │ │  IPC    │ │Scheduler │ │Capability  │        │
│  │  Mgmt   │ │ System  │ │          │ │  System    │        │
│  └─────────┘ └─────────┘ └──────────┘ └────────────┘        │
├─────────────────────────────────────────────────────────────┤
│                      Hardware (x86_64, AArch64, RISC-V)     │
└─────────────────────────────────────────────────────────────┘
```

## Kernel Components

### Memory Management (~95% Complete)
- **Frame Allocator**: Hybrid bitmap/buddy allocator ✅
  - Bitmap for allocations <512 frames
  - Buddy system for larger allocations
  - NUMA-aware with per-node allocators
- **Virtual Memory**: 4-level page table management ✅
  - Automatic intermediate table creation
  - Support for 2MB and 1GB huge pages
  - Full address space management with mmap
- **TLB Management**: Multi-core shootdown support ✅
  - Per-CPU TLB flush operations
  - Architecture-specific implementations
  - <5μs per CPU shootdown latency
- **Kernel Heap**: Slab allocator implementation ✅
  - Cache-friendly allocation for common sizes
  - Global allocator for Rust alloc support
  - <500ns allocation latency
- **Memory Zones**: Zone-aware allocation ✅
  - DMA zone (0-16MB) for legacy devices
  - Normal zone for regular allocations
  - Zone balancing and fallback
- **Shared Memory**: Zero-copy IPC implementation
- **NUMA Support**: Topology-aware allocation ✅

### Process Management
- **Process Model**: Lightweight threads with separate address spaces
- **Scheduling**: O(1) scheduler with priority levels
- **Context Switching**: < 10μs target latency

### Inter-Process Communication (IPC)
- **Synchronous IPC**: Rendezvous-style message passing
- **Asynchronous IPC**: Channel-based communication
- **Shared Memory**: Capability-protected regions
- **Performance**: < 5μs for small messages

### Capability System
- **Token Structure**: 64-bit unforgeable tokens
- **Access Control**: All resources require capabilities
- **Delegation**: Controlled capability sharing
- **Revocation**: Immediate capability invalidation

## Memory Layout

### Virtual Address Space (x86_64)
```
0x0000_0000_0000_0000 - 0x0000_7FFF_FFFF_FFFF  User Space (128 TB)
0x0000_8000_0000_0000 - 0xFFFF_7FFF_FFFF_FFFF  Non-canonical (hole)
0xFFFF_8000_0000_0000 - 0xFFFF_FFFF_FFFF_FFFF  Kernel Space (128 TB)
```

### Physical Memory Map
```
0x0000_0000 - 0x0009_FFFF  Real Mode (640 KB)
0x000A_0000 - 0x000F_FFFF  Device Memory (384 KB)
0x0010_0000 - 0xXXXX_XXXX  Available RAM
0xFEC0_0000 - 0xFFFF_FFFF  Memory-mapped I/O
```

## Security Architecture

### Capability Model
- **Object Capabilities**: Fine-grained access control
- **Hierarchical Delegation**: Parent controls child capabilities
- **No Ambient Authority**: Explicit capability for all access

### Hardware Security Features
- **Intel TDX**: Confidential computing support
- **AMD SEV-SNP**: Encrypted virtualization
- **ARM CCA**: Realm management
- **RISC-V PMP**: Physical memory protection

### Threat Model
- **Malicious Drivers**: Isolated in user space
- **Compromised Services**: Limited by capabilities
- **Hardware Attacks**: Mitigated by security features

## Performance Architecture

### Design for Performance
- **Cache-Aware**: Data structure alignment
- **NUMA-Aware**: Local memory allocation
- **Lock-Free**: Where possible
- **Zero-Copy**: IPC and I/O paths

### Performance Targets
- **Context Switch**: < 10μs (pending scheduler)
- **System Call**: < 500ns
- **IPC Latency**: < 5μs (✅ achieving <1μs for small messages)
- **Memory Allocation**: < 1μs (✅ achieving ~500ns)
- **Page Mapping**: < 2μs (✅ achieving 1.5μs)
- **TLB Shootdown**: < 5μs/CPU (✅ achieving 4.2μs)
- **Heap Allocation**: < 500ns (✅ achieving 350ns)

## Platform Support

### x86_64
- **Boot**: UEFI or Legacy BIOS
- **Features**: AVX, TSX, CET support
- **Virtualization**: Full nested virtualization

### AArch64
- **Boot**: UEFI with device tree
- **Features**: SVE, PAC, BTI support
- **Virtualization**: KVM compatibility

### RISC-V
- **Boot**: OpenSBI firmware
- **Features**: RV64GC base ISA
- **Extensions**: Hypervisor, vector

## Development Phases

### Phase 0: Foundation (✅ Complete)
- Development environment ✅
- Build system ✅
- Basic boot ✅

### Phase 1: Microkernel Core (🔄 ~35% Complete)
- Memory management (~95% complete)
- Process management (0% - starting)
- Basic IPC (~45% complete)
- Capability system (0% - design phase)

### Phase 2: User Space Foundation
- Init process
- Basic drivers
- VFS service

### Phase 3: Security Hardening
- SELinux policies
- Secure boot
- Attestation

### Phase 4: Package Management
- Ports system
- Binary packages
- Updates

### Phase 5: Performance
- Optimization
- Profiling
- Tuning

### Phase 6: Desktop
- GUI framework
- Wayland
- Applications

## Comparison with Other Systems

### vs. Linux
- **Microkernel**: Better isolation
- **Capabilities**: Finer access control
- **User-space drivers**: Better reliability

### vs. seL4
- **Rust**: Memory safety by default
- **Pragmatic**: Balance of verification and features
- **Modern**: Designed for current hardware

### vs. Fuchsia
- **Simpler**: Less complexity
- **POSIX**: Compatibility layer
- **Open**: Community-driven

## Future Directions

### Research Areas
- Formal verification expansion
- Hardware capability support
- Persistent memory integration
- Quantum-resistant cryptography

### Ecosystem Development
- Language runtimes
- Container support
- Cloud integration
- Edge computing

## References

- [Capability Security](docs/capabilities.md)
- [IPC Design](docs/design/IPC-DESIGN.md)
- [Memory Allocator](docs/design/MEMORY-ALLOCATOR-DESIGN.md)
- [Scheduler Design](docs/design/SCHEDULER-DESIGN.md)

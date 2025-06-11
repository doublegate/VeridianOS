# VeridianOS Capability System Design Document

**Version**: 1.1  
**Date**: 2025-06-11  
**Status**: Partially Implemented (~45% Complete)

## Executive Summary

This document defines the capability-based security architecture for VeridianOS, providing unforgeable tokens for all resource access. The design emphasizes O(1) lookup performance, hierarchical delegation, and integration with hardware security features.

## Design Goals

### Security Goals
- **Unforgeable**: Capabilities cannot be guessed or crafted
- **Mandatory**: All resource access requires capabilities
- **Delegatable**: Controlled sharing between processes
- **Revocable**: Support for immediate revocation
- **Auditable**: Complete access control trail

### Performance Goals
- **Lookup**: O(1) average case, O(log n) worst case
- **Validation**: < 100ns per check
- **Creation**: < 500ns
- **Delegation**: < 1μs
- **Memory Overhead**: < 1KB per process

## Capability Model

### Capability Structure
```rust
/// 64-bit capability token
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Capability {
    /// Unique capability ID (48 bits)
    id: u64,
    /// Generation counter (8 bits) 
    generation: u8,
    /// Capability type (4 bits)
    cap_type: CapType,
    /// Flags (4 bits)
    flags: CapFlags,
}

impl Capability {
    /// Pack into 64-bit value
    pub fn to_u64(self) -> u64 {
        (self.id & 0xFFFF_FFFF_FFFF) |
        ((self.generation as u64) << 48) |
        ((self.cap_type as u64) << 56) |
        ((self.flags.bits() as u64) << 60)
    }
    
    /// Unpack from 64-bit value
    pub fn from_u64(value: u64) -> Self {
        Self {
            id: value & 0xFFFF_FFFF_FFFF,
            generation: ((value >> 48) & 0xFF) as u8,
            cap_type: CapType::from_u8(((value >> 56) & 0xF) as u8),
            flags: CapFlags::from_bits_truncate(((value >> 60) & 0xF) as u8),
        }
    }
}
```

### Capability Types
```rust
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CapType {
    /// Memory region access
    Memory = 0,
    /// Thread/process control
    Thread = 1,
    /// IPC endpoint
    Endpoint = 2,
    /// Interrupt handling
    Interrupt = 3,
    /// I/O port access
    IoPort = 4,
    /// File/device access
    Handle = 5,
    /// Scheduling control
    Scheduler = 6,
    /// Page table manipulation
    PageTable = 7,
    /// Capability space manipulation
    CapSpace = 8,
    /// Hardware device access
    Device = 9,
}

bitflags! {
    pub struct CapFlags: u8 {
        /// Can read from resource
        const READ = 0b0001;
        /// Can write to resource
        const WRITE = 0b0010;
        /// Can execute (memory) or invoke (endpoint)
        const EXECUTE = 0b0100;
        /// Can delegate to other processes
        const GRANT = 0b1000;
    }
}
```

## Capability Space

### Per-Process Capability Table
```rust
pub struct CapabilitySpace {
    /// Fast lookup table (L1)
    l1_table: Box<[Option<CapEntry>; L1_SIZE]>, // 256 entries
    /// Second level tables (L2)
    l2_tables: HashMap<u16, Box<[Option<CapEntry>; L2_SIZE]>>, // 256 entries each
    /// Generation counter for revocation
    generation: AtomicU8,
    /// Statistics
    stats: CapSpaceStats,
}

pub struct CapEntry {
    /// The capability token
    capability: Capability,
    /// Object reference
    object: ObjectRef,
    /// Access rights
    rights: Rights,
    /// Usage count
    usage_count: AtomicU64,
}

impl CapabilitySpace {
    /// O(1) lookup in common case
    pub fn lookup(&self, cap: Capability) -> Option<&CapEntry> {
        let index = cap.id as usize;
        
        // Fast path: check L1 table
        if index < L1_SIZE {
            return self.l1_table[index].as_ref()
                .filter(|entry| entry.capability == cap);
        }
        
        // Slow path: check L2 table
        let l1_index = (index >> 8) as u16;
        let l2_index = (index & 0xFF) as usize;
        
        self.l2_tables.get(&l1_index)
            .and_then(|table| table[l2_index].as_ref())
            .filter(|entry| entry.capability == cap)
    }
}
```

### Object References
```rust
/// References to kernel objects
#[derive(Clone)]
pub enum ObjectRef {
    /// Physical memory region
    Memory {
        base: PhysAddr,
        size: usize,
        attributes: MemoryAttributes,
    },
    /// Thread control block
    Thread {
        tcb: Arc<Mutex<ThreadControlBlock>>,
    },
    /// IPC endpoint
    Endpoint {
        endpoint: Arc<IpcEndpoint>,
    },
    /// Hardware device
    Device {
        device: Arc<dyn Device>,
    },
    /// Page table
    PageTable {
        root: PhysAddr,
        asid: u16,
    },
}
```

## Capability Operations

### Creation
```rust
pub struct CapabilityManager {
    /// Global capability registry
    registry: RwLock<CapabilityRegistry>,
    /// ID allocator
    id_allocator: IdAllocator,
    /// Revocation list
    revoked: RwLock<HashSet<u64>>,
}

impl CapabilityManager {
    pub fn create_capability(
        &self,
        object: ObjectRef,
        rights: Rights,
        cap_type: CapType,
    ) -> Result<Capability, CapError> {
        // Allocate unique ID
        let id = self.id_allocator.allocate()?;
        
        // Create capability
        let cap = Capability {
            id,
            generation: 0,
            cap_type,
            flags: rights_to_flags(rights),
        };
        
        // Register in global registry
        self.registry.write().insert(cap, object.clone());
        
        Ok(cap)
    }
}
```

### Delegation
```rust
impl CapabilitySpace {
    pub fn delegate(
        &mut self,
        cap: Capability,
        target: &mut CapabilitySpace,
        new_rights: Rights,
    ) -> Result<Capability, CapError> {
        // Verify source capability
        let entry = self.lookup(cap)
            .ok_or(CapError::InvalidCapability)?;
        
        // Check grant permission
        if !entry.capability.flags.contains(CapFlags::GRANT) {
            return Err(CapError::PermissionDenied);
        }
        
        // Ensure new rights are subset
        let derived_rights = entry.rights.intersection(new_rights);
        
        // Create derived capability
        let new_cap = Capability {
            id: entry.capability.id,
            generation: entry.capability.generation,
            cap_type: entry.capability.cap_type,
            flags: rights_to_flags(derived_rights),
        };
        
        // Insert into target space
        target.insert(new_cap, entry.object.clone(), derived_rights)?;
        
        Ok(new_cap)
    }
}
```

### Revocation
```rust
impl CapabilityManager {
    /// Revoke a capability globally
    pub fn revoke(&self, cap: Capability) -> Result<(), CapError> {
        // Add to revocation list
        self.revoked.write().insert(cap.to_u64());
        
        // Increment generation counter
        self.registry.write().increment_generation(cap.id);
        
        // Notify all capability spaces
        self.broadcast_revocation(cap);
        
        Ok(())
    }
    
    /// Fast revocation check
    #[inline]
    pub fn is_revoked(&self, cap: Capability) -> bool {
        self.revoked.read().contains(&cap.to_u64())
    }
}
```

## Hardware Integration

### Intel TDX Integration
```rust
#[cfg(feature = "tdx")]
pub struct TdxCapability {
    /// TDX-sealed capability
    sealed_cap: SealedData,
    /// Measurement for attestation
    measurement: Measurement,
}

impl TdxCapability {
    pub fn seal(cap: Capability) -> Result<Self, TdxError> {
        let sealed = tdx::seal_data(&cap.to_le_bytes())?;
        let measurement = tdx::get_measurement()?;
        
        Ok(Self {
            sealed_cap: sealed,
            measurement,
        })
    }
}
```

### ARM Pointer Authentication
```rust
#[cfg(target_arch = "aarch64")]
impl Capability {
    /// Sign capability with PAC
    pub fn sign(self) -> SignedCapability {
        let value = self.to_u64();
        let signed = unsafe {
            core::arch::aarch64::__builtin_arm_pacia(
                value as *const (),
                0, // Context
            ) as u64
        };
        
        SignedCapability(signed)
    }
}
```

## Access Control

### Capability Checks
```rust
/// Fast inline capability check
#[inline(always)]
pub fn check_capability(
    cap: Capability,
    required_rights: Rights,
) -> Result<(), CapError> {
    // Get current process capability space
    let cap_space = current_process().cap_space();
    
    // Lookup capability
    let entry = cap_space.lookup(cap)
        .ok_or(CapError::InvalidCapability)?;
    
    // Check rights
    if !entry.rights.contains(required_rights) {
        return Err(CapError::InsufficientRights);
    }
    
    // Update usage statistics
    entry.usage_count.fetch_add(1, Ordering::Relaxed);
    
    Ok(())
}

/// Capability check macro for system calls
#[macro_export]
macro_rules! require_capability {
    ($cap:expr, $rights:expr) => {
        check_capability($cap, $rights)?
    };
}
```

### Memory Capabilities
```rust
impl MemoryCapability {
    pub fn check_access(
        &self,
        addr: VirtAddr,
        size: usize,
        access: Access,
    ) -> Result<(), CapError> {
        // Verify address range
        if addr < self.base || addr + size > self.base + self.size {
            return Err(CapError::OutOfBounds);
        }
        
        // Check permissions
        match access {
            Access::Read => require!(self.rights.contains(Rights::READ)),
            Access::Write => require!(self.rights.contains(Rights::WRITE)),
            Access::Execute => require!(self.rights.contains(Rights::EXECUTE)),
        }
        
        Ok(())
    }
}
```

## Capability Caching

### Per-CPU Capability Cache
```rust
pub struct CapabilityCache {
    /// Recently used capabilities
    cache: [Option<CachedCap>; CACHE_SIZE],
    /// Cache statistics
    hits: AtomicU64,
    misses: AtomicU64,
}

#[repr(align(64))] // Cache line aligned
pub struct CachedCap {
    capability: Capability,
    object_ptr: *const (),
    rights: Rights,
    last_used: Instant,
}

impl CapabilityCache {
    #[inline]
    pub fn lookup(&self, cap: Capability) -> Option<&CachedCap> {
        let hash = cap.id as usize % CACHE_SIZE;
        
        self.cache[hash].as_ref()
            .filter(|cached| cached.capability == cap)
            .map(|cached| {
                self.hits.fetch_add(1, Ordering::Relaxed);
                cached
            })
            .or_else(|| {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            })
    }
}
```

## System Call Interface

### Capability System Calls
```rust
/// Capability-related system calls
pub enum CapSyscall {
    /// Create a new capability
    Create {
        object_type: ObjectType,
        params: CreateParams,
    },
    /// Delegate capability to another process
    Delegate {
        cap: Capability,
        target_pid: ProcessId,
        new_rights: Rights,
    },
    /// Revoke a capability
    Revoke {
        cap: Capability,
    },
    /// Query capability information
    Identify {
        cap: Capability,
    },
}

#[syscall]
pub fn sys_capability(op: CapSyscall) -> Result<SyscallResult, SyscallError> {
    match op {
        CapSyscall::Create { object_type, params } => {
            let cap = cap_manager().create_capability(object_type, params)?;
            Ok(SyscallResult::Capability(cap))
        }
        CapSyscall::Delegate { cap, target_pid, new_rights } => {
            let target = process_table().get(target_pid)?;
            current_process().cap_space().delegate(cap, target.cap_space(), new_rights)?;
            Ok(SyscallResult::Success)
        }
        // ... other operations
    }
}
```

## Security Properties

### Confinement
- Processes start with minimal capabilities
- Parent controls child's initial capabilities
- No ambient authority

### Revocation Safety
- Generation counters prevent use-after-revoke
- Atomic revocation across system
- No dangling references

### Information Flow
- Capability possession implies authorization
- No covert channels through capability system
- Audit trail for all capability operations

## Performance Optimizations

### Fast Path Design
1. L1 capability cache hit: ~10 cycles
2. L1 capability table hit: ~20 cycles
3. L2 capability table hit: ~50 cycles
4. Full validation: ~100 cycles

### Memory Layout
- Cache-line aligned structures
- Hot/cold data separation
- Per-CPU caches to avoid contention

### Batch Operations
```rust
pub fn check_capabilities_batch(
    caps: &[Capability],
    rights: Rights,
) -> Result<(), CapError> {
    // Prefetch capability entries
    for cap in caps {
        prefetch_capability(*cap);
    }
    
    // Check all capabilities
    for cap in caps {
        check_capability(*cap, rights)?;
    }
    
    Ok(())
}
```

## Testing Strategy

### Security Tests
- Capability forging attempts
- Revocation race conditions
- Delegation chains
- Confinement verification

### Performance Tests
- Lookup latency distribution
- Cache hit rates
- Concurrent access scalability
- Revocation performance

### Stress Tests
- Maximum capabilities per process
- Rapid creation/deletion
- Deep delegation chains
- Revocation storms

## Future Enhancements

### Phase 3 (Security Hardening)
- Encrypted capabilities
- Remote attestation
- Distributed capabilities
- Capability persistence

### Phase 5 (Performance)
- Hardware capability support
- SIMD batch validation
- Speculative capability checks
- Machine learning for cache prediction

## Implementation Status (June 11, 2025)

### Completed Components (~45%)
- ✅ **Capability Token Structure**: 64-bit packed tokens implemented
- ✅ **Capability Space**: Two-level table structure with O(1) lookup
- ✅ **Rights Management**: Full rights system with grant/derive/delegate
- ✅ **Object References**: Support for Memory, Process, Thread, Endpoint
- ✅ **Basic Operations**: Create, validate, lookup, basic revoke
- ✅ **IPC Integration**: Complete capability validation for all IPC operations
- ✅ **Memory Integration**: Capability checks for memory operations
- ✅ **System Call Enforcement**: All capability-related syscalls validate

### In Progress
- 🔶 **Capability Inheritance**: Fork/exec inheritance policies
- 🔶 **Cascading Revocation**: Revocation tree tracking
- 🔶 **Per-CPU Cache**: Performance optimization

### Not Started
- ❌ **Encrypted Capabilities**: Phase 3 enhancement
- ❌ **Hardware Integration**: Phase 5 optimization
- ❌ **Distributed Capabilities**: Future enhancement

### Recent Changes (June 11, 2025)
- Added full IPC-Capability integration
- Implemented capability transfer through IPC messages
- Added send/receive permission validation
- Integrated with system call handlers
- Added Rights::difference() method for delegation

---

*This document will be updated based on security analysis and implementation experience.*
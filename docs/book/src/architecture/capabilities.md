# Capability System

**Implementation Status**: ~45% Complete (as of June 11, 2025)

VeridianOS uses a capability-based security model where all resource access is mediated through unforgeable capability tokens. This provides fine-grained access control without the complexity of traditional access control lists.

## Design Principles

### Capability Properties
1. **Unforgeable**: Cannot be created by user code
2. **Transferable**: Can be passed between processes
3. **Restrictable**: Can derive weaker capabilities
4. **Revocable**: Can be invalidated recursively

### No Ambient Authority
Unlike traditional Unix systems, processes have no implicit permissions. Every resource access requires an explicit capability.

## Capability Structure

```rust
pub struct Capability {
    // Object type (16 bits)
    cap_type: CapabilityType,
    
    // Unique object identifier (32 bits)
    object_id: ObjectId,
    
    // Access rights bitmap (16 bits)
    rights: Rights,
    
    // Generation counter (16 bits)
    generation: u16,
}

pub enum CapabilityType {
    Process = 0x0001,
    Thread = 0x0002,
    Memory = 0x0003,
    Port = 0x0004,
    Interrupt = 0x0005,
    Device = 0x0006,
    File = 0x0007,
    // ... more types
}

bitflags! {
    pub struct Rights: u16 {
        const READ = 0x0001;
        const WRITE = 0x0002;
        const EXECUTE = 0x0004;
        const DELETE = 0x0008;
        const GRANT = 0x0010;
        const REVOKE = 0x0020;
        // ... more rights
    }
}
```

## Capability Operations

### Creation
Only the kernel can create new capabilities:

```rust
// Kernel API
pub fn create_capability(
    object: &KernelObject,
    rights: Rights,
) -> Capability {
    Capability {
        cap_type: object.capability_type(),
        object_id: object.id(),
        rights,
        generation: object.generation(),
    }
}
```

### Derivation
Create a weaker capability from an existing one:

```rust
// User API via system call
pub fn derive_capability(
    parent: &Capability,
    new_rights: Rights,
) -> Result<Capability, CapError> {
    // New rights must be subset of parent rights
    if !parent.rights.contains(new_rights) {
        return Err(CapError::InsufficientRights);
    }
    
    // Must have GRANT right to derive
    if !parent.rights.contains(Rights::GRANT) {
        return Err(CapError::NoGrantRight);
    }
    
    Ok(Capability {
        rights: new_rights,
        ..*parent
    })
}
```

### Validation
O(1) capability validation using hash tables:

```rust
pub struct CapabilityTable {
    // Hash table for O(1) lookup
    table: HashMap<ObjectId, CapabilityEntry>,
    
    // LRU cache for hot capabilities
    cache: LruCache<Capability, bool>,
}

impl CapabilityTable {
    pub fn validate(&self, cap: &Capability) -> bool {
        // Check cache first
        if let Some(&valid) = self.cache.get(cap) {
            return valid;
        }
        
        // Lookup in main table
        if let Some(entry) = self.table.get(&cap.object_id) {
            let valid = entry.generation == cap.generation
                && entry.valid
                && entry.rights.contains(cap.rights);
            
            // Update cache
            self.cache.put(*cap, valid);
            valid
        } else {
            false
        }
    }
}
```

## Capability Passing

### IPC Integration
Capabilities can be passed through IPC:

```rust
pub struct IpcMessage {
    // Message data
    data: Vec<u8>,
    
    // Attached capabilities (max 4)
    capabilities: ArrayVec<Capability, 4>,
}

// Send capability to another process
process.send_message(IpcMessage {
    data: b"Here's access to the file".to_vec(),
    capabilities: vec![file_capability].into(),
})?;
```

### Capability Delegation
Parent process can delegate capabilities to children:

```rust
// Create child process with specific capabilities
let child = Process::spawn(
    "child_program",
    &[
        memory_capability,
        network_capability.derive(Rights::READ)?, // Read-only network
    ],
)?;
```

## Revocation

### Recursive Revocation
When a capability is revoked, all derived capabilities are also invalidated:

```rust
pub struct RevocationTree {
    // Parent -> Children mapping
    children: HashMap<Capability, Vec<Capability>>,
}

impl RevocationTree {
    pub fn revoke(&mut self, cap: &Capability) {
        // Mark capability as invalid
        self.invalidate(cap);
        
        // Recursively revoke all children
        if let Some(children) = self.children.get(cap) {
            for child in children.clone() {
                self.revoke(&child);
            }
        }
    }
}
```

### Generation Counters
Prevent capability reuse after revocation:

```rust
impl KernelObject {
    pub fn revoke_all_capabilities(&mut self) {
        // Increment generation, invalidating all existing capabilities
        self.generation = self.generation.wrapping_add(1);
    }
}
```

## Performance Optimizations

### Fast Path Validation
Common capabilities use optimized validation:

```rust
// Fast path for common operations
#[inline(always)]
pub fn validate_memory_read(cap: &Capability, addr: VirtAddr) -> bool {
    cap.cap_type == CapabilityType::Memory
        && cap.rights.contains(Rights::READ)
        && addr_in_range(cap, addr)
}
```

### Capability Caching
Hot capabilities are cached per-CPU:

```rust
pub struct PerCpuCapCache {
    // Recently validated capabilities
    recent: ArrayVec<(Capability, Instant), 16>,
}

// Check cache before full validation
if cpu_cache.contains(cap) && !expired(cap) {
    return Ok(());
}
```

## Security Properties

### Confinement
Processes can only access resources they have capabilities for:
- No ambient authority
- No privilege escalation
- Complete mediation

### Principle of Least Privilege
Easy to grant minimal required permissions:
```rust
// Grant only read access to specific memory region
let read_only = memory_cap.derive(Rights::READ)?;
untrusted_process.grant(read_only);
```

### Accountability
All capability operations are logged:
```rust
pub struct CapabilityAudit {
    timestamp: Instant,
    operation: CapOperation,
    subject: ProcessId,
    capability: Capability,
    result: Result<(), CapError>,
}
```

## Common Patterns

### Capability Bundles
Group related capabilities:

```rust
pub struct FileBundle {
    read: Capability,
    write: Capability,
    metadata: Capability,
}
```

### Temporary Delegation
Grant temporary access:

```rust
// Grant capability that expires
let temp_cap = capability.with_expiration(
    Instant::now() + Duration::from_secs(3600)
);
```

### Capability Stores
Persistent capability storage:

```rust
pub trait CapabilityStore {
    fn save(&mut self, name: &str, cap: Capability);
    fn load(&self, name: &str) -> Option<Capability>;
    fn list(&self) -> Vec<String>;
}
```

## Best Practices

1. **Minimize Capability Rights**: Only grant necessary permissions
2. **Use Derivation**: Create restricted capabilities from broader ones
3. **Audit Capability Usage**: Log all capability operations
4. **Implement Revocation**: Plan for capability invalidation
5. **Cache Validations**: Optimize hot-path capability checks

## Implementation Status (June 11, 2025)

### Completed Features (~45% Complete)

- **Capability Tokens**: 64-bit packed tokens with ID, generation, type, and flags
- **Capability Spaces**: Two-level table structure (L1/L2) with O(1) lookup
- **Rights Management**: Complete rights system (Read, Write, Execute, Grant, Derive, Manage)
- **Object References**: Support for Memory, Process, Thread, Endpoint, and more
- **Basic Operations**: Create, lookup, validate, and basic revoke
- **IPC Integration**: Full capability validation for all IPC operations
- **Memory Integration**: Capability checks for memory operations
- **System Call Enforcement**: All capability-related syscalls validate permissions

### Recent Achievements (June 11, 2025)

- **IPC-Capability Integration**: Complete integration with IPC subsystem
- **Capability Transfer**: Implemented secure capability passing through IPC
- **Permission Enforcement**: All IPC operations validate send/receive rights
- **Shared Memory Validation**: Memory sharing respects capability permissions

### In Progress

- **Capability Inheritance**: Fork/exec inheritance policies (design complete, implementation pending)
- **Cascading Revocation**: Revocation tree tracking (basic revoke done, cascading pending)
- **Per-CPU Cache**: Performance optimization for capability lookups

### Not Yet Started

- **Process Table Integration**: Needed for broadcast revocation
- **Audit Logging**: Comprehensive audit trail
- **Persistence**: Capability storage across reboots
- **Hardware Integration**: Future hardware capability support

The capability system provides the security foundation for VeridianOS, ensuring that all resource access is properly authorized and auditable.

//! Capability inheritance for process creation
//!
//! Implements how capabilities are inherited when creating new processes.

use super::{
    manager::cap_manager,
    space::{CapabilityEntry, CapabilitySpace},
    token::{CapabilityToken, Rights},
};
use crate::{error::KernelError, process::ProcessId};

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Inheritance policy for capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InheritancePolicy {
    /// No capabilities inherited
    None,
    /// All capabilities inherited with same rights
    All,
    /// Only capabilities marked as inheritable
    Inheritable,
    /// Inherit with reduced rights
    Reduced,
    /// Custom policy with filter function
    Custom,
}

/// Default inheritance policy
pub const DEFAULT_INHERITANCE_POLICY: InheritancePolicy = InheritancePolicy::Inheritable;

/// Capability inheritance flags
pub struct InheritanceFlags;

impl InheritanceFlags {
    /// Capability can be inherited by child processes
    pub const INHERITABLE: u32 = 1 << 0;
    /// Capability is preserved across exec
    pub const PRESERVE_EXEC: u32 = 1 << 1;
    /// Capability rights are reduced on inheritance
    pub const REDUCE_RIGHTS: u32 = 1 << 2;
    /// Capability is inherited but starts disabled
    pub const START_DISABLED: u32 = 1 << 3;
}

/// Result of capability inheritance operation
#[derive(Debug)]
pub enum InheritanceResult {
    /// Successfully inherited capabilities
    Success { inherited: usize, skipped: usize },
    /// Partial success - some capabilities couldn't be inherited
    Partial {
        inherited: usize,
        failed: usize,
        #[cfg(feature = "alloc")]
        errors: Vec<(&'static str, CapabilityToken)>,
    },
    /// Complete failure
    Failed(&'static str),
}

/// Copy one entry into `child` with `rights`, keeping its inheritance
/// flags. Returns whether it was inserted.
fn inherit_entry(child_space: &CapabilitySpace, entry: &CapabilityEntry, rights: Rights) -> bool {
    let mut copy = CapabilityEntry::new(entry.capability, entry.object.clone(), rights);
    copy.inheritance_flags = entry.inheritance_flags;
    child_space.insert_entry(copy).is_ok()
}

/// Inherit capabilities from parent to child process
///
/// Every entry of the parent is considered, whatever its ID. (The old
/// two-level table walk visited IDs below 256 and then "the L2 tables";
/// with globally allocated IDs the policies diverged between the two.)
pub fn inherit_capabilities(
    parent_space: &CapabilitySpace,
    child_space: &CapabilitySpace,
    policy: InheritancePolicy,
) -> Result<u32, KernelError> {
    if policy == InheritancePolicy::None {
        return Ok(0);
    }
    let mut inherited_count = 0;
    // A snapshot: the parent's lock is not held while the child is filled.
    for entry in parent_space.entries() {
        if !should_inherit(entry.capability, entry.inheritance_flags, policy) {
            continue;
        }
        let rights = if policy == InheritancePolicy::Reduced {
            reduce_rights_for_inheritance(entry.rights)
        } else {
            entry.rights
        };
        if inherit_entry(child_space, &entry, rights) {
            inherited_count += 1;
        }
    }
    Ok(inherited_count)
}

/// Fork inheritance - copy all capabilities to child
pub fn fork_inherit_capabilities(
    parent_space: &CapabilitySpace,
    child_space: &CapabilitySpace,
) -> Result<(), KernelError> {
    // In fork, child gets exact copy of parent's capabilities
    inherit_capabilities(parent_space, child_space, InheritancePolicy::All)?;
    Ok(())
}

/// Exec inheritance - filter capabilities by PRESERVE_EXEC flag
///
/// Only capabilities marked with PRESERVE_EXEC survive an exec call.
/// This prevents privilege escalation across exec boundaries.
pub fn exec_inherit_capabilities(
    old_space: &CapabilitySpace,
    new_space: &CapabilitySpace,
) -> Result<(), KernelError> {
    // Stricter than the Inheritable policy. Every entry is checked; this
    // used to scan only IDs 0..256 and silently dropped the rest.
    for entry in old_space.entries() {
        if (entry.inheritance_flags & InheritanceFlags::PRESERVE_EXEC) == 0 {
            continue;
        }
        let rights = if (entry.inheritance_flags & InheritanceFlags::REDUCE_RIGHTS) != 0 {
            reduce_rights_for_inheritance(entry.rights)
        } else {
            entry.rights
        };
        if !inherit_entry(new_space, &entry, rights) {
            crate::println!(
                "[CAP] Warning: failed to inherit capability {:#x} during exec",
                entry.capability.id()
            );
        }
    }
    Ok(())
}

/// Create initial capabilities for a new process
pub fn create_initial_capabilities(
    process_id: ProcessId,
    cap_space: &CapabilitySpace,
) -> Result<(), KernelError> {
    // Create basic capabilities that every process needs

    // 1. Capability to access its own process info
    let process_obj = super::object::ObjectRef::Process { pid: process_id };
    let process_rights = Rights::READ | Rights::MODIFY;
    cap_manager()
        .create_capability(process_obj, process_rights, cap_space)
        .map_err(|_| KernelError::InvalidCapability {
            cap_id: 0,
            reason: crate::error::CapError::InvalidObject,
        })?;

    // Create default IPC endpoint capability for the process to communicate
    #[cfg(feature = "alloc")]
    {
        use alloc::sync::Arc;
        let endpoint = Arc::new(crate::ipc::channel::Endpoint::new(process_id));
        let ipc_obj = super::object::ObjectRef::Endpoint { endpoint };
        let ipc_rights = Rights::READ | Rights::WRITE | Rights::EXECUTE;
        cap_manager()
            .create_capability(ipc_obj, ipc_rights, cap_space)
            .map_err(|_| KernelError::InvalidCapability {
                cap_id: 0,
                reason: crate::error::CapError::InvalidObject,
            })?;
    }

    // Create memory capabilities for stack and heap regions
    {
        use super::object::MemoryAttributes;

        // Stack capability (read + write, no execute for W^X)
        let stack_obj = super::object::ObjectRef::Memory {
            base: 0,        // Actual address assigned during process setup
            size: 0x100000, // 1MB default stack
            attributes: MemoryAttributes::normal(),
        };
        let stack_rights = Rights::READ | Rights::WRITE;
        cap_manager()
            .create_capability(stack_obj, stack_rights, cap_space)
            .map_err(|_| KernelError::InvalidCapability {
                cap_id: 0,
                reason: crate::error::CapError::InvalidObject,
            })?;

        // Heap capability (read + write, no execute for W^X)
        let heap_obj = super::object::ObjectRef::Memory {
            base: 0,         // Actual address assigned during process setup
            size: 0x1000000, // 16MB default heap
            attributes: MemoryAttributes::normal(),
        };
        let heap_rights = Rights::READ | Rights::WRITE;
        cap_manager()
            .create_capability(heap_obj, heap_rights, cap_space)
            .map_err(|_| KernelError::InvalidCapability {
                cap_id: 0,
                reason: crate::error::CapError::InvalidObject,
            })?;
    }

    Ok(())
}

/// Rights reduction for inheritance
pub fn reduce_rights_for_inheritance(original: Rights) -> Rights {
    // Remove dangerous rights
    original.remove(Rights::GRANT).remove(Rights::REVOKE)
}

/// Check if a capability should be inherited
pub fn should_inherit(_cap: CapabilityToken, flags: u32, policy: InheritancePolicy) -> bool {
    match policy {
        InheritancePolicy::None => false,
        InheritancePolicy::All => true,
        InheritancePolicy::Inheritable => (flags & InheritanceFlags::INHERITABLE) != 0,
        InheritancePolicy::Reduced => true,
        InheritancePolicy::Custom => {
            // Default custom policy: inherit if marked
            (flags & InheritanceFlags::INHERITABLE) != 0
        }
    }
}

/// Process capability inheritance for system calls
pub fn inherit_for_syscall(
    syscall: &str,
    parent_space: &CapabilitySpace,
    child_space: &CapabilitySpace,
) -> Result<(), KernelError> {
    match syscall {
        "fork" => fork_inherit_capabilities(parent_space, child_space),
        "exec" => exec_inherit_capabilities(parent_space, child_space),
        "spawn" => {
            // New process with limited inheritance
            inherit_capabilities(parent_space, child_space, InheritancePolicy::Inheritable)
                .map(|_| ())
        }
        _ => Err(KernelError::InvalidArgument {
            name: "syscall",
            value: "unknown syscall for capability inheritance",
        }),
    }
}

// Helper functions

/// Delegate a capability to another process
pub fn delegate_capability(
    source_space: &CapabilitySpace,
    target_space: &CapabilitySpace,
    cap: CapabilityToken,
    new_rights: Option<Rights>,
) -> Result<CapabilityToken, KernelError> {
    // Lookup capability in source space
    let (object, source_rights) =
        source_space
            .lookup_entry(cap)
            .ok_or(KernelError::InvalidCapability {
                cap_id: cap.id(),
                reason: crate::error::CapError::NotFound,
            })?;

    // Check if source has grant right
    if !source_rights.contains(Rights::GRANT) {
        return Err(KernelError::PermissionDenied {
            operation: "delegate_capability",
        });
    }

    // Determine rights for target
    let target_rights = if let Some(requested) = new_rights {
        // Can only grant rights that source has
        if !source_rights.contains(requested) {
            return Err(KernelError::InsufficientRights {
                required: requested.bits(),
                actual: source_rights.bits(),
            });
        }
        requested
    } else {
        // Grant same rights minus GRANT
        source_rights & !Rights::GRANT
    };

    // Create new capability for target
    // Generate new capability ID
    use super::token::alloc_cap_id;
    let new_cap_id = alloc_cap_id().map_err(|_| KernelError::ResourceExhausted {
        resource: "capability IDs",
    })?;
    let new_cap = CapabilityToken::from_parts(
        new_cap_id,
        0, // Object ID - not used for now
        target_space.generation(),
        0, // Metadata
    );

    // Insert into target space
    target_space.insert(new_cap, object, target_rights)?;

    Ok(new_cap)
}

/// Revoke all capabilities derived from a parent capability
pub fn cascading_revoke(
    space: &CapabilitySpace,
    parent_cap: CapabilityToken,
) -> Result<usize, KernelError> {
    // In a full implementation, this would:
    // 1. Track parent-child relationships
    // 2. Find all derived capabilities
    // 3. Revoke them recursively
    // 4. Update generation counters

    // For now, just revoke the single capability
    if space.remove(parent_cap).is_some() {
        Ok(1)
    } else {
        Err(KernelError::InvalidCapability {
            cap_id: parent_cap.id(),
            reason: crate::error::CapError::NotFound,
        })
    }
}

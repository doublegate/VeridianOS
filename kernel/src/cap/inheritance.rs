//! Capability inheritance for process creation
//!
//! Implements how capabilities are inherited when creating new processes.

use super::{
    manager::cap_manager,
    space::{CapabilityEntry, CapabilitySpace},
    token::{CapabilityToken, Rights},
};
use crate::process::ProcessId;

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

/// Inherit capabilities from parent to child process
pub fn inherit_capabilities(
    parent_space: &CapabilitySpace,
    child_space: &CapabilitySpace,
    policy: InheritancePolicy,
) -> Result<u32, &'static str> {
    let mut inherited_count = 0;
    let mut _skipped_count = 0;

    match policy {
        InheritancePolicy::None => {
            // No inheritance
            Ok(0)
        }
        InheritancePolicy::All => {
            // Inherit all capabilities
            #[cfg(feature = "alloc")]
            {
                // Iterate through parent's L1 table
                for cap_id in 0..256 {
                    if let Some(cap_entry) = get_capability_at(parent_space, cap_id) {
                        if child_space
                            .insert(
                                cap_entry.capability,
                                cap_entry.object.clone(),
                                cap_entry.rights,
                            )
                            .is_ok()
                        {
                            inherited_count += 1;
                        }
                    }
                }

                // Handle L2 capabilities
                inherited_count += inherit_l2_capabilities(parent_space, child_space, None);
            }

            #[cfg(not(feature = "alloc"))]
            {
                // Only L1 table available
                for cap_id in 0..256 {
                    if let Some(cap_entry) = get_capability_at(parent_space, cap_id) {
                        if child_space
                            .insert(
                                cap_entry.capability,
                                cap_entry.object.clone(),
                                cap_entry.rights,
                            )
                            .is_ok()
                        {
                            inherited_count += 1;
                        }
                    }
                }
            }

            Ok(inherited_count)
        }
        InheritancePolicy::Inheritable => {
            // Only inherit capabilities marked as inheritable
            #[cfg(feature = "alloc")]
            {
                // Check L1 table
                for cap_id in 0..256 {
                    if let Some(cap_entry) = get_capability_at(parent_space, cap_id) {
                        if should_inherit(cap_entry.capability, cap_entry.inheritance_flags, policy)
                        {
                            if let Ok(()) = child_space.insert(
                                cap_entry.capability,
                                cap_entry.object.clone(),
                                cap_entry.rights,
                            ) {
                                inherited_count += 1
                            }
                        } else {
                            _skipped_count += 1;
                        }
                    }
                }

                // Handle L2 capabilities
                inherited_count += inherit_l2_capabilities(
                    parent_space,
                    child_space,
                    Some(InheritanceFlags::INHERITABLE),
                );
            }

            Ok(inherited_count)
        }
        InheritancePolicy::Reduced => {
            // Inherit with reduced rights (remove GRANT permission)
            #[cfg(feature = "alloc")]
            {
                for cap_id in 0..256 {
                    if let Some(cap_entry) = get_capability_at(parent_space, cap_id) {
                        let reduced_rights = reduce_rights_for_inheritance(cap_entry.rights);
                        if let Ok(()) = child_space.insert(
                            cap_entry.capability,
                            cap_entry.object.clone(),
                            reduced_rights,
                        ) {
                            inherited_count += 1
                        }
                    }
                }

                inherited_count += inherit_l2_capabilities_reduced(parent_space, child_space);
            }

            Ok(inherited_count)
        }
        InheritancePolicy::Custom => {
            // Apply custom filter - for now, same as Inheritable
            inherit_capabilities(parent_space, child_space, InheritancePolicy::Inheritable)
        }
    }
}

/// Fork inheritance - copy all capabilities to child
pub fn fork_inherit_capabilities(
    parent_space: &CapabilitySpace,
    child_space: &CapabilitySpace,
) -> Result<(), &'static str> {
    // In fork, child gets exact copy of parent's capabilities
    inherit_capabilities(parent_space, child_space, InheritancePolicy::All)?;
    Ok(())
}

/// Exec inheritance - filter capabilities based on preserve flags
pub fn exec_inherit_capabilities(
    old_space: &CapabilitySpace,
    new_space: &CapabilitySpace,
) -> Result<(), &'static str> {
    // Only preserve capabilities marked with PRESERVE_EXEC flag
    // TODO: Implement exec filtering
    inherit_capabilities(old_space, new_space, InheritancePolicy::Inheritable)?;
    Ok(())
}

/// Create initial capabilities for a new process
pub fn create_initial_capabilities(
    process_id: ProcessId,
    cap_space: &CapabilitySpace,
) -> Result<(), &'static str> {
    // Create basic capabilities that every process needs

    // 1. Capability to access its own process info
    let process_obj = super::object::ObjectRef::Process { pid: process_id };
    let process_rights = Rights::READ | Rights::MODIFY;
    cap_manager()
        .create_capability(process_obj, process_rights, cap_space)
        .map_err(|_| "Failed to create process capability")?;

    // 2. Basic IPC receive capability
    // TODO: Create default IPC endpoint for process

    // 3. Basic memory capabilities for stack and heap
    // TODO: Create memory capabilities for initial mappings

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
) -> Result<(), &'static str> {
    match syscall {
        "fork" => fork_inherit_capabilities(parent_space, child_space),
        "exec" => exec_inherit_capabilities(parent_space, child_space),
        "spawn" => {
            // New process with limited inheritance
            inherit_capabilities(parent_space, child_space, InheritancePolicy::Inheritable)
                .map(|_| ())
        }
        _ => Err("Unknown syscall for capability inheritance"),
    }
}

// Helper functions

/// Get capability at specific index in L1 table
fn get_capability_at(space: &CapabilitySpace, index: usize) -> Option<CapabilityEntry> {
    space.get_entry(index)
}

/// Inherit L2 capabilities with optional flag filter
#[cfg(feature = "alloc")]
fn inherit_l2_capabilities(
    parent_space: &CapabilitySpace,
    child_space: &CapabilitySpace,
    required_flag: Option<u32>,
) -> u32 {
    let mut inherited = 0;

    let _ = parent_space.iter_capabilities(|cap_entry| {
        // Skip L1 capabilities (already handled)
        if cap_entry.capability.id() < 256 {
            return true; // Continue iteration
        }

        // Check flag requirement
        if let Some(flag) = required_flag {
            if cap_entry.inheritance_flags & flag == 0 {
                return true; // Skip this one
            }
        }

        // Try to inherit
        if let Ok(()) = child_space.insert(
            cap_entry.capability,
            cap_entry.object.clone(),
            cap_entry.rights,
        ) {
            inherited += 1;
        }

        true // Continue iteration
    });

    inherited
}

/// Inherit L2 capabilities with reduced rights
#[cfg(feature = "alloc")]
fn inherit_l2_capabilities_reduced(
    parent_space: &CapabilitySpace,
    child_space: &CapabilitySpace,
) -> u32 {
    let mut inherited = 0;

    let _ = parent_space.iter_capabilities(|cap_entry| {
        // Skip L1 capabilities
        if cap_entry.capability.id() < 256 {
            return true;
        }

        let reduced_rights = reduce_rights_for_inheritance(cap_entry.rights);
        if let Ok(()) = child_space.insert(
            cap_entry.capability,
            cap_entry.object.clone(),
            reduced_rights,
        ) {
            inherited += 1;
        }

        true // Continue iteration
    });

    inherited
}

/// Delegate a capability to another process
pub fn delegate_capability(
    source_space: &CapabilitySpace,
    target_space: &CapabilitySpace,
    cap: CapabilityToken,
    new_rights: Option<Rights>,
) -> Result<CapabilityToken, &'static str> {
    // Lookup capability in source space
    let (object, source_rights) = source_space
        .lookup_entry(cap)
        .ok_or("Capability not found in source")?;

    // Check if source has grant right
    if !source_rights.contains(Rights::GRANT) {
        return Err("Source lacks grant permission");
    }

    // Determine rights for target
    let target_rights = if let Some(requested) = new_rights {
        // Can only grant rights that source has
        if !source_rights.contains(requested) {
            return Err("Cannot grant rights not possessed");
        }
        requested
    } else {
        // Grant same rights minus GRANT
        source_rights & !Rights::GRANT
    };

    // Create new capability for target
    // Generate new capability ID
    use super::token::alloc_cap_id;
    let new_cap_id = alloc_cap_id().map_err(|_| "Out of capability IDs")?;
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
) -> Result<usize, &'static str> {
    // In a full implementation, this would:
    // 1. Track parent-child relationships
    // 2. Find all derived capabilities
    // 3. Revoke them recursively
    // 4. Update generation counters

    // For now, just revoke the single capability
    if space.remove(parent_cap).is_some() {
        Ok(1)
    } else {
        Err("Capability not found")
    }
}

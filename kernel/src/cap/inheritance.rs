//! Capability inheritance for process creation
//!
//! Implements how capabilities are inherited when creating new processes.

use super::{
    manager::cap_manager,
    space::CapabilitySpace,
    token::{CapabilityToken, Rights},
};
use crate::process::ProcessId;

#[cfg(feature = "alloc")]
extern crate alloc;

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

/// Inherit capabilities from parent to child process
pub fn inherit_capabilities(
    parent_space: &CapabilitySpace,
    _child_space: &CapabilitySpace,
    policy: InheritancePolicy,
) -> Result<u32, &'static str> {
    let inherited_count = 0;

    match policy {
        InheritancePolicy::None => {
            // No inheritance
            Ok(0)
        }
        InheritancePolicy::All => {
            // Inherit all capabilities
            #[cfg(feature = "alloc")]
            {
                let parent_stats = parent_space.stats();
                let total_caps = parent_stats
                    .total_caps
                    .load(core::sync::atomic::Ordering::Relaxed);

                // TODO: Iterate through parent's capabilities and copy them
                // This requires adding an iterator to CapabilitySpace

                Ok(total_caps as u32)
            }

            #[cfg(not(feature = "alloc"))]
            Ok(0)
        }
        InheritancePolicy::Inheritable => {
            // Only inherit capabilities marked as inheritable
            // TODO: Check inheritance flags for each capability
            Ok(inherited_count)
        }
        InheritancePolicy::Reduced => {
            // Inherit with reduced rights (remove GRANT permission)
            // TODO: Implement reduced rights inheritance
            Ok(inherited_count)
        }
        InheritancePolicy::Custom => {
            // Apply custom filter
            // TODO: Allow custom inheritance filters
            Ok(inherited_count)
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

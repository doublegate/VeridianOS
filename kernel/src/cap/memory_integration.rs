//! Memory capability integration
//!
//! Integrates capability-based access control with memory management.

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::{vec, vec::Vec};

use super::{
    manager::{cap_manager, CapError},
    object::{MemoryAttributes, ObjectRef},
    space::CapabilitySpace,
    token::{CapabilityToken, Rights},
};
use crate::{
    mm::{PhysicalAddress, VirtualAddress},
    process::ProcessId,
};

/// Memory-specific capability rights
pub struct MemoryRights;

impl MemoryRights {
    /// Can read from memory region
    pub const READ: Rights = Rights::READ;
    /// Can write to memory region
    pub const WRITE: Rights = Rights::WRITE;
    /// Can execute code in memory region
    pub const EXECUTE: Rights = Rights::EXECUTE;
    /// Can map memory into address space
    pub const MAP: Rights = Rights::MODIFY;
    /// Can share memory with other processes
    pub const SHARE: Rights = Rights::GRANT;
}

/// Create a memory capability for a physical memory region
pub fn create_memory_capability(
    base: usize, // Physical address
    size: usize,
    attributes: MemoryAttributes,
    rights: Rights,
    cap_space: &CapabilitySpace,
) -> Result<CapabilityToken, CapError> {
    let object = ObjectRef::Memory {
        base,
        size,
        attributes,
    };

    cap_manager().create_capability(object, rights, cap_space)
}

/// Check if process has permission to map memory
pub fn check_map_permission(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
) -> Result<(), CapError> {
    super::manager::check_capability(cap, MemoryRights::MAP, cap_space)
}

/// Check if process has permission to read memory
pub fn check_read_permission(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
) -> Result<(), CapError> {
    super::manager::check_capability(cap, MemoryRights::READ, cap_space)
}

/// Check if process has permission to write memory
pub fn check_write_permission(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
) -> Result<(), CapError> {
    super::manager::check_capability(cap, MemoryRights::WRITE, cap_space)
}

/// Check if process has permission to execute in memory
pub fn check_execute_permission(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
) -> Result<(), CapError> {
    super::manager::check_capability(cap, MemoryRights::EXECUTE, cap_space)
}

/// Check memory access with specific range
pub fn check_memory_access(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
    _addr: VirtualAddress,
    _size: usize,
    access_type: MemoryAccessType,
) -> Result<(), CapError> {
    // First check basic permission
    let required_rights = match access_type {
        MemoryAccessType::Read => MemoryRights::READ,
        MemoryAccessType::Write => MemoryRights::WRITE,
        MemoryAccessType::Execute => MemoryRights::EXECUTE,
    };

    super::manager::check_capability(cap, required_rights, cap_space)?;

    // TODO: Check if the requested range falls within the capability's memory
    // region This requires looking up the ObjectRef from the capability

    Ok(())
}

/// Memory access type for permission checking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryAccessType {
    Read,
    Write,
    Execute,
}

/// Map memory with capability check
pub fn map_memory_with_capability(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
    _virt_addr: VirtualAddress,
    _phys_addr: PhysicalAddress,
    _size: usize,
    flags: crate::mm::PageFlags,
) -> Result<(), CapError> {
    // Check map permission
    check_map_permission(cap, cap_space)?;

    // Check if requested flags match capability rights
    if flags.contains(crate::mm::PageFlags::WRITABLE) {
        check_write_permission(cap, cap_space)?;
    }
    if flags.contains(crate::mm::PageFlags::EXECUTABLE) {
        check_execute_permission(cap, cap_space)?;
    }

    // TODO: Perform actual mapping through VMM
    // vmm::map_pages(virt_addr, phys_addr, size, flags)?;

    Ok(())
}

/// Share memory capability with another process
pub fn share_memory_capability(
    cap: CapabilityToken,
    source_cap_space: &CapabilitySpace,
    target_cap_space: &CapabilitySpace,
    new_rights: Rights,
) -> Result<CapabilityToken, CapError> {
    // Check if source has share permission
    super::manager::check_capability(cap, MemoryRights::SHARE, source_cap_space)?;

    // Delegate the capability
    cap_manager().delegate(cap, source_cap_space, target_cap_space, new_rights)
}

/// Create a shared memory region between processes
pub fn create_shared_memory(
    size: usize,
    owner_cap_space: &CapabilitySpace,
    share_with: &[(ProcessId, Rights, &CapabilitySpace)],
) -> Result<(PhysicalAddress, Vec<CapabilityToken>), CapError> {
    // TODO: Allocate physical memory for shared region
    let phys_addr = PhysicalAddress::new(0); // Placeholder

    let attributes = MemoryAttributes::normal();

    // Create capability for owner with full rights
    let owner_rights =
        MemoryRights::READ | MemoryRights::WRITE | MemoryRights::MAP | MemoryRights::SHARE;
    let owner_cap = create_memory_capability(
        phys_addr.as_usize(),
        size,
        attributes,
        owner_rights,
        owner_cap_space,
    )?;

    let mut caps = vec![owner_cap];

    // Create capabilities for other processes
    for (_pid, rights, cap_space) in share_with {
        let cap =
            create_memory_capability(phys_addr.as_usize(), size, attributes, *rights, cap_space)?;
        caps.push(cap);
    }

    Ok((phys_addr, caps))
}

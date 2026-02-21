//! Capability transfer through IPC
//!
//! This module handles the secure transfer of capabilities between processes
//! through IPC messages.

use super::{
    error::{IpcError, Result},
    message::Message,
};
use crate::{
    cap::{
        manager::{cap_manager, CapError},
        space::CapabilitySpace,
        token::{CapabilityToken, Rights},
    },
    process::ProcessId,
};

/// Transfer a capability through an IPC message
///
/// This function handles the delegation of a capability from the sender
/// to the receiver of an IPC message.
pub fn transfer_capability(
    msg: &Message,
    sender_cap_space: &CapabilitySpace,
    receiver_pid: ProcessId,
) -> Result<Option<CapabilityToken>> {
    let cap_id = msg.capability();

    // If no capability in message, nothing to transfer
    if cap_id == 0 {
        return Ok(None);
    }

    // Convert to capability token
    let cap_token = CapabilityToken::from_u64(cap_id);

    // Verify sender has the capability and GRANT right
    match crate::cap::manager::check_capability(cap_token, Rights::GRANT, sender_cap_space) {
        Ok(()) => {}
        Err(CapError::InvalidCapability) => return Err(IpcError::InvalidCapability),
        Err(CapError::InsufficientRights) => return Err(IpcError::PermissionDenied),
        Err(CapError::CapabilityRevoked) => return Err(IpcError::InvalidCapability),
        Err(_) => return Err(IpcError::InvalidCapability),
    }

    // Get receiver's capability space
    let receiver_process = match crate::process::table::get_process(receiver_pid) {
        Some(process) => process,
        None => return Err(IpcError::ProcessNotFound),
    };
    let receiver_cap_space = receiver_process.capability_space.lock();

    // Determine rights to transfer (same as sender's minus GRANT)
    let sender_rights = sender_cap_space
        .lookup(cap_token)
        .ok_or(IpcError::InvalidCapability)?;
    let transfer_rights = sender_rights.difference(Rights::GRANT);

    // Delegate the capability to receiver
    match cap_manager().delegate(
        cap_token,
        sender_cap_space,
        &receiver_cap_space,
        transfer_rights,
    ) {
        Ok(new_cap) => Ok(Some(new_cap)),
        Err(CapError::OutOfMemory) => Err(IpcError::OutOfMemory),
        Err(_) => Err(IpcError::PermissionDenied),
    }
}

/// Extract and validate capabilities from received message
///
/// This function should be called when a process receives an IPC message
/// to properly handle any transferred capabilities.
#[allow(dead_code)] // IPC capability transfer API
pub fn receive_capability(
    msg: &Message,
    receiver_cap_space: &CapabilitySpace,
) -> Result<Option<CapabilityToken>> {
    let cap_id = msg.capability();

    // If no capability in message, nothing to receive
    if cap_id == 0 {
        return Ok(None);
    }

    // Convert to capability token
    let cap_token = CapabilityToken::from_u64(cap_id);

    // Check if the capability exists in receiver's space
    // (It should have been transferred during send)
    if receiver_cap_space.lookup(cap_token).is_some() {
        Ok(Some(cap_token))
    } else {
        // Capability wasn't properly transferred
        Err(IpcError::InvalidCapability)
    }
}

/// Revoke a capability that was transferred via IPC
///
/// This allows the original owner to revoke a capability they previously
/// granted through IPC.
#[allow(dead_code)] // IPC capability revocation API
pub fn revoke_transferred_capability(
    cap_token: CapabilityToken,
    owner_cap_space: &CapabilitySpace,
) -> Result<()> {
    // Verify owner has the capability
    if owner_cap_space.lookup(cap_token).is_none() {
        return Err(IpcError::InvalidCapability);
    }

    // Revoke globally
    match cap_manager().revoke(cap_token) {
        Ok(()) => Ok(()),
        Err(CapError::InvalidCapability) => Err(IpcError::InvalidCapability),
        Err(_) => Err(IpcError::PermissionDenied),
    }
}

#[cfg(test)]
mod tests {
    // Tests would go here but are disabled in no_std environment
}

//! IPC capability integration
//!
//! Integrates capability-based access control with the IPC system.

use super::{
    manager::{cap_manager, CapError},
    object::ObjectRef,
    space::CapabilitySpace,
    token::{CapabilityToken, Rights},
};
use crate::{
    ipc::{EndpointId, IpcError},
    process::ProcessId,
};

#[cfg(feature = "alloc")]
extern crate alloc;

/// IPC-specific capability rights
pub struct IpcRights;

impl IpcRights {
    /// Can send messages to endpoint
    pub const SEND: Rights = Rights::WRITE;
    /// Can receive messages from endpoint
    pub const RECEIVE: Rights = Rights::READ;
    /// Can bind to endpoint
    pub const BIND: Rights = Rights::EXECUTE;
    /// Can delegate endpoint capability
    pub const GRANT: Rights = Rights::GRANT;
    /// Can create new endpoints
    pub const CREATE: Rights = Rights::CREATE;
}

/// Create an IPC endpoint capability
pub fn create_endpoint_capability(
    _endpoint_id: EndpointId,
    owner: ProcessId,
    rights: Rights,
    cap_space: &CapabilitySpace,
) -> Result<CapabilityToken, CapError> {
    // Use ObjectRef::Endpoint with an Arc<Endpoint> when alloc is available
    #[cfg(feature = "alloc")]
    {
        let endpoint = alloc::sync::Arc::new(crate::ipc::channel::Endpoint::new(owner));
        let object = ObjectRef::Endpoint { endpoint };
        cap_manager().create_capability(object, rights, cap_space)
    }
    #[cfg(not(feature = "alloc"))]
    {
        let object = ObjectRef::Process { pid: owner };
        cap_manager().create_capability(object, rights, cap_space)
    }
}

/// The endpoint `cap` names in `cap_space`, if it names one (N-80). Only an
/// endpoint capability confers IPC rights: the IPC rights are general bits
/// (SEND is WRITE), which capabilities of other objects carry too.
#[cfg(feature = "alloc")]
fn endpoint_of(cap: CapabilityToken, cap_space: &CapabilitySpace) -> Option<u64> {
    match cap_space.lookup_entry(cap) {
        Some((ObjectRef::Endpoint { endpoint }, _)) => Some(endpoint.id()),
        _ => None,
    }
}

/// `cap` is a valid endpoint capability in `cap_space` with `rights`.
fn check_ipc_permission(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
    rights: Rights,
) -> Result<(), IpcError> {
    match super::manager::check_capability(cap, rights, cap_space) {
        Ok(()) => {}
        Err(CapError::InvalidCapability) => return Err(IpcError::InvalidCapability),
        Err(CapError::InsufficientRights) => return Err(IpcError::PermissionDenied),
        Err(CapError::CapabilityRevoked) => return Err(IpcError::InvalidCapability),
        Err(_) => return Err(IpcError::PermissionDenied),
    }
    #[cfg(feature = "alloc")]
    if endpoint_of(cap, cap_space).is_none() {
        return Err(IpcError::InvalidCapability);
    }
    Ok(())
}

/// `cap` is a valid capability in `cap_space` for endpoint `endpoint_id`
/// with `rights`: the check for an operation on a known endpoint.
pub fn check_endpoint_permission(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
    endpoint_id: u64,
    rights: Rights,
) -> Result<(), IpcError> {
    check_ipc_permission(cap, cap_space, rights)?;
    #[cfg(feature = "alloc")]
    if endpoint_of(cap, cap_space) != Some(endpoint_id) {
        return Err(IpcError::InvalidCapability);
    }
    #[cfg(not(feature = "alloc"))]
    let _ = endpoint_id;
    Ok(())
}

/// Check if process has permission to send to endpoint
pub fn check_send_permission(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
) -> Result<(), IpcError> {
    check_ipc_permission(cap, cap_space, IpcRights::SEND)
}

/// Check if process has permission to receive from endpoint
pub fn check_receive_permission(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
) -> Result<(), IpcError> {
    check_ipc_permission(cap, cap_space, IpcRights::RECEIVE)
}

/// Check if process has permission to bind to endpoint
pub fn check_bind_permission(
    cap: CapabilityToken,
    cap_space: &CapabilitySpace,
) -> Result<(), IpcError> {
    check_ipc_permission(cap, cap_space, IpcRights::BIND)
}

/// Delegate IPC endpoint capability to another process
pub fn delegate_endpoint_capability(
    cap: CapabilityToken,
    source_cap_space: &CapabilitySpace,
    target_cap_space: &CapabilitySpace,
    new_rights: Rights,
) -> Result<CapabilityToken, IpcError> {
    match cap_manager().delegate(cap, source_cap_space, target_cap_space, new_rights) {
        Ok(new_cap) => Ok(new_cap),
        Err(CapError::InvalidCapability) => Err(IpcError::InvalidCapability),
        Err(CapError::PermissionDenied) => Err(IpcError::PermissionDenied),
        Err(_) => Err(IpcError::PermissionDenied),
    }
}

/// Wrapper for IPC operations with capability checks
pub struct CapabilityCheckedIpc;

impl CapabilityCheckedIpc {
    /// Send a message with capability check
    pub fn send_with_capability(
        endpoint_id: EndpointId,
        cap: CapabilityToken,
        cap_space: &CapabilitySpace,
        msg: crate::ipc::Message,
    ) -> Result<(), IpcError> {
        // Check send permission
        check_send_permission(cap, cap_space)?;

        // Perform the actual send
        crate::ipc::sync_send(msg, endpoint_id)
    }

    /// Receive a message with capability check
    pub fn receive_with_capability(
        endpoint_id: EndpointId,
        cap: CapabilityToken,
        cap_space: &CapabilitySpace,
    ) -> Result<crate::ipc::Message, IpcError> {
        // Check receive permission
        check_receive_permission(cap, cap_space)?;

        // Perform the actual receive
        crate::ipc::sync_receive(endpoint_id)
    }
}

/// Create a new IPC endpoint with initial capability
pub fn create_endpoint_with_capability(
    cap_space: &CapabilitySpace,
) -> Result<(EndpointId, CapabilityToken), IpcError> {
    // Get current process ID
    let owner = crate::process::current_process()
        .map(|p| p.pid)
        .unwrap_or(ProcessId(0));

    // Create the endpoint through the registry
    let (endpoint_id, _ipc_cap) = crate::ipc::registry::create_endpoint(owner)?;

    // Create full-rights capability for owner
    let rights = IpcRights::SEND | IpcRights::RECEIVE | IpcRights::BIND | IpcRights::GRANT;
    let cap = create_endpoint_capability(endpoint_id, owner, rights, cap_space)
        .map_err(|_| IpcError::OutOfMemory)?;

    Ok((endpoint_id, cap))
}

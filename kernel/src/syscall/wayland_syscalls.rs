//! Wayland compositor syscall handlers (Phase 6).
//!
//! Syscalls 240-247: Wayland client connection, surface management, and
//! event delivery.

use super::{SyscallError, SyscallResult};

/// Connect to the Wayland compositor.
///
/// # Returns
/// Client ID on success.
pub(super) fn sys_wl_connect() -> SyscallResult {
    crate::desktop::wayland::connect_client().map_err(|_| SyscallError::OutOfMemory)
}

/// Disconnect from the Wayland compositor.
pub(super) fn sys_wl_disconnect(client_id: usize) -> SyscallResult {
    crate::desktop::wayland::disconnect_client(client_id as u32);
    Ok(0)
}

/// Send a Wayland protocol message.
///
/// # Arguments
/// - `client_id`: Client identifier.
/// - `msg_ptr`: User-space pointer to message bytes.
/// - `msg_len`: Length of message in bytes.
pub(super) fn sys_wl_send_message(
    client_id: usize,
    msg_ptr: usize,
    msg_len: usize,
) -> SyscallResult {
    super::validate_user_buffer(msg_ptr, msg_len)?;

    // SAFETY: msg_ptr validated.
    let msg_bytes = unsafe { core::slice::from_raw_parts(msg_ptr as *const u8, msg_len) };

    crate::desktop::wayland::handle_client_message(client_id as u32, msg_bytes)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Receive pending Wayland events.
///
/// # Arguments
/// - `client_id`: Client identifier.
/// - `buf_ptr`: User-space buffer to receive event bytes.
/// - `buf_len`: Buffer capacity.
///
/// # Returns
/// Number of bytes written to buffer.
pub(super) fn sys_wl_recv_message(
    client_id: usize,
    buf_ptr: usize,
    buf_len: usize,
) -> SyscallResult {
    super::validate_user_buffer(buf_ptr, buf_len)?;

    let bytes_written =
        crate::desktop::wayland::read_client_events(client_id as u32, buf_ptr, buf_len)
            .map_err(|_| SyscallError::InvalidState)?;

    Ok(bytes_written)
}

/// Create a shared memory pool for Wayland buffers.
///
/// # Arguments
/// - `client_id`: Client identifier.
/// - `size`: Pool size in bytes.
///
/// # Returns
/// Pool object ID.
pub(super) fn sys_wl_create_shm_pool(client_id: usize, size: usize) -> SyscallResult {
    if size == 0 || size > 64 * 1024 * 1024 {
        return Err(SyscallError::InvalidArgument);
    }

    crate::desktop::wayland::create_shm_pool(client_id as u32, size)
        .map_err(|_| SyscallError::OutOfMemory)
}

/// Create a Wayland surface.
///
/// # Arguments
/// - `client_id`: Client identifier.
/// - `width`: Surface width.
/// - `height`: Surface height.
/// - `pool_id`: SHM pool to use for buffer.
///
/// # Returns
/// Surface object ID.
pub(super) fn sys_wl_create_surface(
    client_id: usize,
    width: usize,
    height: usize,
    pool_id: usize,
) -> SyscallResult {
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(SyscallError::InvalidArgument);
    }

    crate::desktop::wayland::create_surface(
        client_id as u32,
        width as u32,
        height as u32,
        pool_id as u32,
    )
    .map_err(|_| SyscallError::OutOfMemory)
}

/// Commit a surface (present the buffer contents).
///
/// # Arguments
/// - `client_id`: Client identifier.
/// - `surface_id`: Surface to commit.
pub(super) fn sys_wl_commit_surface(client_id: usize, surface_id: usize) -> SyscallResult {
    crate::desktop::wayland::commit_surface(client_id as u32, surface_id as u32)
        .map_err(|_| SyscallError::InvalidArgument)?;

    Ok(0)
}

/// Get pending input events for a Wayland window.
///
/// # Arguments
/// - `client_id`: Client identifier.
/// - `events_ptr`: User-space buffer for input events.
/// - `max_count`: Maximum events to return.
///
/// # Returns
/// Number of events written.
pub(super) fn sys_wl_get_events(
    client_id: usize,
    events_ptr: usize,
    max_count: usize,
) -> SyscallResult {
    use crate::drivers::input_event::InputEvent;

    if max_count == 0 {
        return Ok(0);
    }
    let byte_size = max_count
        .checked_mul(core::mem::size_of::<InputEvent>())
        .ok_or(SyscallError::InvalidArgument)?;
    super::validate_user_buffer(events_ptr, byte_size)?;

    let count = crate::desktop::wayland::get_client_events(client_id as u32, events_ptr, max_count)
        .map_err(|_| SyscallError::InvalidState)?;

    Ok(count)
}

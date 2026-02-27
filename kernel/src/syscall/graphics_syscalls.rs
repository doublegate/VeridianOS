//! Graphics and input syscall handlers (Phase 6).
//!
//! Syscalls 230-234: framebuffer info, framebuffer map, input polling/reading,
//! and double-buffer swap.

use super::{validate_user_ptr_typed, SyscallError, SyscallResult};
use crate::graphics::framebuffer::FbInfo;

/// Get framebuffer information.
///
/// # Arguments
/// - `info_ptr`: User-space pointer to `FbInfo` struct to fill.
pub(super) fn sys_fb_get_info(info_ptr: usize) -> SyscallResult {
    validate_user_ptr_typed::<FbInfo>(info_ptr)?;

    let fb_info = crate::graphics::framebuffer::get_fb_info().ok_or(SyscallError::InvalidState)?;

    // SAFETY: info_ptr validated as non-null, aligned, and in user space.
    unsafe {
        *(info_ptr as *mut FbInfo) = fb_info;
    }

    Ok(0)
}

/// Map the framebuffer into the calling process's address space.
///
/// # Arguments
/// - `phys_addr`: Physical address of the framebuffer (must match kernel's fb).
/// - `size`: Size in bytes to map.
///
/// # Returns
/// Virtual address of the mapping in user space, or error.
pub(super) fn sys_fb_map(phys_addr: usize, size: usize) -> SyscallResult {
    if size == 0 || size > 64 * 1024 * 1024 {
        return Err(SyscallError::InvalidArgument);
    }

    // Verify the requested phys_addr matches the actual framebuffer
    let real_phys = crate::graphics::framebuffer::get_phys_addr();
    if real_phys == 0 {
        return Err(SyscallError::InvalidState);
    }
    if phys_addr as u64 != real_phys {
        return Err(SyscallError::AccessDenied);
    }

    // Map the framebuffer physical frames into the process's VAS
    let vaddr = crate::mm::vas::map_physical_region_user(phys_addr as u64, size)?;

    Ok(vaddr)
}

/// Poll for pending input events.
///
/// # Arguments
/// - `timeout_ms`: Maximum time to wait (0 = non-blocking).
///
/// # Returns
/// Bitmask of ready input sources (bit 0 = keyboard, bit 1 = mouse).
pub(super) fn sys_input_poll(_timeout_ms: usize) -> SyscallResult {
    let mut mask: usize = 0;

    // Poll all input sources first
    crate::drivers::input_event::poll_all();

    // Check keyboard (always available if keyboard driver is initialized)
    if crate::drivers::keyboard::is_initialized() {
        mask |= 1;
    }

    // Check mouse
    if crate::drivers::mouse::is_initialized() {
        mask |= 2;
    }

    Ok(mask)
}

/// Read input events into user buffer.
///
/// # Arguments
/// - `events_ptr`: User-space pointer to `InputEvent` array.
/// - `max_count`: Maximum number of events to read.
///
/// # Returns
/// Number of events actually read.
pub(super) fn sys_input_read(events_ptr: usize, max_count: usize) -> SyscallResult {
    use crate::drivers::input_event::InputEvent;

    if max_count == 0 {
        return Ok(0);
    }
    let byte_size = max_count
        .checked_mul(core::mem::size_of::<InputEvent>())
        .ok_or(SyscallError::InvalidArgument)?;
    super::validate_user_buffer(events_ptr, byte_size)?;

    let mut count = 0usize;
    let events = events_ptr as *mut InputEvent;

    while count < max_count {
        if let Some(event) = crate::drivers::input_event::read_event() {
            // SAFETY: events_ptr validated for max_count entries.
            unsafe {
                events.add(count).write(event);
            }
            count += 1;
        } else {
            break;
        }
    }

    Ok(count)
}

/// Swap framebuffer (blit back-buffer to display).
pub(super) fn sys_fb_swap() -> SyscallResult {
    crate::graphics::fbcon::flush();
    Ok(0)
}

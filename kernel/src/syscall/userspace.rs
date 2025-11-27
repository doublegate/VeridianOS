//! User space memory access utilities
//!
//! Safe functions for copying data between kernel and user space.

use core::{slice, str};

use super::SyscallError;

/// Maximum string length we'll copy from user space
const MAX_USER_STRING_LEN: usize = 4096;

/// User space memory range constants
const USER_SPACE_START: usize = 0x0000_0000_0000_0000;
const USER_SPACE_END: usize = 0x0000_7FFF_FFFF_FFFF; // 128TB
const PAGE_SIZE: usize = 4096;

/// Check if a user pointer is valid with comprehensive validation
pub fn validate_user_ptr<T>(ptr: *const T, len: usize) -> Result<(), SyscallError> {
    let addr = ptr as usize;

    // Check for null pointer
    if addr == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // Calculate end address and check for overflow
    let end = addr.checked_add(len).ok_or(SyscallError::InvalidPointer)?;

    // Check address range is within user space
    // Note: USER_SPACE_START is 0, so we only need to check the upper bound
    if end > USER_SPACE_END {
        return Err(SyscallError::InvalidPointer);
    }

    // Validate page mappings for the entire range
    validate_page_mappings(addr, end)?;

    Ok(())
}

/// Validate that all pages in the given range are mapped and accessible
fn validate_page_mappings(start: usize, end: usize) -> Result<(), SyscallError> {
    // Get current process's address space
    let current_process = match crate::process::current_process() {
        Some(proc) => proc,
        None => return Err(SyscallError::ProcessNotFound),
    };

    // Get the virtual address space (VAS) from the process
    let _vas = current_process.memory_space.lock();

    // Check each page in the range
    for page_addr in (start..end).step_by(PAGE_SIZE) {
        // Use the VMM to check if the page is mapped
        if !crate::mm::is_user_addr_valid(page_addr) {
            return Err(SyscallError::UnmappedMemory);
        }

        // Additional check: verify the page is accessible from user mode
        // This would involve checking page table entry flags
        #[cfg(feature = "alloc")]
        {
            // Get the page table entry and check permissions
            if let Some(entry) = crate::mm::translate_user_address(page_addr) {
                // Check if page is user-accessible
                // The translate_user_address function already checks is_present()
                use crate::mm::user_validation::PageTableEntryExt;
                if !entry.is_user_accessible() {
                    return Err(SyscallError::AccessDenied);
                }
            } else {
                return Err(SyscallError::UnmappedMemory);
            }
        }
    }

    // Also check the last byte if it doesn't align with page boundary
    if !end.is_multiple_of(PAGE_SIZE) {
        let last_page = (end - 1) & !(PAGE_SIZE - 1);
        if !crate::mm::is_user_addr_valid(last_page) {
            return Err(SyscallError::UnmappedMemory);
        }
    }

    Ok(())
}

/// Check if a user pointer is valid (compatibility wrapper)
pub fn validate_user_ptr_compat(ptr: usize, size: usize) -> Result<(), SyscallError> {
    validate_user_ptr(ptr as *const u8, size)
}

/// Copy a null-terminated string from user space
///
/// # Safety
/// This function reads from user-provided pointers and must validate them
pub unsafe fn copy_string_from_user(user_ptr: usize) -> Result<String, SyscallError> {
    validate_user_ptr(user_ptr as *const u8, 1)?;

    // Find string length by looking for null terminator
    let mut len = 0;
    let mut ptr = user_ptr as *const u8;

    while len < MAX_USER_STRING_LEN {
        // Validate each page as we cross boundaries
        if len % 4096 == 0 {
            validate_user_ptr(ptr, 1)?;
        }

        let byte = ptr::read_volatile(ptr);
        if byte == 0 {
            break;
        }

        len += 1;
        ptr = ptr.offset(1);
    }

    if len >= MAX_USER_STRING_LEN {
        return Err(SyscallError::InvalidArgument);
    }

    // Copy the string
    let slice = slice::from_raw_parts(user_ptr as *const u8, len);
    use alloc::string::String;
    let string = String::from(str::from_utf8(slice).map_err(|_| SyscallError::InvalidArgument)?);

    Ok(string)
}

/// Copy data from user space to kernel space
///
/// # Safety
/// This function reads from user-provided pointers and must validate them
pub unsafe fn copy_from_user<T>(user_ptr: usize) -> Result<T, SyscallError>
where
    T: Copy,
{
    let size = core::mem::size_of::<T>();
    validate_user_ptr(user_ptr as *const T, size)?;

    // Use volatile read to prevent optimization issues
    let value = ptr::read_volatile(user_ptr as *const T);
    Ok(value)
}

/// Copy data from kernel space to user space
///
/// # Safety
/// This function writes to user-provided pointers and must validate them
pub unsafe fn copy_to_user<T>(user_ptr: usize, value: &T) -> Result<(), SyscallError>
where
    T: Copy,
{
    let size = core::mem::size_of::<T>();
    validate_user_ptr(user_ptr as *const T, size)?;

    // Use volatile write to prevent optimization issues
    ptr::write_volatile(user_ptr as *mut T, *value);
    Ok(())
}

/// Copy a byte slice from user space
///
/// # Safety
/// This function reads from user-provided pointers and must validate them
pub unsafe fn copy_slice_from_user(user_ptr: usize, len: usize) -> Result<Vec<u8>, SyscallError> {
    validate_user_ptr(user_ptr as *const u8, len)?;

    let slice = slice::from_raw_parts(user_ptr as *const u8, len);
    Ok(slice.to_vec())
}

/// Copy a byte slice to user space
///
/// # Safety
/// This function writes to user-provided pointers and must validate them
pub unsafe fn copy_slice_to_user(user_ptr: usize, data: &[u8]) -> Result<(), SyscallError> {
    validate_user_ptr(user_ptr as *const u8, data.len())?;

    let dest = slice::from_raw_parts_mut(user_ptr as *mut u8, data.len());
    dest.copy_from_slice(data);
    Ok(())
}

/// Copy a null-terminated string array from user space (like argv/envp)
///
/// # Safety
/// This function reads from user-provided pointers and must validate them
pub unsafe fn copy_string_array_from_user(array_ptr: usize) -> Result<Vec<String>, SyscallError> {
    if array_ptr == 0 {
        return Ok(Vec::new());
    }

    let mut strings = Vec::new();
    let mut current_ptr = array_ptr;

    // Read pointers until we hit null
    loop {
        validate_user_ptr(current_ptr as *const usize, 8)?; // 64-bit pointer
        let string_ptr = ptr::read_volatile(current_ptr as *const usize);

        if string_ptr == 0 {
            break;
        }

        let string = copy_string_from_user(string_ptr)?;
        strings.push(string);

        current_ptr += 8; // Move to next pointer

        // Sanity check
        if strings.len() > 1024 {
            return Err(SyscallError::InvalidArgument);
        }
    }

    Ok(strings)
}

use core::ptr;

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};

//! User space memory access utilities
//!
//! Safe functions for copying data between kernel and user space.

use core::{slice, str};

use super::SyscallError;

/// Maximum string length we'll copy from user space
const MAX_USER_STRING_LEN: usize = 4096;

/// Check if a user pointer is valid
pub fn validate_user_ptr(ptr: usize, size: usize) -> Result<(), SyscallError> {
    // Check for null pointer
    if ptr == 0 {
        return Err(SyscallError::InvalidPointer);
    }

    // Check for overflow
    if ptr.checked_add(size).is_none() {
        return Err(SyscallError::InvalidPointer);
    }

    // Check if pointer is in user space (below kernel space)
    // User space is 0x0 - 0x7FFF_FFFF_FFFF (128TB)
    if ptr >= 0x8000_0000_0000 {
        return Err(SyscallError::InvalidPointer);
    }

    // TODO: Check if the memory is actually mapped and accessible
    // This would involve walking the page tables

    Ok(())
}

/// Copy a null-terminated string from user space
///
/// # Safety
/// This function reads from user-provided pointers and must validate them
pub unsafe fn copy_string_from_user(user_ptr: usize) -> Result<String, SyscallError> {
    validate_user_ptr(user_ptr, 1)?;

    // Find string length by looking for null terminator
    let mut len = 0;
    let mut ptr = user_ptr as *const u8;

    while len < MAX_USER_STRING_LEN {
        // Validate each page as we cross boundaries
        if len % 4096 == 0 {
            validate_user_ptr(ptr as usize, 1)?;
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
    validate_user_ptr(user_ptr, size)?;

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
    validate_user_ptr(user_ptr, size)?;

    // Use volatile write to prevent optimization issues
    ptr::write_volatile(user_ptr as *mut T, *value);
    Ok(())
}

/// Copy a byte slice from user space
///
/// # Safety
/// This function reads from user-provided pointers and must validate them
pub unsafe fn copy_slice_from_user(user_ptr: usize, len: usize) -> Result<Vec<u8>, SyscallError> {
    validate_user_ptr(user_ptr, len)?;

    let slice = slice::from_raw_parts(user_ptr as *const u8, len);
    Ok(slice.to_vec())
}

/// Copy a byte slice to user space
///
/// # Safety
/// This function writes to user-provided pointers and must validate them
pub unsafe fn copy_slice_to_user(user_ptr: usize, data: &[u8]) -> Result<(), SyscallError> {
    validate_user_ptr(user_ptr, data.len())?;

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
        validate_user_ptr(current_ptr, 8)?; // 64-bit pointer
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

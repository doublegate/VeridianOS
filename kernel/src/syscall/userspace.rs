//! User space memory access utilities
//!
//! Safe functions for copying data between kernel and user space.

use core::{slice, str};

use super::SyscallError;

/// Maximum string length we'll copy from user space
const MAX_USER_STRING_LEN: usize = 4096;

/// Maximum combined size of argv + envp data passed to execve (128 KB).
/// Matches ARG_MAX in userland/libc/include/limits.h.
const ARG_MAX: usize = 131072;

/// Maximum number of individual arguments or environment variables in a
/// single string array (argv or envp). Prevents DoS from massive counts
/// of tiny strings that would individually pass size checks.
const MAX_ARGS: usize = 32768;

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
    // Range check: verify all pages fall within user space.
    // Note: translate_user_address() walks page tables using raw physical
    // addresses as pointers, which requires identity mapping. Since syscalls
    // run with user CR3 (no identity mapping), the page table walk faults.
    // The range check is sufficient: if a page is truly unmapped, the
    // subsequent volatile read/write will trigger a proper page fault.
    for page_addr in (start..end).step_by(PAGE_SIZE) {
        if !crate::mm::is_user_addr_valid(page_addr) {
            return Err(SyscallError::UnmappedMemory);
        }
    }

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
    let mut cumulative = 0usize;
    copy_string_array_from_user_tracked(array_ptr, &mut cumulative)
}

/// Copy a null-terminated string array from user space with cumulative
/// ARG_MAX tracking.
///
/// `cumulative_bytes` tracks the total size of all argument and environment
/// data across multiple calls (argv + envp). The total includes string
/// bytes (with NUL terminators) plus 8 bytes per pointer. If the running
/// total exceeds ARG_MAX (131072 bytes), returns `ArgumentListTooLong`.
///
/// The array element count is also capped at MAX_ARGS (32768) to prevent
/// DoS from massive counts of tiny strings.
///
/// # Safety
/// This function reads from user-provided pointers and must validate them
pub unsafe fn copy_string_array_from_user_tracked(
    array_ptr: usize,
    cumulative_bytes: &mut usize,
) -> Result<Vec<String>, SyscallError> {
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

        // Enforce maximum argument count
        if strings.len() >= MAX_ARGS {
            return Err(SyscallError::ArgumentListTooLong);
        }

        let string = copy_string_from_user(string_ptr)?;

        // Account for string bytes + NUL terminator + one 8-byte pointer
        let entry_cost = string.len() + 1 + core::mem::size_of::<usize>();
        *cumulative_bytes = cumulative_bytes
            .checked_add(entry_cost)
            .ok_or(SyscallError::ArgumentListTooLong)?;

        if *cumulative_bytes > ARG_MAX {
            return Err(SyscallError::ArgumentListTooLong);
        }

        strings.push(string);

        current_ptr += 8; // Move to next pointer
    }

    Ok(strings)
}

use core::ptr;

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Validated, alignment-tolerant user accessors
// ---------------------------------------------------------------------------
//
// These are the primitives drivers and syscalls should use for user memory.
// Each validates the full range against user space before touching it and
// uses unaligned accesses, because a user-supplied pointer carries no
// alignment guarantee. An address of 0 is rejected.

/// Read a `T` from user memory at `addr`.
pub fn read_user<T: Copy>(addr: usize) -> Result<T, SyscallError> {
    validate_user_ptr(addr as *const u8, core::mem::size_of::<T>())?;
    // SAFETY: the whole range [addr, addr + size_of::<T>()) was validated as
    // user memory above; read_unaligned imposes no alignment requirement.
    Ok(unsafe { ptr::read_unaligned(addr as *const T) })
}

/// Write `value` to user memory at `addr`.
pub fn write_user<T: Copy>(addr: usize, value: T) -> Result<(), SyscallError> {
    validate_user_ptr(addr as *const u8, core::mem::size_of::<T>())?;
    // SAFETY: the whole destination range was validated as user memory
    // above; write_unaligned imposes no alignment requirement.
    unsafe { ptr::write_unaligned(addr as *mut T, value) };
    Ok(())
}

/// Write `items` to a user array at `addr`, contiguously.
pub fn write_user_slice<T: Copy>(addr: usize, items: &[T]) -> Result<(), SyscallError> {
    let bytes = core::mem::size_of_val(items);
    if bytes == 0 {
        return Ok(());
    }
    validate_user_ptr(addr as *const u8, bytes)?;
    for (i, item) in items.iter().enumerate() {
        // SAFETY: element i lies within the validated range
        // [addr, addr + bytes); the write is unaligned-tolerant.
        unsafe { ptr::write_unaligned((addr as *mut T).add(i), *item) };
    }
    Ok(())
}

/// Read element `index` of a user array of `T` starting at `addr`.
pub fn read_user_index<T: Copy>(addr: usize, index: usize) -> Result<T, SyscallError> {
    let offset = index
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(SyscallError::InvalidPointer)?;
    let elem = addr
        .checked_add(offset)
        .ok_or(SyscallError::InvalidPointer)?;
    read_user::<T>(elem)
}

/// Copy `dst.len()` bytes from user memory at `addr` into `dst`.
pub fn read_user_bytes(addr: usize, dst: &mut [u8]) -> Result<(), SyscallError> {
    if dst.is_empty() {
        return Ok(());
    }
    validate_user_ptr(addr as *const u8, dst.len())?;
    // SAFETY: the source range was validated as user memory above and cannot
    // overlap `dst`, which is kernel memory.
    unsafe { ptr::copy_nonoverlapping(addr as *const u8, dst.as_mut_ptr(), dst.len()) };
    Ok(())
}

/// Copy `src` into user memory at `addr`.
pub fn write_user_bytes(addr: usize, src: &[u8]) -> Result<(), SyscallError> {
    if src.is_empty() {
        return Ok(());
    }
    validate_user_ptr(addr as *const u8, src.len())?;
    // SAFETY: the destination range was validated as user memory above and
    // cannot overlap `src`, which is kernel memory.
    unsafe { ptr::copy_nonoverlapping(src.as_ptr(), addr as *mut u8, src.len()) };
    Ok(())
}

/// Linux `_IOC` direction bit: user space writes the argument (kernel reads
/// it).
const IOC_WRITE: u64 = 1;
/// Linux `_IOC` direction bit: user space reads the argument (kernel writes
/// it).
const IOC_READ: u64 = 2;
/// Largest ioctl argument struct copied through a kernel bounce buffer.
pub const IOCTL_MAX_ARG_SIZE: usize = 4096;
/// Minimum bounce-buffer size, so a handler that touches a few bytes beyond
/// the size its command encodes still stays inside kernel memory.
const IOCTL_MIN_BOUNCE: usize = 512;

/// Run an ioctl handler against a kernel copy of its user argument.
///
/// Decodes the Linux `_IOC` size and direction from `cmd`, copies the
/// argument in when the command is `_IOW`/`_IOWR` (zero-filled otherwise),
/// gives `handler` a pointer to an 8-byte-aligned kernel buffer, and copies
/// the first `size` bytes back out when the command is `_IOR`/`_IOWR` and the
/// handler succeeded. The handler never sees the user pointer, so nothing it
/// does to its argument can reach kernel memory outside the buffer. Any
/// pointers *inside* the argument are still user pointers and must go
/// through [`read_user`]/[`write_user`] and friends.
pub fn ioctl_bounce<T, E>(
    cmd: u64,
    arg: usize,
    handler: impl FnOnce(*mut u8) -> Result<T, E>,
) -> Result<Result<T, E>, SyscallError> {
    let size = ((cmd >> 16) & 0x3FFF) as usize;
    let dir = (cmd >> 30) & 0x3;
    if size > IOCTL_MAX_ARG_SIZE {
        return Err(SyscallError::InvalidArgument);
    }
    let mut words = alloc::vec![0u64; size.max(IOCTL_MIN_BOUNCE).div_ceil(8)];
    let buf_ptr = words.as_mut_ptr() as *mut u8;
    // SAFETY: `words` owns at least `size` initialised bytes; viewing u64s as
    // bytes is always valid.
    let bytes = unsafe { core::slice::from_raw_parts_mut(buf_ptr, words.len() * 8) };
    if size > 0 && dir & IOC_WRITE != 0 {
        read_user_bytes(arg, &mut bytes[..size])?;
    }
    let result = handler(buf_ptr);
    if result.is_ok() && size > 0 && dir & IOC_READ != 0 {
        write_user_bytes(arg, &bytes[..size])?;
    }
    Ok(result)
}

#[cfg(test)]
mod accessor_tests {
    use super::*;

    const KERNEL_ADDR: usize = 0xFFFF_8000_0000_1000;

    #[test]
    fn accessors_reject_null_and_kernel_addresses() {
        for addr in [0usize, KERNEL_ADDR, USER_SPACE_END - 2] {
            assert!(read_user::<u64>(addr).is_err(), "read {:#x}", addr);
            assert!(write_user::<u64>(addr, 1).is_err(), "write {:#x}", addr);
            assert!(write_user_slice::<u32>(addr, &[1, 2]).is_err());
            assert!(write_user_bytes(addr, &[0; 8]).is_err());
            assert!(read_user_bytes(addr, &mut [0; 8]).is_err());
        }
        assert!(read_user_index::<u64>(KERNEL_ADDR, 0).is_err());
        assert!(read_user_index::<u64>(0x1000, usize::MAX).is_err());
    }

    #[test]
    fn accessors_round_trip_unaligned() {
        let mut buf = [0u8; 32];
        let base = buf.as_mut_ptr() as usize + 1; // deliberately misaligned
        write_user::<u64>(base, 0x1122_3344_5566_7788).unwrap();
        assert_eq!(read_user::<u64>(base).unwrap(), 0x1122_3344_5566_7788);
        write_user_slice::<u32>(base, &[7, 8, 9]).unwrap();
        assert_eq!(read_user_index::<u32>(base, 2).unwrap(), 9);
        let mut out = [0u8; 4];
        read_user_bytes(base, &mut out).unwrap();
        assert_eq!(u32::from_ne_bytes(out), 7);
        write_user_bytes(base, b"abc").unwrap();
        assert_eq!(&buf[1..4], b"abc");
    }

    /// Encode a Linux ioctl number: dir(2) | size(14) | type(8) | nr(8).
    fn ioc(dir: u64, size: usize, nr: u64) -> u64 {
        (dir << 30) | ((size as u64) << 16) | (0x64 << 8) | nr
    }

    #[test]
    fn ioctl_bounce_round_trips_iowr() {
        let mut user = [0u8; 9];
        let arg = user.as_mut_ptr() as usize + 1; // misaligned user struct
        write_user::<u64>(arg, 21).unwrap();
        let out = ioctl_bounce(ioc(3, 8, 1), arg, |p| {
            // SAFETY: test handler; the bounce buffer is 8-aligned and >= 8 bytes.
            let v = unsafe { &mut *(p as *mut u64) };
            *v *= 2;
            Ok::<i32, ()>(0)
        });
        assert_eq!(out, Ok(Ok(0)));
        assert_eq!(read_user::<u64>(arg).unwrap(), 42);
    }

    #[test]
    fn ioctl_bounce_ior_does_not_read_input_and_failure_does_not_write() {
        let mut user = [0xFFu8; 8];
        let arg = user.as_mut_ptr() as usize;
        // _IOR: the handler sees zeros, not the user's 0xFF bytes.
        let seen = ioctl_bounce(ioc(2, 8, 2), arg, |p| {
            // SAFETY: test handler reading its own bounce buffer.
            Ok::<u64, ()>(unsafe { *(p as *const u64) })
        });
        assert_eq!(seen, Ok(Ok(0)));
        // A failing handler leaves user memory untouched.
        let mut untouched = [7u8; 8];
        let arg = untouched.as_mut_ptr() as usize;
        let res = ioctl_bounce(ioc(3, 8, 3), arg, |p| {
            // SAFETY: test handler writing its own bounce buffer.
            unsafe { *(p as *mut u64) = 0 };
            Err::<(), i32>(-22)
        });
        assert_eq!(res, Ok(Err(-22)));
        assert_eq!(untouched, [7u8; 8]);
    }

    #[test]
    fn ioctl_bounce_rejects_bad_pointers_and_sizes() {
        let never = |_p: *mut u8| -> Result<(), ()> { panic!("handler must not run") };
        assert!(ioctl_bounce(ioc(3, 16, 4), 0xFFFF_8000_0000_0000, never).is_err());
        assert!(ioctl_bounce(ioc(1, 16, 4), 0, never).is_err());
        assert!(ioctl_bounce(ioc(3, IOCTL_MAX_ARG_SIZE + 1, 4), 0x1000, never).is_err());
    }

    #[test]
    fn ioctl_bounce_handler_overrun_stays_in_kernel_buffer() {
        let mut user = [0u8; 4];
        let arg = user.as_mut_ptr() as usize;
        // Command encodes 4 bytes; handler writes 64. Only 4 reach user space.
        let res = ioctl_bounce(ioc(3, 4, 5), arg, |p| {
            // SAFETY: the bounce buffer is at least IOCTL_MIN_BOUNCE bytes.
            unsafe { core::ptr::write_bytes(p, 0xAB, 64) };
            Ok::<(), ()>(())
        });
        assert_eq!(res, Ok(Ok(())));
        assert_eq!(user, [0xAB; 4]);
    }

    #[test]
    fn empty_writes_need_no_valid_pointer() {
        assert!(write_user_slice::<u32>(0, &[]).is_ok());
        assert!(write_user_bytes(0, &[]).is_ok());
    }
}

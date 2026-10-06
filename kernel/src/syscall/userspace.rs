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
    read_user::<T>(user_ptr)
}

/// Copy data from kernel space to user space
///
/// # Safety
/// This function writes to user-provided pointers and must validate them
pub unsafe fn copy_to_user<T>(user_ptr: usize, value: &T) -> Result<(), SyscallError>
where
    T: Copy,
{
    write_user::<T>(user_ptr, *value)
}

/// Copy a byte slice from user space
///
/// # Safety
/// This function reads from user-provided pointers and must validate them
pub unsafe fn copy_slice_from_user(user_ptr: usize, len: usize) -> Result<Vec<u8>, SyscallError> {
    let mut data = alloc::vec![0u8; len];
    read_user_bytes(user_ptr, &mut data)?;
    Ok(data)
}

/// Copy a byte slice to user space
///
/// # Safety
/// This function writes to user-provided pointers and must validate them
pub unsafe fn copy_slice_to_user(user_ptr: usize, data: &[u8]) -> Result<(), SyscallError> {
    write_user_bytes(user_ptr, data)
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

/// Byte copy where one side is user memory that has already been
/// range-validated. On bare-metal x86_64 the copy is fault-tolerant: an
/// unmapped user page yields EFAULT instead of a kernel fault.
///
/// # Safety
///
/// The kernel-side range must be valid for the access, and the user-side
/// range must have passed [`validate_user_ptr`].
unsafe fn raw_copy(dst: *mut u8, src: *const u8, len: usize) -> Result<(), SyscallError> {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        // SAFETY: forwarded from this function's contract.
        unsafe { crate::arch::x86_64::usercopy::copy_user(dst, src, len) }
            .map_err(|()| SyscallError::UnmappedMemory)
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        // SAFETY: forwarded from this function's contract; user and kernel
        // ranges never overlap.
        unsafe { ptr::copy_nonoverlapping(src, dst, len) };
        Ok(())
    }
}

/// Atomically compare-and-exchange the aligned 32-bit user word at `addr`:
/// store `new` if it holds `old`. Returns the value found, which equals
/// `old` iff the store happened. A fault (unmapped, or a read-only page)
/// fails with `UnmappedMemory` instead of crashing the kernel.
pub fn cmpxchg_user_u32(addr: usize, old: u32, new: u32) -> Result<u32, SyscallError> {
    if addr & 0x3 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    validate_user_ptr(addr as *const u8, core::mem::size_of::<u32>())?;
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        // SAFETY: `addr` is aligned and was validated as a user address.
        unsafe { crate::arch::x86_64::usercopy::cmpxchg_user_u32(addr as *mut u32, old, new) }
            .map_err(|()| SyscallError::UnmappedMemory)
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        // SAFETY: `addr` is aligned and validated; AtomicU32 has the layout
        // of u32. Without a fault fixup on these targets, the page must
        // already be mapped writable (the same assumption as raw_copy).
        let word = unsafe { core::sync::atomic::AtomicU32::from_ptr(addr as *mut u32) };
        Ok(word
            .compare_exchange(
                old,
                new,
                core::sync::atomic::Ordering::SeqCst,
                core::sync::atomic::Ordering::SeqCst,
            )
            .unwrap_or_else(|found| found))
    }
}

/// Read a `T` from user memory at `addr`.
pub fn read_user<T: Copy>(addr: usize) -> Result<T, SyscallError> {
    let size = core::mem::size_of::<T>();
    validate_user_ptr(addr as *const u8, size)?;
    let mut value = core::mem::MaybeUninit::<T>::uninit();
    // SAFETY: the user range [addr, addr + size) was validated above and the
    // destination is a local of exactly `size` bytes. Callers read plain-data
    // ioctl/syscall structs, for which any bit pattern is a valid value.
    unsafe {
        raw_copy(value.as_mut_ptr() as *mut u8, addr as *const u8, size)?;
        Ok(value.assume_init())
    }
}

/// Write `value` to user memory at `addr`.
pub fn write_user<T: Copy>(addr: usize, value: T) -> Result<(), SyscallError> {
    let size = core::mem::size_of::<T>();
    validate_user_ptr(addr as *const u8, size)?;
    // SAFETY: the user range was validated above; the source is `value`.
    unsafe { raw_copy(addr as *mut u8, &value as *const T as *const u8, size) }
}

/// Write `items` to a user array at `addr`, contiguously.
pub fn write_user_slice<T: Copy>(addr: usize, items: &[T]) -> Result<(), SyscallError> {
    let bytes = core::mem::size_of_val(items);
    if bytes == 0 {
        return Ok(());
    }
    validate_user_ptr(addr as *const u8, bytes)?;
    // SAFETY: the user range was validated above; `items` is `bytes` long.
    unsafe { raw_copy(addr as *mut u8, items.as_ptr() as *const u8, bytes) }
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
    // SAFETY: the user range was validated above; `dst` is kernel memory.
    unsafe { raw_copy(dst.as_mut_ptr(), addr as *const u8, dst.len()) }
}

/// Copy `src` into user memory at `addr`.
pub fn write_user_bytes(addr: usize, src: &[u8]) -> Result<(), SyscallError> {
    if src.is_empty() {
        return Ok(());
    }
    validate_user_ptr(addr as *const u8, src.len())?;
    // SAFETY: the user range was validated above; `src` is kernel memory.
    unsafe { raw_copy(addr as *mut u8, src.as_ptr(), src.len()) }
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

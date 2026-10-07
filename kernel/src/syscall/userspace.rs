//! User space memory access utilities
//!
//! Safe functions for copying data between kernel and user space.

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
use crate::mm::user_layout::USER_SPACE_END; // exclusive end
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
/// Kept `unsafe` for its callers; the copy itself is fault-tolerant and
/// validated (see [`read_user_cstr`]).
pub unsafe fn copy_string_from_user(user_ptr: usize) -> Result<String, SyscallError> {
    read_user_cstr(user_ptr, MAX_USER_STRING_LEN - 1)
}

/// Read a NUL-terminated UTF-8 string of at most `max_len` bytes (not
/// counting the NUL) from user memory.
///
/// The string is read in chunks that never cross a page boundary, each
/// through [`read_user_bytes`], so an unmapped page fails with EFAULT
/// instead of faulting in the kernel, and nothing past the terminating
/// NUL's page is touched. The previous readers dereferenced the user
/// pointer directly (review of the v0.26.0 stack, PR #14). A string with
/// no NUL within `max_len` bytes is `InvalidArgument`.
pub fn read_user_cstr(addr: usize, max_len: usize) -> Result<String, SyscallError> {
    const PAGE: usize = 4096;
    let mut out: Vec<u8> = Vec::new();
    let mut cursor = addr;
    let mut chunk = [0u8; 256];
    // max_len bytes of text plus the NUL.
    while out.len() <= max_len {
        let to_page_end = PAGE - (cursor % PAGE);
        let want = to_page_end.min(chunk.len()).min(max_len + 1 - out.len());
        read_user_bytes(cursor, &mut chunk[..want])?;
        if let Some(nul) = chunk[..want].iter().position(|&b| b == 0) {
            out.extend_from_slice(&chunk[..nul]);
            return String::from_utf8(out).map_err(|_| SyscallError::InvalidArgument);
        }
        out.extend_from_slice(&chunk[..want]);
        cursor = cursor
            .checked_add(want)
            .ok_or(SyscallError::InvalidPointer)?;
    }
    Err(SyscallError::InvalidArgument)
}

/// Copy data from user space to kernel space
///
/// # Safety
/// Kept `unsafe` for its callers; the copy is validated (see [`read_user`]),
/// and `T: UserPod` guarantees any bytes user space supplies form a valid
/// `T`.
pub unsafe fn copy_from_user<T>(user_ptr: usize) -> Result<T, SyscallError>
where
    T: UserPod,
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

/// Types that can be built from arbitrary bytes copied out of user memory.
///
/// [`read_user`], [`read_user_index`] and [`copy_from_user`] accept only
/// these, because user space controls every byte they copy. A bound of
/// plain `Copy` admitted `bool`, `char`, enums, `NonZero*` and references,
/// for which most bit patterns are undefined behaviour (review of the
/// v0.26.0 stack, PR #8):
///
/// ```compile_fail,E0277
/// let _ = veridian_kernel::read_user::<bool>(0x1000);
/// ```
///
/// while a plain integer is accepted:
///
/// ```no_run
/// let _ = veridian_kernel::read_user::<u32>(0x1000);
/// ```
///
/// # Safety
///
/// Implement only for types for which every bit pattern of
/// `size_of::<Self>()` bytes is a valid value: integers, arrays of
/// `UserPod`, and `#[repr(C)]` structs whose fields are all `UserPod` and
/// that have no padding.
pub unsafe trait UserPod: Copy {}

macro_rules! impl_user_pod {
    ($($t:ty),* $(,)?) => {
        // SAFETY: every bit pattern is a valid value of a primitive integer.
        $(unsafe impl UserPod for $t {})*
    };
}

impl_user_pod!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

// SAFETY: an array has no padding between elements, so every bit pattern
// is valid when it is valid for each element.
unsafe impl<T: UserPod, const N: usize> UserPod for [T; N] {}

/// Read a `T` from user memory at `addr`.
pub fn read_user<T: UserPod>(addr: usize) -> Result<T, SyscallError> {
    let size = core::mem::size_of::<T>();
    validate_user_ptr(addr as *const u8, size)?;
    let mut value = core::mem::MaybeUninit::<T>::uninit();
    // SAFETY: the user range [addr, addr + size) was validated above and the
    // destination is a local of exactly `size` bytes. `T: UserPod`, so any
    // bit pattern user space supplies is a valid `T`.
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
pub fn read_user_index<T: UserPod>(addr: usize, index: usize) -> Result<T, SyscallError> {
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

/// Bounce-buffer size for bulk transfers between the kernel and user memory.
pub(crate) const USER_COPY_CHUNK: usize = 64 * 1024;

/// Move up to `count` bytes produced by the kernel into user memory at
/// `addr`, one bounded kernel buffer at a time.
///
/// `produce` fills the buffer it is given and returns how many bytes it
/// wrote. With `repeat`, another chunk follows a full one (regular files,
/// where a further read cannot block); without it `produce` runs exactly
/// once, so a pipe, socket or tty read keeps its one-call semantics.
///
/// User memory is touched only through [`write_user_bytes`]: an unmapped
/// page gives EFAULT, or the partial count if some bytes already reached the
/// caller, as Linux does. Bytes `produce` consumed but that could not be
/// copied out are lost, as they are for Linux on a mid-copy fault. Before
/// v0.27 these paths built slices over user memory and faulted in the kernel
/// instead (N-43).
pub fn produce_to_user(
    addr: usize,
    count: usize,
    repeat: bool,
    mut produce: impl FnMut(&mut [u8]) -> Result<usize, SyscallError>,
) -> Result<usize, SyscallError> {
    if count == 0 {
        return Ok(0);
    }
    validate_user_ptr(addr as *const u8, count)?;
    let mut kbuf = alloc::vec![0u8; count.min(USER_COPY_CHUNK)];
    let mut total = 0;
    while total < count {
        let want = (count - total).min(kbuf.len());
        let n = match produce(&mut kbuf[..want]) {
            Ok(n) => n.min(want),
            Err(e) if total == 0 => return Err(e),
            Err(_) => break,
        };
        if n > 0 {
            if let Err(e) = write_user_bytes(addr + total, &kbuf[..n]) {
                return if total > 0 { Ok(total) } else { Err(e) };
            }
        }
        total += n;
        if !repeat || n < want {
            break;
        }
    }
    Ok(total)
}

/// Hand up to `count` bytes of user memory at `addr` to `consume`, one
/// bounded kernel buffer at a time. `consume` returns how many bytes it
/// accepted; a short count ends the transfer. `repeat` as for
/// [`produce_to_user`]. An unmapped page gives EFAULT, or the count accepted
/// so far.
pub fn consume_from_user(
    addr: usize,
    count: usize,
    repeat: bool,
    mut consume: impl FnMut(&[u8]) -> Result<usize, SyscallError>,
) -> Result<usize, SyscallError> {
    if count == 0 {
        return Ok(0);
    }
    validate_user_ptr(addr as *const u8, count)?;
    let mut kbuf = alloc::vec![0u8; count.min(USER_COPY_CHUNK)];
    let mut total = 0;
    while total < count {
        let want = (count - total).min(kbuf.len());
        if let Err(e) = read_user_bytes(addr + total, &mut kbuf[..want]) {
            return if total > 0 { Ok(total) } else { Err(e) };
        }
        let n = match consume(&kbuf[..want]) {
            Ok(n) => n.min(want),
            Err(e) if total == 0 => return Err(e),
            Err(_) => break,
        };
        total += n;
        if !repeat || n < want {
            break;
        }
    }
    Ok(total)
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

// Host only: these tests pass stack buffers as "user" pointers, which are
// kernel-half addresses under the bare-metal harness, where boot tests
// cover the accessors instead (review of the v0.26.0 stack, PR #8).
#[cfg(all(test, not(target_os = "none")))]
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

    /// The UserPod types other callers read: integers and arrays of them
    /// (e.g. a timespec as `[i64; 2]`).
    #[test]
    fn user_pod_arrays_and_signed_integers_read_back() {
        let mut buf = [0u8; 40];
        let base = buf.as_mut_ptr() as usize + 3;
        write_user::<[i64; 2]>(base, [-5, 7]).unwrap();
        assert_eq!(read_user::<[i64; 2]>(base).unwrap(), [-5, 7]);
        assert_eq!(read_user::<i64>(base).unwrap(), -5);
        assert_eq!(read_user_index::<usize>(base, 1).unwrap(), 7);
        // SAFETY: as for read_user; the address is a valid host buffer.
        assert_eq!(
            unsafe { copy_from_user::<[u32; 2]>(base) }.unwrap()[0],
            -5i32 as u32
        );
    }

    #[test]
    fn produce_to_user_chunks_regular_reads_and_stops_short() {
        // A source of 150_000 bytes, more than two chunks.
        let src: alloc::vec::Vec<u8> = (0..150_000u32).map(|i| i as u8).collect();
        let mut dst = alloc::vec![0u8; 200_000];
        let mut pos = 0;
        let n = produce_to_user(dst.as_mut_ptr() as usize, dst.len(), true, |k| {
            let n = k.len().min(src.len() - pos);
            k[..n].copy_from_slice(&src[pos..pos + n]);
            pos += n;
            Ok(n)
        })
        .unwrap();
        assert_eq!(n, 150_000);
        assert_eq!(&dst[..n], &src[..]);
        // Without repeat, exactly one producer call, however large the request.
        let mut calls = 0;
        let n = produce_to_user(dst.as_mut_ptr() as usize, dst.len(), false, |k| {
            calls += 1;
            Ok(k.len())
        })
        .unwrap();
        assert_eq!((n, calls), (USER_COPY_CHUNK, 1));
        // An error before anything is copied is returned; after, the count.
        assert_eq!(
            produce_to_user(dst.as_mut_ptr() as usize, 10, true, |_| Err(
                SyscallError::WouldBlock
            )),
            Err(SyscallError::WouldBlock)
        );
        let mut first = true;
        let n = produce_to_user(dst.as_mut_ptr() as usize, dst.len(), true, |k| {
            if first {
                first = false;
                Ok(k.len())
            } else {
                Err(SyscallError::IoError)
            }
        })
        .unwrap();
        assert_eq!(n, USER_COPY_CHUNK);
        assert!(produce_to_user(KERNEL_ADDR, 8, true, |k| Ok(k.len())).is_err());
    }

    #[test]
    fn consume_from_user_hands_over_chunks_until_short() {
        let src: alloc::vec::Vec<u8> = (0..100_000u32).map(|i| (i * 7) as u8).collect();
        let mut got = alloc::vec::Vec::new();
        let n = consume_from_user(src.as_ptr() as usize, src.len(), true, |k| {
            got.extend_from_slice(k);
            Ok(k.len())
        })
        .unwrap();
        assert_eq!((n, &got[..]), (src.len(), &src[..]));
        // A consumer that accepts less ends the transfer there.
        let n =
            consume_from_user(src.as_ptr() as usize, src.len(), true, |k| Ok(k.len() / 2)).unwrap();
        assert_eq!(n, USER_COPY_CHUNK / 2);
        assert!(consume_from_user(KERNEL_ADDR, 8, true, |k| Ok(k.len())).is_err());
    }

    #[test]
    fn read_user_cstr_reads_across_chunks_and_enforces_the_limit() {
        // Longer than one 256-byte chunk, from a misaligned start.
        let mut text = alloc::vec![b'x'; 700];
        text.push(0);
        let mut buf = alloc::vec![0u8; 1];
        buf.extend_from_slice(&text);
        let addr = buf.as_ptr() as usize + 1;
        assert_eq!(read_user_cstr(addr, 700).unwrap().len(), 700);
        // One byte over the limit: the NUL is never reached.
        assert_eq!(
            read_user_cstr(addr, 699),
            Err(SyscallError::InvalidArgument)
        );
        assert!(read_user_cstr(0, 10).is_err());
        assert!(read_user_cstr(KERNEL_ADDR, 10).is_err());
        // The source covers max_len + 1 bytes: the reader copies that
        // much in one chunk before it looks for the NUL.
        let bad = [0xffu8, 0, 0, 0, 0];
        assert_eq!(
            read_user_cstr(bad.as_ptr() as usize, 4),
            Err(SyscallError::InvalidArgument)
        );
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

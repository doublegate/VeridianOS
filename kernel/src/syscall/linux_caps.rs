//! capget(2) and capset(2): a thread's Linux capability sets (ADR 0020).
//!
//! The header is `struct __user_cap_header_struct { u32 version; int pid; }`
//! and the data one or two `struct __user_cap_data_struct`s, by version
//! (Linux 7.3 kernel/capability.c).

use super::{
    process::process_of_thread,
    userspace::{read_user, write_user},
    SyscallError, SyscallResult,
};
use crate::process::{
    current_thread, get_process,
    linux_caps::{data_words, sets_from_user_data, CapData, CapSets, LINUX_CAPABILITY_VERSION_3},
    ThreadId,
};

/// The size of one data struct.
const DATA_SIZE: usize = core::mem::size_of::<CapData>();

/// The header's data struct count, or None after writing VERSION_3 back
/// for an unknown version (Linux's `cap_validate_magic`).
fn header_words(header: usize) -> Result<Option<usize>, SyscallError> {
    let version = read_user::<u32>(header)?;
    match data_words(version) {
        Some(words) => Ok(Some(words)),
        None => {
            write_user::<u32>(header, LINUX_CAPABILITY_VERSION_3)?;
            Ok(None)
        }
    }
}

/// The header's pid: a thread id, 0 for the caller.
fn header_pid(header: usize) -> Result<i32, SyscallError> {
    read_user::<i32>(header + 4)
}

/// capget(header, data). An unknown version is EINVAL, except with a null
/// `data`, which probes for the kernel's version and returns 0.
pub fn sys_capget(header: usize, data: usize) -> SyscallResult {
    let words = header_words(header)?;
    if data == 0 {
        return Ok(0);
    }
    let words = words.ok_or(SyscallError::InvalidArgument)?;
    let pid = header_pid(header)?;
    if pid < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let caps = target_caps(pid as u64)?;
    for (k, words) in caps.user_data()[..words].iter().enumerate() {
        write_user::<CapData>(data + k * DATA_SIZE, *words)?;
    }
    Ok(0)
}

/// The capability sets of thread `tid` (0: the caller); ESRCH if there is
/// no such thread.
fn target_caps(tid: u64) -> Result<CapSets, SyscallError> {
    let me = current_thread().ok_or(SyscallError::InvalidState)?;
    if tid == 0 || tid == me.tid.0 {
        return Ok(*me.caps.lock());
    }
    let owner = process_of_thread(tid as usize).ok_or(SyscallError::ProcessNotFound)?;
    let thread = get_process(owner)
        .and_then(|p| p.get_thread(ThreadId(tid)))
        .ok_or(SyscallError::ProcessNotFound)?;
    let caps = *thread.caps.lock();
    Ok(caps)
}

/// capset(header, data): change the caller's own sets only; any other pid
/// is EPERM (Linux since 2.6.24).
pub fn sys_capset(header: usize, data: usize) -> SyscallResult {
    let words = header_words(header)?.ok_or(SyscallError::InvalidArgument)?;
    let pid = header_pid(header)?;
    let me = current_thread().ok_or(SyscallError::InvalidState)?;
    if pid != 0 && pid as u64 != me.tid.0 {
        return Err(SyscallError::OperationNotPermitted);
    }
    let mut structs = [[0u32; 3]; 2];
    for (k, s) in structs[..words].iter_mut().enumerate() {
        *s = read_user::<CapData>(data + k * DATA_SIZE)?;
    }
    let (effective, permitted, inheritable) = sets_from_user_data(structs);
    me.caps.lock().capset(effective, permitted, inheritable)?;
    Ok(0)
}

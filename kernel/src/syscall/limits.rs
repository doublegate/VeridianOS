//! getrlimit, setrlimit and prlimit64 (N-224): Linux's do_prlimit on the
//! per-process limits (`process::rlimit`).

use super::{SyscallError, SyscallResult};
use crate::process::{creds::Credentials, rlimit::Rlimit};

/// Whether `caller` may read or change the limits of a process with
/// credentials `target` (Linux's check_prlimit_permission): root, or the
/// caller's real user and group are all of the target's IDs.
pub(crate) fn may_prlimit(caller: &Credentials, target: &Credentials) -> bool {
    caller.euid == 0
        || ([target.ruid, target.euid, target.suid]
            .iter()
            .all(|&id| id == caller.ruid)
            && [target.rgid, target.egid, target.sgid]
                .iter()
                .all(|&id| id == caller.rgid))
}

/// prlimit64(pid, resource, new, old): the limit of process `pid` (0: the
/// caller) goes to `old` and, if `new` is given, is replaced by it. ESRCH
/// for no such process, EPERM without permission (`may_prlimit`), the
/// `Limits::set` errors otherwise; `new` is read before anything changes.
#[cfg(feature = "alloc")]
pub fn sys_prlimit64(pid: usize, resource: usize, new_ptr: usize, old_ptr: usize) -> SyscallResult {
    let caller = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let pid = pid as u32 as i32;
    let target = match pid {
        0 => caller.clone(),
        p if p > 0 => {
            let target = crate::process::find_process(crate::process::ProcessId(p as u64))
                .ok_or(SyscallError::ProcessNotFound)?;
            if target.pid != caller.pid
                && !may_prlimit(&caller.credentials(), &target.credentials())
            {
                return Err(SyscallError::OperationNotPermitted);
            }
            target
        }
        _ => return Err(SyscallError::ProcessNotFound),
    };
    let new: Option<Rlimit> = (new_ptr != 0)
        .then(|| super::userspace::read_user(new_ptr))
        .transpose()?;
    let old = match new {
        Some(new) => target.set_rlimit(resource, new, caller.euid() == 0)?,
        None => target.limits().get(resource)?,
    };
    if old_ptr != 0 {
        super::userspace::write_user(old_ptr, old)?;
    }
    Ok(0)
}

/// getrlimit(resource, rlim): the caller's limit.
#[cfg(feature = "alloc")]
pub fn sys_getrlimit(resource: usize, rlim_ptr: usize) -> SyscallResult {
    if rlim_ptr == 0 {
        return Err(SyscallError::InvalidPointer);
    }
    sys_prlimit64(0, resource, 0, rlim_ptr)
}

/// setrlimit(resource, rlim): set the caller's limit.
#[cfg(feature = "alloc")]
pub fn sys_setrlimit(resource: usize, rlim_ptr: usize) -> SyscallResult {
    if rlim_ptr == 0 {
        return Err(SyscallError::InvalidPointer);
    }
    sys_prlimit64(0, resource, rlim_ptr, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prlimit_permission_needs_all_ids_or_root() {
        let user = Credentials::new(1000, 100);
        assert!(may_prlimit(&user, &Credentials::new(1000, 100)));
        assert!(!may_prlimit(&user, &Credentials::new(1001, 100)));
        assert!(!may_prlimit(&user, &Credentials::new(1000, 101)));
        let mut setuid_root = Credentials::new(1000, 100);
        setuid_root.euid = 0;
        assert!(!may_prlimit(&user, &setuid_root));
        assert!(may_prlimit(&Credentials::new(0, 0), &setuid_root));
    }
}

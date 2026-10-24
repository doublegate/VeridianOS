//! getrlimit, setrlimit and prlimit64 (N-224): Linux's do_prlimit on the
//! per-process limits (`process::rlimit`).

use super::{SyscallError, SyscallResult};
use crate::process::{
    creds::{Caller, Credentials},
    linux_caps::CAP_SYS_RESOURCE,
    rlimit::Rlimit,
};

/// Whether `caller` may read or change the limits of a process with
/// credentials `target` (Linux's check_prlimit_permission): the caller's
/// real user and group are all of the target's IDs, or it holds
/// CAP_SYS_RESOURCE.
pub(crate) fn may_prlimit(caller: &Caller, target: &Credentials) -> bool {
    let c = &caller.creds;
    ([target.ruid, target.euid, target.suid]
        .iter()
        .all(|&id| id == c.ruid)
        && [target.rgid, target.egid, target.sgid]
            .iter()
            .all(|&id| id == c.rgid))
        || caller.capable(CAP_SYS_RESOURCE)
}

/// prlimit64(pid, resource, new, old): the limit of process `pid` (0: the
/// caller) goes to `old` and, if `new` is given, is replaced by it. ESRCH
/// for no such process, EPERM without permission (`may_prlimit`), the
/// `Limits::set` errors otherwise (raising a hard limit needs
/// CAP_SYS_RESOURCE); `new` is read before anything changes.
#[cfg(feature = "alloc")]
pub fn sys_prlimit64(pid: usize, resource: usize, new_ptr: usize, old_ptr: usize) -> SyscallResult {
    let caller = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let me = Caller::current();
    let pid = pid as u32 as i32;
    let target = match pid {
        0 => caller.clone(),
        p if p > 0 => {
            let target = crate::process::find_process(crate::process::ProcessId(p as u64))
                .ok_or(SyscallError::ProcessNotFound)?;
            if target.pid != caller.pid && !may_prlimit(&me, &target.credentials()) {
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
        Some(new) => target.set_rlimit(resource, new, me.capable(CAP_SYS_RESOURCE))?,
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
    fn prlimit_permission_needs_all_ids_or_cap_sys_resource() {
        use crate::process::linux_caps::LinuxCaps;
        let caller = |uid, gid, caps: LinuxCaps| Caller {
            creds: Credentials::new(uid, gid),
            caps,
        };
        let user = caller(1000, 100, LinuxCaps::EMPTY);
        assert!(may_prlimit(&user, &Credentials::new(1000, 100)));
        assert!(!may_prlimit(&user, &Credentials::new(1001, 100)));
        assert!(!may_prlimit(&user, &Credentials::new(1000, 101)));
        let mut setuid_root = Credentials::new(1000, 100);
        setuid_root.euid = 0;
        assert!(!may_prlimit(&user, &setuid_root));
        assert!(may_prlimit(&Caller::KERNEL, &setuid_root));
        let resource = caller(1000, 100, LinuxCaps::EMPTY.with(CAP_SYS_RESOURCE));
        assert!(may_prlimit(&resource, &setuid_root));
        assert!(!may_prlimit(&caller(0, 0, LinuxCaps::EMPTY), &setuid_root));
    }
}

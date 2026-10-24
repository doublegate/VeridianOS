//! Process credentials (N-248): real, effective, saved and filesystem user
//! and group IDs and the supplementary groups, with the Linux rules for
//! changing them. The privilege to change them is the calling thread's
//! CAP_SETUID or CAP_SETGID, which the caller passes in (ADR 0020).
//! Filesystem privilege is still filesystem uid 0 (`fs_privileged`), the
//! approximation of Linux's filesystem capabilities (CHOWN, DAC_OVERRIDE,
//! FOWNER, MKNOD, ...), which leave the effective set with the fsuid.

use super::super::syscall::SyscallError;

/// Most supplementary groups a process can hold.
pub const NGROUPS_MAX: usize = 64;

/// "Leave unchanged" in setresuid and friends: (uid_t)-1.
pub const UNCHANGED: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Credentials {
    pub ruid: u32,
    pub euid: u32,
    pub suid: u32,
    pub rgid: u32,
    pub egid: u32,
    pub sgid: u32,
    /// The IDs file access is checked as and new files are owned by
    /// (Linux's fsuid, fsgid): the effective ones, unless set apart with
    /// setfsuid / setfsgid.
    pub fsuid: u32,
    pub fsgid: u32,
    ngroups: usize,
    groups: [u32; NGROUPS_MAX],
}

impl Default for Credentials {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl Credentials {
    /// All three user IDs `uid`, all three group IDs `gid`, no
    /// supplementary groups.
    pub const fn new(uid: u32, gid: u32) -> Self {
        Self {
            ruid: uid,
            euid: uid,
            suid: uid,
            rgid: gid,
            egid: gid,
            sgid: gid,
            fsuid: uid,
            fsgid: gid,
            ngroups: 0,
            groups: [0; NGROUPS_MAX],
        }
    }

    pub fn groups(&self) -> &[u32] {
        &self.groups[..self.ngroups]
    }

    /// Whether file access is unrestricted: filesystem uid 0.
    pub fn fs_privileged(&self) -> bool {
        self.fsuid == 0
    }

    /// Whether the process is in group `gid` for file access (its
    /// filesystem gid or a supplementary group; Linux's `in_group_p`).
    pub fn in_group(&self, gid: u32) -> bool {
        self.fsgid == gid || self.groups().contains(&gid)
    }

    /// The group ID a permission check should use against a file owned by
    /// `file_gid`: that group if the process belongs to it, otherwise the
    /// filesystem gid. With `Permissions::can_*` this gives POSIX's
    /// owner/group/other precedence over all the process's groups.
    pub fn gid_for(&self, file_gid: u32) -> u32 {
        if self.in_group(file_gid) {
            file_gid
        } else {
            self.fsgid
        }
    }

    /// The credentials a program runs with after exec (Linux's
    /// bprm_fill_uid and cap_bprm_creds_from_file). `set_uid` is the
    /// owner of a set-user-ID file, `set_gid` the group of a set-group-ID
    /// file with group execute; they become the effective IDs if
    /// `may_gain` (not under no_new_privs, not traced by an unprivileged
    /// tracer). The saved IDs then take the effective ones, whether or not
    /// they changed. The real IDs and supplementary groups stay.
    pub fn after_exec(&self, set_uid: Option<u32>, set_gid: Option<u32>, may_gain: bool) -> Self {
        let mut new = *self;
        if may_gain {
            if let Some(uid) = set_uid {
                new.euid = uid;
            }
            if let Some(gid) = set_gid {
                new.egid = gid;
            }
        }
        new.suid = new.euid;
        new.sgid = new.egid;
        new.fsuid = new.euid;
        new.fsgid = new.egid;
        new
    }

    /// setfsuid(uid): returns the old filesystem uid whatever happens. It
    /// changes only to the real, effective, saved or current filesystem
    /// uid, or anything when `may` (the caller holds CAP_SETUID); -1 just
    /// reads it (Linux's `__sys_setfsuid`).
    ///
    /// In every set*id rule here `may` is the calling thread's capability
    /// (ADR 0020): CAP_SETUID for the user IDs, CAP_SETGID for the group
    /// IDs and the supplementary groups.
    pub fn setfsuid(&mut self, uid: u32, may: bool) -> u32 {
        let old = self.fsuid;
        if uid != UNCHANGED && (may || [self.ruid, self.euid, self.suid, old].contains(&uid)) {
            self.fsuid = uid;
        }
        old
    }

    /// setfsgid(gid), the group counterpart of `setfsuid`.
    pub fn setfsgid(&mut self, gid: u32, may: bool) -> u32 {
        let old = self.fsgid;
        if gid != UNCHANGED && (may || [self.rgid, self.egid, self.sgid, old].contains(&gid)) {
            self.fsgid = gid;
        }
        old
    }

    /// setresuid(r, e, s): `UNCHANGED` keeps a value. Unprivileged, each
    /// new value must be one of the current real, effective or saved IDs.
    pub fn setresuid(&mut self, r: u32, e: u32, s: u32, may: bool) -> Result<(), SyscallError> {
        let current = [self.ruid, self.euid, self.suid];
        if !may
            && [r, e, s]
                .iter()
                .any(|&id| id != UNCHANGED && !current.contains(&id))
        {
            return Err(SyscallError::OperationNotPermitted);
        }
        if r != UNCHANGED {
            self.ruid = r;
        }
        if e != UNCHANGED {
            self.euid = e;
        }
        if s != UNCHANGED {
            self.suid = s;
        }
        self.fsuid = self.euid;
        Ok(())
    }

    /// setresgid(r, e, s), with the same rules over the group IDs.
    pub fn setresgid(&mut self, r: u32, e: u32, s: u32, may: bool) -> Result<(), SyscallError> {
        let current = [self.rgid, self.egid, self.sgid];
        if !may
            && [r, e, s]
                .iter()
                .any(|&id| id != UNCHANGED && !current.contains(&id))
        {
            return Err(SyscallError::OperationNotPermitted);
        }
        if r != UNCHANGED {
            self.rgid = r;
        }
        if e != UNCHANGED {
            self.egid = e;
        }
        if s != UNCHANGED {
            self.sgid = s;
        }
        self.fsgid = self.egid;
        Ok(())
    }

    /// setreuid(r, e): unprivileged, the real ID may become the real or
    /// effective one, the effective ID any of the three. The saved ID
    /// follows the new effective ID when the real ID is set, or the
    /// effective ID is set to something other than the old real ID.
    pub fn setreuid(&mut self, r: u32, e: u32, may: bool) -> Result<(), SyscallError> {
        let old = *self;
        if !may {
            if r != UNCHANGED && r != old.ruid && r != old.euid {
                return Err(SyscallError::OperationNotPermitted);
            }
            if e != UNCHANGED && e != old.ruid && e != old.euid && e != old.suid {
                return Err(SyscallError::OperationNotPermitted);
            }
        }
        if r != UNCHANGED {
            self.ruid = r;
        }
        if e != UNCHANGED {
            self.euid = e;
        }
        if r != UNCHANGED || (e != UNCHANGED && e != old.ruid) {
            self.suid = self.euid;
        }
        self.fsuid = self.euid;
        Ok(())
    }

    /// setregid(r, e), the group counterpart of `setreuid`.
    pub fn setregid(&mut self, r: u32, e: u32, may: bool) -> Result<(), SyscallError> {
        let old = *self;
        if !may {
            if r != UNCHANGED && r != old.rgid && r != old.egid {
                return Err(SyscallError::OperationNotPermitted);
            }
            if e != UNCHANGED && e != old.rgid && e != old.egid && e != old.sgid {
                return Err(SyscallError::OperationNotPermitted);
            }
        }
        if r != UNCHANGED {
            self.rgid = r;
        }
        if e != UNCHANGED {
            self.egid = e;
        }
        if r != UNCHANGED || (e != UNCHANGED && e != old.rgid) {
            self.sgid = self.egid;
        }
        self.fsgid = self.egid;
        Ok(())
    }

    /// setuid(uid): with `may`, sets all three IDs; otherwise `uid` must
    /// be the real or saved ID and only the effective ID changes.
    pub fn setuid(&mut self, uid: u32, may: bool) -> Result<(), SyscallError> {
        if uid == UNCHANGED {
            return Err(SyscallError::InvalidArgument);
        }
        if may {
            self.ruid = uid;
            self.euid = uid;
            self.suid = uid;
        } else if uid == self.ruid || uid == self.suid {
            self.euid = uid;
        } else {
            return Err(SyscallError::OperationNotPermitted);
        }
        self.fsuid = self.euid;
        Ok(())
    }

    /// setgid(gid), the group counterpart of `setuid`.
    pub fn setgid(&mut self, gid: u32, may: bool) -> Result<(), SyscallError> {
        if gid == UNCHANGED {
            return Err(SyscallError::InvalidArgument);
        }
        if may {
            self.rgid = gid;
            self.egid = gid;
            self.sgid = gid;
        } else if gid == self.rgid || gid == self.sgid {
            self.egid = gid;
        } else {
            return Err(SyscallError::OperationNotPermitted);
        }
        self.fsgid = self.egid;
        Ok(())
    }

    /// setgroups: only with `may`; at most `NGROUPS_MAX` groups, none of
    /// them (gid_t)-1, which is no group (EINVAL, as Linux's
    /// groups_from_user; it was installed -- PR #22 review).
    pub fn setgroups(&mut self, list: &[u32], may: bool) -> Result<(), SyscallError> {
        if !may {
            return Err(SyscallError::OperationNotPermitted);
        }
        if list.len() > NGROUPS_MAX || list.contains(&UNCHANGED) {
            return Err(SyscallError::InvalidArgument);
        }
        self.groups[..list.len()].copy_from_slice(list);
        self.ngroups = list.len();
        Ok(())
    }
}

/// Who a privilege or permission check is made for (ADR 0020, decision 3):
/// the process's credentials and the calling thread's effective
/// capabilities. Linux keeps both in the task's `cred`; here the IDs are
/// per process and the capability sets per thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    pub creds: Credentials,
    /// The effective set (pE): what the checks consult.
    pub caps: super::linux_caps::LinuxCaps,
}

impl Caller {
    /// The kernel itself (no current process, as the kernel console):
    /// root with every capability.
    pub const KERNEL: Self = Self {
        creds: Credentials::new(0, 0),
        caps: super::linux_caps::LinuxCaps::FULL,
    };

    /// The calling thread: its process's credentials and its own
    /// effective set. Kernel context, with no process, is [`Self::KERNEL`];
    /// a process without a current thread holds nothing (fails closed).
    pub fn current() -> Self {
        let Some(process) = super::current_process() else {
            return Self::KERNEL;
        };
        let caps = super::current_thread().map_or(super::linux_caps::LinuxCaps::EMPTY, |t| {
            t.caps.lock().effective
        });
        Self {
            creds: process.credentials(),
            caps,
        }
    }

    /// Whether the caller holds `cap` (Linux's `capable`, and
    /// `capable_wrt_inode_uidgid`, in the initial user namespace).
    pub const fn capable(&self, cap: u32) -> bool {
        self.caps.has(cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setgroups_refuses_the_invalid_gid() {
        let mut root = Credentials::new(0, 0);
        assert_eq!(
            root.setgroups(&[5, u32::MAX], true),
            Err(SyscallError::InvalidArgument)
        );
        assert!(root.groups().is_empty(), "nothing installed");
        let mut user = Credentials::new(1000, 1000);
        assert_eq!(
            user.setgroups(&[u32::MAX], false),
            Err(SyscallError::OperationNotPermitted)
        );
    }

    const U: u32 = UNCHANGED;

    /// The filesystem IDs (Linux's fsuid/fsgid) follow the effective IDs
    /// through every change and exec, unless set apart with setfsuid or
    /// setfsgid; permission checks and file ownership use them.
    #[test]
    fn fs_ids_follow_the_effective_ids() {
        let mut c = Credentials::new(0, 0);
        assert_eq!((c.fsuid, c.fsgid), (0, 0));
        c.setresgid(U, 100, U, true).unwrap();
        assert_eq!(c.fsgid, 100);
        c.setresuid(U, 1000, U, true).unwrap();
        assert_eq!(c.fsuid, 1000);
        c.setresuid(U, 0, U, false).unwrap(); // back through the saved ID
        assert_eq!(c.fsuid, 0);
        c.setreuid(U, 7, true).unwrap();
        assert_eq!(c.fsuid, 7);
        let mut c = Credentials::new(0, 0);
        c.setregid(U, 9, true).unwrap();
        c.setgid(8, true).unwrap();
        c.setuid(42, true).unwrap();
        assert_eq!((c.fsuid, c.fsgid), (42, 8));
        let c = Credentials::new(1000, 100).after_exec(Some(0), Some(5), true);
        assert_eq!((c.fsuid, c.fsgid), (0, 5));
    }

    /// setfsuid/setfsgid return the old value whatever happens, take -1 as
    /// a query, and change the ID only to the real, effective, saved or
    /// current filesystem ID, unless the caller holds CAP_SETUID (Linux's
    /// `__sys_setfsuid`). Filesystem privilege and group checks follow.
    #[test]
    fn setfsuid_rules() {
        let mut root = Credentials::new(0, 0);
        assert_eq!(root.setfsuid(1000, true), 0);
        assert_eq!(root.setfsuid(U, true), 1000);
        assert_eq!((root.euid, root.fsuid), (0, 1000));
        assert!(!root.fs_privileged());
        assert_eq!(root.setfsgid(50, true), 0);
        assert!(!root.in_group(0) && root.in_group(50));
        assert_eq!(root.gid_for(50), 50);
        assert_eq!(root.gid_for(60), 50);
        // Still holds CAP_SETUID: may go back.
        assert_eq!(root.setfsuid(0, true), 1000);
        assert!(root.fs_privileged());

        let mut user = Credentials::new(1000, 100);
        assert_eq!(user.setfsuid(0, false), 1000);
        assert_eq!(user.fsuid, 1000, "not one of its IDs");
        assert_eq!(user.setfsgid(0, false), 100);
        assert_eq!(user.fsgid, 100);
        // A setuid-root program's saved ID is allowed.
        let mut c = Credentials::new(1000, 100);
        c.suid = 0;
        assert_eq!(c.setfsuid(0, false), 1000);
        assert_eq!(c.fsuid, 0);
        assert!(c.fs_privileged());
    }

    #[test]
    fn exec_of_a_setuid_program_takes_its_owner() {
        let user = Credentials::new(1000, 100);
        // pkexec: setuid root.
        let c = user.after_exec(Some(0), None, true);
        assert_eq!((c.ruid, c.euid, c.suid), (1000, 0, 0));
        assert_eq!((c.rgid, c.egid, c.sgid), (100, 100, 100));
        // setgid with group execute.
        let c = user.after_exec(None, Some(5), true);
        assert_eq!((c.rgid, c.egid, c.sgid), (100, 5, 5));
        // no_new_privs or an unprivileged tracer: nothing is gained.
        let c = user.after_exec(Some(0), Some(5), false);
        assert_eq!((c.euid, c.suid, c.egid, c.sgid), (1000, 1000, 100, 100));
    }

    #[test]
    fn exec_sets_the_saved_ids_from_the_effective_ones() {
        // A setuid-root program that dropped to the user with seteuid:
        // exec of an ordinary program leaves the root saved ID behind.
        let mut c = Credentials::new(1000, 100);
        c.euid = 1000;
        c.suid = 0;
        let c = c.after_exec(None, None, true);
        assert_eq!((c.ruid, c.euid, c.suid), (1000, 1000, 1000));
    }

    #[test]
    fn root_drops_privilege_for_good_with_setuid() {
        let mut c = Credentials::new(0, 0);
        c.setuid(1000, true).unwrap();
        assert_eq!((c.ruid, c.euid, c.suid), (1000, 1000, 1000));
        // The setuid fixup has cleared its capabilities.
        assert_eq!(c.setuid(0, false), Err(SyscallError::OperationNotPermitted));
    }

    #[test]
    fn seteuid_round_trip_keeps_the_saved_id() {
        // A setuid-root program: real 1000, effective and saved 0.
        let mut c = Credentials::new(1000, 100);
        c.euid = 0;
        c.suid = 0;
        c.setresuid(U, 1000, U, true).unwrap(); // seteuid(1000)
        assert_eq!((c.ruid, c.euid, c.suid), (1000, 1000, 0));
        c.setresuid(U, 0, U, false).unwrap(); // back through the saved ID
        assert_eq!(c.euid, 0);
    }

    #[test]
    fn unprivileged_cannot_take_new_ids() {
        let mut c = Credentials::new(1000, 100);
        assert_eq!(
            c.setresuid(U, 0, U, false),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(
            c.setresgid(U, 0, U, false),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(
            c.setreuid(2000, U, false),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(
            c.setgroups(&[5], false),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(c, Credentials::new(1000, 100));
        // Values it already holds are allowed.
        c.setresuid(1000, 1000, 1000, false).unwrap();
    }

    /// The capability decides, not uid 0 (ADR 0020): a non-root caller
    /// holding CAP_SETUID may take any ID, and root without it may not.
    #[test]
    fn the_capability_decides_not_uid_0() {
        let mut c = Credentials::new(1000, 100);
        c.setuid(0, true).unwrap();
        assert_eq!((c.ruid, c.euid, c.suid, c.fsuid), (0, 0, 0, 0));
        let mut c = Credentials::new(1000, 100);
        c.setresgid(5, 5, 5, true).unwrap();
        c.setgroups(&[7], true).unwrap();
        assert_eq!((c.rgid, c.groups()), (5, &[7][..]));

        let mut root = Credentials::new(0, 0);
        assert_eq!(
            root.setresuid(U, 1000, U, false),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(
            root.setgroups(&[5], false),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(root.setfsuid(1000, false), 0);
        assert_eq!(root.fsuid, 0, "not one of its IDs");
        assert_eq!(root, Credentials::new(0, 0));
    }

    #[test]
    fn setreuid_updates_the_saved_id_like_linux() {
        let mut c = Credentials::new(0, 0);
        c.setreuid(U, 1000, true).unwrap(); // effective != old real: saved follows
        assert_eq!((c.ruid, c.euid, c.suid), (0, 1000, 1000));
        let mut c = Credentials::new(0, 0);
        c.setreuid(1000, 1000, true).unwrap();
        assert_eq!((c.ruid, c.euid, c.suid), (1000, 1000, 1000));
    }

    #[test]
    fn supplementary_groups_grant_group_access() {
        let mut c = Credentials::new(0, 0);
        c.setgroups(&[10, 20], true).unwrap();
        c.setresuid(1000, 1000, 1000, true).unwrap();
        assert!(c.in_group(20));
        assert_eq!(c.gid_for(20), 20);
        assert_eq!(c.gid_for(30), c.egid);
        assert_eq!(c.groups(), &[10, 20]);
        let too_many = [0u32; NGROUPS_MAX + 1];
        let mut root = Credentials::new(0, 0);
        assert_eq!(
            root.setgroups(&too_many, true),
            Err(SyscallError::InvalidArgument)
        );
    }
}

//! Process credentials (N-248): real, effective and saved user and group
//! IDs and the supplementary groups, with the Linux rules for changing
//! them. Privilege is effective uid 0 (Linux's CAP_SETUID / CAP_SETGID
//! without a capability model, N-131).

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
            ngroups: 0,
            groups: [0; NGROUPS_MAX],
        }
    }

    pub fn groups(&self) -> &[u32] {
        &self.groups[..self.ngroups]
    }

    fn privileged(&self) -> bool {
        self.euid == 0
    }

    /// Whether the process is in group `gid` (effective or supplementary).
    pub fn in_group(&self, gid: u32) -> bool {
        self.egid == gid || self.groups().contains(&gid)
    }

    /// The group ID a permission check should use against a file owned by
    /// `file_gid`: that group if the process belongs to it, otherwise the
    /// effective gid. With `Permissions::can_*` this gives POSIX's
    /// owner/group/other precedence over all the process's groups.
    pub fn gid_for(&self, file_gid: u32) -> u32 {
        if self.in_group(file_gid) {
            file_gid
        } else {
            self.egid
        }
    }

    /// `gid_for` with the real group ID: what access(2) checks against.
    pub fn real_gid_for(&self, file_gid: u32) -> u32 {
        if self.rgid == file_gid || self.groups().contains(&file_gid) {
            file_gid
        } else {
            self.rgid
        }
    }

    /// setresuid(r, e, s): `UNCHANGED` keeps a value. Unprivileged, each
    /// new value must be one of the current real, effective or saved IDs.
    pub fn setresuid(&mut self, r: u32, e: u32, s: u32) -> Result<(), SyscallError> {
        let current = [self.ruid, self.euid, self.suid];
        if !self.privileged()
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
        Ok(())
    }

    /// setresgid(r, e, s), with the same rules over the group IDs (and
    /// privilege still decided by the effective uid).
    pub fn setresgid(&mut self, r: u32, e: u32, s: u32) -> Result<(), SyscallError> {
        let current = [self.rgid, self.egid, self.sgid];
        if !self.privileged()
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
        Ok(())
    }

    /// setreuid(r, e): unprivileged, the real ID may become the real or
    /// effective one, the effective ID any of the three. The saved ID
    /// follows the new effective ID when the real ID is set, or the
    /// effective ID is set to something other than the old real ID.
    pub fn setreuid(&mut self, r: u32, e: u32) -> Result<(), SyscallError> {
        let old = *self;
        if !self.privileged() {
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
        Ok(())
    }

    /// setregid(r, e), the group counterpart of `setreuid`.
    pub fn setregid(&mut self, r: u32, e: u32) -> Result<(), SyscallError> {
        let old = *self;
        if !self.privileged() {
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
        Ok(())
    }

    /// setuid(uid): privileged, sets all three IDs; otherwise `uid` must
    /// be the real or saved ID and only the effective ID changes.
    pub fn setuid(&mut self, uid: u32) -> Result<(), SyscallError> {
        if uid == UNCHANGED {
            return Err(SyscallError::InvalidArgument);
        }
        if self.privileged() {
            self.ruid = uid;
            self.euid = uid;
            self.suid = uid;
        } else if uid == self.ruid || uid == self.suid {
            self.euid = uid;
        } else {
            return Err(SyscallError::OperationNotPermitted);
        }
        Ok(())
    }

    /// setgid(gid), the group counterpart of `setuid`.
    pub fn setgid(&mut self, gid: u32) -> Result<(), SyscallError> {
        if gid == UNCHANGED {
            return Err(SyscallError::InvalidArgument);
        }
        if self.privileged() {
            self.rgid = gid;
            self.egid = gid;
            self.sgid = gid;
        } else if gid == self.rgid || gid == self.sgid {
            self.egid = gid;
        } else {
            return Err(SyscallError::OperationNotPermitted);
        }
        Ok(())
    }

    /// setgroups: privileged only; at most `NGROUPS_MAX` groups.
    pub fn setgroups(&mut self, list: &[u32]) -> Result<(), SyscallError> {
        if !self.privileged() {
            return Err(SyscallError::OperationNotPermitted);
        }
        if list.len() > NGROUPS_MAX {
            return Err(SyscallError::InvalidArgument);
        }
        self.groups[..list.len()].copy_from_slice(list);
        self.ngroups = list.len();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: u32 = UNCHANGED;

    #[test]
    fn root_drops_privilege_for_good_with_setuid() {
        let mut c = Credentials::new(0, 0);
        c.setuid(1000).unwrap();
        assert_eq!((c.ruid, c.euid, c.suid), (1000, 1000, 1000));
        assert_eq!(c.setuid(0), Err(SyscallError::OperationNotPermitted));
    }

    #[test]
    fn seteuid_round_trip_keeps_the_saved_id() {
        // A setuid-root program: real 1000, effective and saved 0.
        let mut c = Credentials::new(1000, 100);
        c.euid = 0;
        c.suid = 0;
        c.setresuid(U, 1000, U).unwrap(); // seteuid(1000)
        assert_eq!((c.ruid, c.euid, c.suid), (1000, 1000, 0));
        c.setresuid(U, 0, U).unwrap(); // back through the saved ID
        assert_eq!(c.euid, 0);
    }

    #[test]
    fn unprivileged_cannot_take_new_ids() {
        let mut c = Credentials::new(1000, 100);
        assert_eq!(
            c.setresuid(U, 0, U),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(
            c.setresgid(U, 0, U),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(
            c.setreuid(2000, U),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(c.setgroups(&[5]), Err(SyscallError::OperationNotPermitted));
        assert_eq!(c, Credentials::new(1000, 100));
        // Values it already holds are allowed.
        c.setresuid(1000, 1000, 1000).unwrap();
    }

    #[test]
    fn setreuid_updates_the_saved_id_like_linux() {
        let mut c = Credentials::new(0, 0);
        c.setreuid(U, 1000).unwrap(); // effective != old real: saved follows
        assert_eq!((c.ruid, c.euid, c.suid), (0, 1000, 1000));
        let mut c = Credentials::new(0, 0);
        c.setreuid(1000, 1000).unwrap();
        assert_eq!((c.ruid, c.euid, c.suid), (1000, 1000, 1000));
    }

    #[test]
    fn supplementary_groups_grant_group_access() {
        let mut c = Credentials::new(0, 0);
        c.setgroups(&[10, 20]).unwrap();
        c.setresuid(1000, 1000, 1000).unwrap();
        assert!(c.in_group(20));
        assert_eq!(c.gid_for(20), 20);
        assert_eq!(c.gid_for(30), c.egid);
        assert_eq!(c.groups(), &[10, 20]);
        let too_many = [0u32; NGROUPS_MAX + 1];
        let mut root = Credentials::new(0, 0);
        assert_eq!(
            root.setgroups(&too_many),
            Err(SyscallError::InvalidArgument)
        );
    }
}

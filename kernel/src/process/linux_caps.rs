//! Linux capabilities (ADR 0020): the per-thread effective, permitted,
//! inheritable, bounding and ambient sets, the securebits, and Linux's rules
//! for changing them (`kernel/capability.c`, `security/commoncap.c`).
//!
//! These are POSIX/Linux privilege bits, checked where Linux checks them.
//! They are unrelated to the kernel's object capabilities (`crate::cap`),
//! which name and authorise kernel objects; nothing here is called
//! `Capability`.
//!
//! Everything in this module is pure: the system calls read the calling
//! thread's [`CapSets`], apply one of these transitions and store the result.

use super::super::syscall::SyscallError;

pub const CAP_CHOWN: u32 = 0;
pub const CAP_DAC_OVERRIDE: u32 = 1;
pub const CAP_DAC_READ_SEARCH: u32 = 2;
pub const CAP_FOWNER: u32 = 3;
pub const CAP_FSETID: u32 = 4;
pub const CAP_KILL: u32 = 5;
pub const CAP_SETGID: u32 = 6;
pub const CAP_SETUID: u32 = 7;
pub const CAP_SETPCAP: u32 = 8;
pub const CAP_LINUX_IMMUTABLE: u32 = 9;
pub const CAP_NET_BIND_SERVICE: u32 = 10;
pub const CAP_NET_BROADCAST: u32 = 11;
pub const CAP_NET_ADMIN: u32 = 12;
pub const CAP_NET_RAW: u32 = 13;
pub const CAP_IPC_LOCK: u32 = 14;
pub const CAP_IPC_OWNER: u32 = 15;
pub const CAP_SYS_MODULE: u32 = 16;
pub const CAP_SYS_RAWIO: u32 = 17;
pub const CAP_SYS_CHROOT: u32 = 18;
pub const CAP_SYS_PTRACE: u32 = 19;
pub const CAP_SYS_PACCT: u32 = 20;
pub const CAP_SYS_ADMIN: u32 = 21;
pub const CAP_SYS_BOOT: u32 = 22;
pub const CAP_SYS_NICE: u32 = 23;
pub const CAP_SYS_RESOURCE: u32 = 24;
pub const CAP_SYS_TIME: u32 = 25;
pub const CAP_SYS_TTY_CONFIG: u32 = 26;
pub const CAP_MKNOD: u32 = 27;
pub const CAP_LEASE: u32 = 28;
pub const CAP_AUDIT_WRITE: u32 = 29;
pub const CAP_AUDIT_CONTROL: u32 = 30;
pub const CAP_SETFCAP: u32 = 31;
pub const CAP_MAC_OVERRIDE: u32 = 32;
pub const CAP_MAC_ADMIN: u32 = 33;
pub const CAP_SYSLOG: u32 = 34;
pub const CAP_WAKE_ALARM: u32 = 35;
pub const CAP_BLOCK_SUSPEND: u32 = 36;
pub const CAP_AUDIT_READ: u32 = 37;
pub const CAP_PERFMON: u32 = 38;
pub const CAP_BPF: u32 = 39;
pub const CAP_CHECKPOINT_RESTORE: u32 = 40;
/// The highest capability number (`/proc/sys/kernel/cap_last_cap`).
pub const CAP_LAST_CAP: u32 = CAP_CHECKPOINT_RESTORE;

const VALID_MASK: u64 = (1 << (CAP_LAST_CAP + 1)) - 1;

/// Whether `cap` names a capability (Linux's `cap_valid`). It takes the
/// raw system call argument, so a negative number, sign-extended, is not one.
pub const fn cap_valid(cap: u64) -> bool {
    cap <= CAP_LAST_CAP as u64
}

/// A set of Linux capabilities: bit N is capability N. Only the bits of
/// existing capabilities can be set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinuxCaps(u64);

impl LinuxCaps {
    pub const EMPTY: Self = Self(0);
    pub const FULL: Self = Self(VALID_MASK);

    /// The set with these bits, without those of capabilities that do not
    /// exist (Linux's `mk_kernel_cap`).
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits & VALID_MASK)
    }

    /// The set from the two 32-bit words of the capget/capset ABI.
    pub const fn from_words(low: u32, high: u32) -> Self {
        Self::from_bits(low as u64 | (high as u64) << 32)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    /// The set holding only `cap` (which must be valid).
    pub const fn of(cap: u32) -> Self {
        Self::from_bits(1 << cap)
    }

    pub const fn has(self, cap: u32) -> bool {
        cap <= CAP_LAST_CAP && self.0 & (1 << cap) != 0
    }

    pub const fn with(self, cap: u32) -> Self {
        Self::from_bits(self.0 | 1 << cap)
    }

    pub const fn without_cap(self, cap: u32) -> Self {
        Self(self.0 & !(1 << cap))
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The capabilities in `self` that are not in `other`.
    pub const fn minus(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub const fn is_subset_of(self, other: Self) -> bool {
        self.0 & !other.0 == 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// The capabilities file access by uid 0 historically meant, which follow
/// the filesystem uid (Linux's `CAP_FS_SET`: `CAP_FS_MASK` and
/// CAP_LINUX_IMMUTABLE).
pub const FS_SET: LinuxCaps = LinuxCaps::from_bits(
    1 << CAP_CHOWN
        | 1 << CAP_MKNOD
        | 1 << CAP_DAC_OVERRIDE
        | 1 << CAP_DAC_READ_SEARCH
        | 1 << CAP_FOWNER
        | 1 << CAP_FSETID
        | 1 << CAP_MAC_OVERRIDE
        | 1 << CAP_LINUX_IMMUTABLE,
);

// The securebits (`<linux/securebits.h>`): each setting has a bit and, one
// above it, a lock that makes the setting immutable.
pub const SECURE_NOROOT: u32 = 0;
pub const SECURE_NO_SETUID_FIXUP: u32 = 2;
pub const SECURE_KEEP_CAPS: u32 = 4;
pub const SECURE_KEEP_CAPS_LOCKED: u32 = 5;
pub const SECURE_NO_CAP_AMBIENT_RAISE: u32 = 6;
pub const SECURE_EXEC_RESTRICT_FILE: u32 = 8;
pub const SECURE_EXEC_DENY_INTERACTIVE: u32 = 10;

const fn secbit(bit: u32) -> u32 {
    1 << bit
}

pub const SECURE_ALL_BITS: u32 = secbit(SECURE_NOROOT)
    | secbit(SECURE_NO_SETUID_FIXUP)
    | secbit(SECURE_KEEP_CAPS)
    | secbit(SECURE_NO_CAP_AMBIENT_RAISE)
    | secbit(SECURE_EXEC_RESTRICT_FILE)
    | secbit(SECURE_EXEC_DENY_INTERACTIVE);
pub const SECURE_ALL_LOCKS: u32 = SECURE_ALL_BITS << 1;
/// Settings a thread may change without CAP_SETPCAP: they are advice to
/// interpreters, which the kernel does not enforce.
pub const SECURE_ALL_UNPRIVILEGED: u32 =
    secbit(SECURE_EXEC_RESTRICT_FILE) | secbit(SECURE_EXEC_DENY_INTERACTIVE);

// prctl options (`<linux/prctl.h>`).
pub const PR_GET_KEEPCAPS: u32 = 7;
pub const PR_SET_KEEPCAPS: u32 = 8;
pub const PR_CAPBSET_READ: u32 = 23;
pub const PR_CAPBSET_DROP: u32 = 24;
pub const PR_GET_SECUREBITS: u32 = 27;
pub const PR_SET_SECUREBITS: u32 = 28;
pub const PR_CAP_AMBIENT: u32 = 47;
pub const PR_CAP_AMBIENT_IS_SET: u64 = 1;
pub const PR_CAP_AMBIENT_RAISE: u64 = 2;
pub const PR_CAP_AMBIENT_LOWER: u64 = 3;
pub const PR_CAP_AMBIENT_CLEAR_ALL: u64 = 4;

/// The user IDs the UID-change rules look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Uids {
    pub ruid: u32,
    pub euid: u32,
    pub suid: u32,
    pub fsuid: u32,
}

impl Uids {
    const fn any_root(&self) -> bool {
        self.ruid == 0 || self.euid == 0 || self.suid == 0
    }
}

/// A file's capabilities (the `security.capability` attribute), as exec
/// uses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileCaps {
    pub permitted: LinuxCaps,
    pub inheritable: LinuxCaps,
    /// The effective bit: the program runs with pE' = pP'.
    pub effective: bool,
}

/// What exec needs to know besides the old and new IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecContext {
    /// The program's capabilities, if it has any.
    pub file: Option<FileCaps>,
    /// The thread is under no_new_privs.
    pub no_new_privs: bool,
    /// Traced by a tracer without CAP_SYS_PTRACE, or the filesystem state
    /// (cwd, root) is shared with a process that is not exec'ing (Linux's
    /// LSM_UNSAFE_PTRACE without `ptracer_capable`, and LSM_UNSAFE_SHARE).
    pub unsafe_exec: bool,
    /// The new effective gid is the old filesystem gid or a supplementary
    /// group (Linux's `in_group_p(new->egid)`).
    pub egid_in_groups: bool,
}

/// The result of exec for the capability sets and the effective IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecOutcome {
    pub caps: CapSets,
    pub euid: u32,
    pub egid: u32,
    /// AT_SECURE: the program runs with privilege its invoker did not have.
    pub secure: bool,
    /// The permitted set grew: the process becomes non-dumpable and the
    /// dangerous personality flags are cleared.
    pub gained: bool,
}

/// A thread's Linux capability state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapSets {
    pub effective: LinuxCaps,
    pub permitted: LinuxCaps,
    pub inheritable: LinuxCaps,
    pub bounding: LinuxCaps,
    pub ambient: LinuxCaps,
    pub securebits: u32,
}

impl Default for CapSets {
    fn default() -> Self {
        Self::ROOT
    }
}

impl CapSets {
    /// The first process's state, and every kernel-created thread's (Linux's
    /// `init_cred`): all capabilities permitted, effective and in the
    /// bounding set, none inheritable or ambient.
    pub const ROOT: Self = Self {
        effective: LinuxCaps::FULL,
        permitted: LinuxCaps::FULL,
        inheritable: LinuxCaps::EMPTY,
        bounding: LinuxCaps::FULL,
        ambient: LinuxCaps::EMPTY,
        securebits: 0,
    };

    /// No capabilities at all, with the full bounding set: what a process
    /// started directly as a non-root user has.
    pub const NONE: Self = Self {
        effective: LinuxCaps::EMPTY,
        permitted: LinuxCaps::EMPTY,
        inheritable: LinuxCaps::EMPTY,
        bounding: LinuxCaps::FULL,
        ambient: LinuxCaps::EMPTY,
        securebits: 0,
    };

    /// Whether the thread holds `cap` (Linux's `capable`, in the initial
    /// user namespace).
    pub const fn capable(&self, cap: u32) -> bool {
        self.effective.has(cap)
    }

    const fn secure(&self, bit: u32) -> bool {
        self.securebits & secbit(bit) != 0
    }

    /// capset(2) on the calling thread (Linux's `cap_capset`).
    pub fn capset(
        &mut self,
        effective: LinuxCaps,
        permitted: LinuxCaps,
        inheritable: LinuxCaps,
    ) -> Result<(), SyscallError> {
        let old = *self;
        // Without CAP_SETPCAP, the inheritable set may only take what the
        // thread already has.
        if !old.capable(CAP_SETPCAP)
            && !inheritable.is_subset_of(old.inheritable.union(old.permitted))
        {
            return Err(SyscallError::OperationNotPermitted);
        }
        // Never anything outside the bounding set.
        if !inheritable.is_subset_of(old.inheritable.union(old.bounding)) {
            return Err(SyscallError::OperationNotPermitted);
        }
        if !permitted.is_subset_of(old.permitted) || !effective.is_subset_of(permitted) {
            return Err(SyscallError::OperationNotPermitted);
        }
        self.effective = effective;
        self.permitted = permitted;
        self.inheritable = inheritable;
        // Ambient capabilities must stay permitted and inheritable.
        self.ambient = old.ambient.intersect(permitted.intersect(inheritable));
        Ok(())
    }

    /// The fixup after setuid, setreuid or setresuid (Linux's
    /// `cap_emulate_setxuid`, unless SECBIT_NO_SETUID_FIXUP).
    pub fn fix_setuid(&mut self, old: Uids, new: Uids) {
        if self.secure(SECURE_NO_SETUID_FIXUP) {
            return;
        }
        if old.any_root() && !new.any_root() {
            if !self.secure(SECURE_KEEP_CAPS) {
                self.permitted = LinuxCaps::EMPTY;
                self.effective = LinuxCaps::EMPTY;
            }
            // Programs from before ambient capabilities expect setresuid to
            // a non-root user, then exec, to drop everything.
            self.ambient = LinuxCaps::EMPTY;
        }
        if old.euid == 0 && new.euid != 0 {
            self.effective = LinuxCaps::EMPTY;
        }
        if old.euid != 0 && new.euid == 0 {
            self.effective = self.permitted;
        }
    }

    /// The fixup after setfsuid (Linux's `cap_task_fix_setuid` for
    /// LSM_SETID_FS): leaving fsuid 0 drops the filesystem capabilities
    /// from the effective set, reaching it raises them from the permitted set.
    pub fn fix_setfsuid(&mut self, old_fsuid: u32, new_fsuid: u32) {
        if self.secure(SECURE_NO_SETUID_FIXUP) {
            return;
        }
        if old_fsuid == 0 && new_fsuid != 0 {
            self.effective = self.effective.minus(FS_SET);
        }
        if old_fsuid != 0 && new_fsuid == 0 {
            self.effective = self.effective.union(self.permitted.intersect(FS_SET));
        }
    }

    /// The capability prctl options (Linux's `cap_task_prctl`). `None` for
    /// an option that is not one of them; otherwise prctl's return value.
    pub fn prctl(
        &mut self,
        option: u32,
        arg2: u64,
        arg3: u64,
        arg4: u64,
        arg5: u64,
    ) -> Option<Result<usize, SyscallError>> {
        use SyscallError::{InvalidArgument as EINVAL, OperationNotPermitted as EPERM};
        let r = match option {
            PR_CAPBSET_READ => {
                if !cap_valid(arg2) {
                    Err(EINVAL)
                } else {
                    Ok(self.bounding.has(arg2 as u32) as usize)
                }
            }
            PR_CAPBSET_DROP => {
                if !self.capable(CAP_SETPCAP) {
                    Err(EPERM)
                } else if !cap_valid(arg2) {
                    Err(EINVAL)
                } else {
                    self.bounding = self.bounding.without_cap(arg2 as u32);
                    Ok(0)
                }
            }
            PR_SET_SECUREBITS => self.set_securebits(arg2).map(|()| 0),
            PR_GET_SECUREBITS => Ok(self.securebits as usize),
            PR_GET_KEEPCAPS => Ok(self.secure(SECURE_KEEP_CAPS) as usize),
            PR_SET_KEEPCAPS => {
                if arg2 > 1 {
                    Err(EINVAL)
                } else if self.secure(SECURE_KEEP_CAPS_LOCKED) {
                    Err(EPERM)
                } else {
                    if arg2 == 1 {
                        self.securebits |= secbit(SECURE_KEEP_CAPS);
                    } else {
                        self.securebits &= !secbit(SECURE_KEEP_CAPS);
                    }
                    Ok(0)
                }
            }
            PR_CAP_AMBIENT => self.cap_ambient(arg2, arg3, arg4, arg5),
            _ => return None,
        };
        Some(r)
    }

    fn set_securebits(&mut self, arg: u64) -> Result<(), SyscallError> {
        let old = self.securebits as u64;
        let all_locks = SECURE_ALL_LOCKS as u64;
        // No change to a locked setting, no unlocking, no unknown bits.
        if ((old & all_locks) >> 1) & (old ^ arg) != 0
            || old & all_locks & !arg != 0
            || arg & !(all_locks | SECURE_ALL_BITS as u64) != 0
        {
            return Err(SyscallError::OperationNotPermitted);
        }
        if !self.capable(CAP_SETPCAP) {
            let unprivileged = (SECURE_ALL_UNPRIVILEGED | SECURE_ALL_UNPRIVILEGED << 1) as u64;
            let changed = old ^ arg;
            // Linux refuses a no-op here, for compatibility.
            if changed == 0 || changed & !unprivileged != 0 {
                return Err(SyscallError::OperationNotPermitted);
            }
        }
        self.securebits = arg as u32;
        Ok(())
    }

    fn cap_ambient(
        &mut self,
        op: u64,
        cap: u64,
        arg4: u64,
        arg5: u64,
    ) -> Result<usize, SyscallError> {
        use SyscallError::{InvalidArgument as EINVAL, OperationNotPermitted as EPERM};
        if op == PR_CAP_AMBIENT_CLEAR_ALL {
            if cap | arg4 | arg5 != 0 {
                return Err(EINVAL);
            }
            self.ambient = LinuxCaps::EMPTY;
            return Ok(0);
        }
        if !cap_valid(cap) || arg4 | arg5 != 0 {
            return Err(EINVAL);
        }
        let cap = cap as u32;
        match op {
            PR_CAP_AMBIENT_IS_SET => Ok(self.ambient.has(cap) as usize),
            PR_CAP_AMBIENT_RAISE => {
                if !self.permitted.has(cap)
                    || !self.inheritable.has(cap)
                    || self.secure(SECURE_NO_CAP_AMBIENT_RAISE)
                {
                    return Err(EPERM);
                }
                self.ambient = self.ambient.with(cap);
                Ok(0)
            }
            PR_CAP_AMBIENT_LOWER => {
                self.ambient = self.ambient.without_cap(cap);
                Ok(0)
            }
            _ => Err(EINVAL),
        }
    }

    /// The capability sets and effective IDs after exec (Linux's
    /// `cap_bprm_creds_from_file` with `get_file_caps`). `old` are the
    /// IDs before exec, `new_euid`/`new_egid` the effective IDs after the
    /// set-user-ID and set-group-ID bits. Err(EPERM) when the program's
    /// effective bit is set and it would not get all of its file-permitted
    /// capabilities (a "capability-dumb" program would run without them).
    pub fn after_exec(
        &self,
        old_ruid: u32,
        old_euid: u32,
        old_rgid: u32,
        new_euid: u32,
        new_egid: u32,
        ctx: ExecContext,
    ) -> Result<ExecOutcome, SyscallError> {
        let old = *self;
        let mut new = old;
        let mut euid = new_euid;
        let mut egid = new_egid;
        let mut effective = false;
        let has_fcap = ctx.file.is_some();

        // pP' = (X & fP) | (pI & fI); the ambient part comes later.
        new.permitted = LinuxCaps::EMPTY;
        if let Some(f) = ctx.file {
            effective = f.effective;
            new.permitted = old
                .bounding
                .intersect(f.permitted)
                .union(old.inheritable.intersect(f.inheritable));
            if f.effective && !f.permitted.is_subset_of(new.permitted) {
                return Err(SyscallError::OperationNotPermitted);
            }
        }

        // Root is emulated: for a root real or effective uid the file's
        // permitted and inheritable sets count as full (unless
        // SECBIT_NOROOT), and a root effective uid sets the effective bit. A
        // set-user-ID-root program with file capabilities, run by someone
        // else, gets only its file capabilities.
        let ruid = old_ruid;
        let suid_root = ruid != 0 && euid == 0;
        let root_rules = !old.secure(SECURE_NOROOT);
        let file_caps_only = has_fcap && suid_root;
        if root_rules && !file_caps_only {
            if euid == 0 || ruid == 0 {
                new.permitted = old.bounding.union(old.inheritable);
            }
            if euid == 0 {
                effective = true;
            }
        }

        let gained = !new.permitted.is_subset_of(old.permitted);
        let id_changed = euid != old_euid || !ctx.egid_in_groups;

        // Traced by an unprivileged tracer, sharing filesystem state, or
        // under no_new_privs: nothing more than before, maybe less.
        if (id_changed || gained) && (ctx.unsafe_exec || ctx.no_new_privs) {
            if !old.capable(CAP_SETUID) || ctx.no_new_privs {
                euid = ruid;
                egid = old_rgid;
            }
            new.permitted = new.permitted.intersect(old.permitted);
        }

        // File capabilities or a changed ID cancel the ambient set.
        if has_fcap || id_changed {
            new.ambient = LinuxCaps::EMPTY;
        }
        new.permitted = new.permitted.union(new.ambient);
        new.effective = if effective {
            new.permitted
        } else {
            new.ambient
        };
        new.securebits &= !secbit(SECURE_KEEP_CAPS);

        let secure = id_changed
            || euid != ruid
            || egid != old_rgid
            || (ruid != 0 && (effective || !new.permitted.is_subset_of(new.ambient)));
        Ok(ExecOutcome {
            caps: new,
            euid,
            egid,
            secure,
            gained,
        })
    }
}

/// The capget/capset header versions (`<linux/capability.h>`).
pub const LINUX_CAPABILITY_VERSION_1: u32 = 0x1998_0330;
pub const LINUX_CAPABILITY_VERSION_2: u32 = 0x2007_1026;
pub const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;

/// How many 32-bit data structs a header version reads or writes, or None
/// for an unknown version (capget/capset then store VERSION_3 in the
/// header and fail with EINVAL; Linux's `cap_validate_magic`).
pub const fn data_words(version: u32) -> Option<usize> {
    match version {
        LINUX_CAPABILITY_VERSION_1 => Some(1),
        LINUX_CAPABILITY_VERSION_2 | LINUX_CAPABILITY_VERSION_3 => Some(2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(list: &[u32]) -> LinuxCaps {
        list.iter().fold(LinuxCaps::EMPTY, |s, &c| s.with(c))
    }

    fn uids(r: u32, e: u32, s: u32) -> Uids {
        Uids {
            ruid: r,
            euid: e,
            suid: s,
            fsuid: e,
        }
    }

    const EPERM: SyscallError = SyscallError::OperationNotPermitted;
    const EINVAL: SyscallError = SyscallError::InvalidArgument;

    #[test]
    fn sets_hold_only_existing_capabilities() {
        assert_eq!(LinuxCaps::FULL.bits(), (1u64 << 41) - 1);
        assert_eq!(LinuxCaps::from_bits(u64::MAX), LinuxCaps::FULL);
        assert_eq!(LinuxCaps::from_words(u32::MAX, u32::MAX), LinuxCaps::FULL);
        assert!(!LinuxCaps::FULL.has(41));
        assert!(cap_valid(40) && !cap_valid(41) && !cap_valid(u64::MAX));
        assert!(FS_SET.has(CAP_LINUX_IMMUTABLE) && FS_SET.has(CAP_MAC_OVERRIDE));
        assert!(!FS_SET.has(CAP_KILL));
    }

    #[test]
    fn capset_follows_the_four_rules() {
        let mut c = CapSets::ROOT;
        // Dropping is always allowed.
        let few = caps(&[CAP_CHOWN, CAP_KILL]);
        c.capset(caps(&[CAP_KILL]), few, LinuxCaps::EMPTY).unwrap();
        assert_eq!(c.permitted, few);
        assert!(!c.capable(CAP_CHOWN));
        // Raising effective within permitted is allowed.
        c.capset(few, few, LinuxCaps::EMPTY).unwrap();
        // Permitted cannot grow.
        assert_eq!(
            c.capset(few, few.with(CAP_SETUID), LinuxCaps::EMPTY),
            Err(EPERM)
        );
        // Effective must be within the new permitted set.
        assert_eq!(
            c.capset(few, caps(&[CAP_KILL]), LinuxCaps::EMPTY),
            Err(EPERM)
        );
        // Without CAP_SETPCAP, inheritable takes only what is permitted or
        // already inheritable.
        assert_eq!(c.capset(few, few, caps(&[CAP_SETUID])), Err(EPERM));
        c.capset(few, few, caps(&[CAP_CHOWN])).unwrap();
        assert_eq!(c.inheritable, caps(&[CAP_CHOWN]));
    }

    #[test]
    fn capset_with_setpcap_is_still_bounded() {
        let mut c = CapSets::ROOT;
        // CAP_SETPCAP may add any inheritable capability in the bounding set.
        c.capset(
            caps(&[CAP_SETPCAP]),
            caps(&[CAP_SETPCAP]),
            caps(&[CAP_SYS_ADMIN]),
        )
        .unwrap();
        assert!(c.inheritable.has(CAP_SYS_ADMIN));
        c.bounding = c.bounding.without_cap(CAP_NET_RAW);
        assert_eq!(
            c.capset(c.effective, c.permitted, c.inheritable.with(CAP_NET_RAW)),
            Err(EPERM)
        );
    }

    #[test]
    fn capset_trims_the_ambient_set() {
        let mut c = CapSets::ROOT;
        let both = caps(&[CAP_NET_RAW, CAP_KILL]);
        c.capset(LinuxCaps::FULL, LinuxCaps::FULL, both).unwrap();
        c.ambient = both;
        // CAP_KILL leaves the inheritable set: it leaves the ambient set.
        c.capset(LinuxCaps::FULL, LinuxCaps::FULL, caps(&[CAP_NET_RAW]))
            .unwrap();
        assert_eq!(c.ambient, caps(&[CAP_NET_RAW]));
        // CAP_NET_RAW leaves the permitted set: gone too.
        let p = LinuxCaps::FULL.without_cap(CAP_NET_RAW);
        c.capset(p, p, caps(&[CAP_NET_RAW])).unwrap();
        assert!(c.ambient.is_empty());
    }

    #[test]
    fn root_dropping_every_root_uid_loses_everything() {
        let mut c = CapSets::ROOT;
        c.ambient = caps(&[CAP_KILL]);
        c.inheritable = caps(&[CAP_KILL]);
        c.fix_setuid(uids(0, 0, 0), uids(1000, 1000, 1000));
        assert!(c.permitted.is_empty() && c.effective.is_empty() && c.ambient.is_empty());
        assert_eq!(c.bounding, LinuxCaps::FULL);
    }

    #[test]
    fn keep_caps_keeps_permitted_but_not_effective_or_ambient() {
        let mut c = CapSets::ROOT;
        c.securebits = secbit(SECURE_KEEP_CAPS);
        c.ambient = caps(&[CAP_KILL]);
        c.fix_setuid(uids(0, 0, 0), uids(1000, 1000, 1000));
        assert_eq!(c.permitted, LinuxCaps::FULL);
        assert!(c.effective.is_empty(), "the euid left 0");
        assert!(c.ambient.is_empty());
    }

    #[test]
    fn seteuid_round_trip_moves_the_effective_set() {
        let mut c = CapSets::ROOT;
        // seteuid(1000) keeps the saved uid 0: permitted stays.
        c.fix_setuid(uids(0, 0, 0), uids(0, 1000, 0));
        assert!(c.effective.is_empty());
        assert_eq!(c.permitted, LinuxCaps::FULL);
        c.fix_setuid(uids(0, 1000, 0), uids(0, 0, 0));
        assert_eq!(c.effective, LinuxCaps::FULL);
    }

    #[test]
    fn no_setuid_fixup_leaves_the_sets() {
        let mut c = CapSets::ROOT;
        c.securebits = secbit(SECURE_NO_SETUID_FIXUP);
        c.fix_setuid(uids(0, 0, 0), uids(1000, 1000, 1000));
        assert_eq!(
            c,
            CapSets {
                securebits: c.securebits,
                ..CapSets::ROOT
            }
        );
        c.fix_setfsuid(0, 1000);
        assert_eq!(c.effective, LinuxCaps::FULL);
    }

    #[test]
    fn fsuid_moves_the_filesystem_capabilities() {
        let mut c = CapSets::ROOT;
        c.fix_setfsuid(0, 1000);
        assert_eq!(c.effective, LinuxCaps::FULL.minus(FS_SET));
        assert!(c.capable(CAP_KILL) && !c.capable(CAP_DAC_OVERRIDE));
        c.fix_setfsuid(1000, 0);
        assert_eq!(c.effective, LinuxCaps::FULL);
        // Raised only from the permitted set.
        c.permitted = caps(&[CAP_CHOWN]);
        c.effective = LinuxCaps::EMPTY;
        c.fix_setfsuid(1000, 0);
        assert_eq!(c.effective, caps(&[CAP_CHOWN]));
    }

    #[test]
    fn bounding_set_read_and_drop() {
        let mut c = CapSets::ROOT;
        assert_eq!(
            c.prctl(PR_CAPBSET_READ, CAP_NET_RAW as u64, 0, 0, 0),
            Some(Ok(1))
        );
        assert_eq!(c.prctl(PR_CAPBSET_READ, 41, 0, 0, 0), Some(Err(EINVAL)));
        assert_eq!(
            c.prctl(PR_CAPBSET_DROP, CAP_NET_RAW as u64, 0, 0, 0),
            Some(Ok(0))
        );
        assert_eq!(
            c.prctl(PR_CAPBSET_READ, CAP_NET_RAW as u64, 0, 0, 0),
            Some(Ok(0))
        );
        assert_eq!(
            c.prctl(PR_CAPBSET_DROP, u64::MAX, 0, 0, 0),
            Some(Err(EINVAL))
        );
        // Without CAP_SETPCAP, EPERM comes first, even for a bad number.
        c.effective = c.effective.without_cap(CAP_SETPCAP);
        assert_eq!(c.prctl(PR_CAPBSET_DROP, 41, 0, 0, 0), Some(Err(EPERM)));
        assert_eq!(c.prctl(1, 0, 0, 0, 0), None, "not a capability option");
    }

    #[test]
    fn keepcaps_option() {
        let mut c = CapSets::ROOT;
        assert_eq!(c.prctl(PR_GET_KEEPCAPS, 0, 0, 0, 0), Some(Ok(0)));
        assert_eq!(c.prctl(PR_SET_KEEPCAPS, 2, 0, 0, 0), Some(Err(EINVAL)));
        assert_eq!(c.prctl(PR_SET_KEEPCAPS, 1, 0, 0, 0), Some(Ok(0)));
        assert_eq!(c.prctl(PR_GET_KEEPCAPS, 0, 0, 0, 0), Some(Ok(1)));
        assert_eq!(c.securebits, secbit(SECURE_KEEP_CAPS));
        c.securebits |= secbit(SECURE_KEEP_CAPS_LOCKED);
        assert_eq!(c.prctl(PR_SET_KEEPCAPS, 0, 0, 0, 0), Some(Err(EPERM)));
    }

    #[test]
    fn securebits_locks_and_privilege() {
        let mut c = CapSets::ROOT;
        let noroot = secbit(SECURE_NOROOT) as u64;
        let noroot_locked = noroot | secbit(SECURE_NOROOT + 1) as u64;
        assert_eq!(
            c.prctl(PR_SET_SECUREBITS, 1 << 12, 0, 0, 0),
            Some(Err(EPERM))
        );
        assert_eq!(
            c.prctl(PR_SET_SECUREBITS, noroot_locked, 0, 0, 0),
            Some(Ok(0))
        );
        assert_eq!(c.prctl(PR_GET_SECUREBITS, 0, 0, 0, 0), Some(Ok(3)));
        // A locked setting cannot change, and a lock cannot be undone.
        assert_eq!(
            c.prctl(PR_SET_SECUREBITS, noroot_locked & !noroot, 0, 0, 0),
            Some(Err(EPERM))
        );
        assert_eq!(
            c.prctl(PR_SET_SECUREBITS, noroot, 0, 0, 0),
            Some(Err(EPERM))
        );
        // Without CAP_SETPCAP only the interpreter bits may change, and a
        // no-op is refused.
        c.effective = LinuxCaps::EMPTY;
        let restrict = noroot_locked | secbit(SECURE_EXEC_RESTRICT_FILE) as u64;
        assert_eq!(
            c.prctl(PR_SET_SECUREBITS, noroot_locked, 0, 0, 0),
            Some(Err(EPERM))
        );
        assert_eq!(c.prctl(PR_SET_SECUREBITS, restrict, 0, 0, 0), Some(Ok(0)));
        let keep = restrict | secbit(SECURE_KEEP_CAPS) as u64;
        assert_eq!(c.prctl(PR_SET_SECUREBITS, keep, 0, 0, 0), Some(Err(EPERM)));
    }

    #[test]
    fn ambient_raise_needs_permitted_and_inheritable() {
        let mut c = CapSets::ROOT;
        let raw = CAP_NET_RAW as u64;
        assert_eq!(
            c.prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, raw, 0, 0),
            Some(Err(EPERM))
        );
        c.inheritable = caps(&[CAP_NET_RAW]);
        assert_eq!(
            c.prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, raw, 0, 0),
            Some(Ok(0))
        );
        assert_eq!(
            c.prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, raw, 0, 0),
            Some(Ok(1))
        );
        assert_eq!(
            c.prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_LOWER, raw, 0, 0),
            Some(Ok(0))
        );
        assert!(c.ambient.is_empty());
        c.securebits = secbit(SECURE_NO_CAP_AMBIENT_RAISE);
        assert_eq!(
            c.prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, raw, 0, 0),
            Some(Err(EPERM))
        );
        assert_eq!(
            c.prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, raw, 1, 0),
            Some(Err(EINVAL))
        );
        assert_eq!(
            c.prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, 41, 0, 0),
            Some(Err(EINVAL))
        );
        assert_eq!(c.prctl(PR_CAP_AMBIENT, 9, raw, 0, 0), Some(Err(EINVAL)));
        assert_eq!(
            c.prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 1, 0, 0),
            Some(Err(EINVAL))
        );
        assert_eq!(
            c.prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0),
            Some(Ok(0))
        );
    }

    fn plain() -> ExecContext {
        ExecContext {
            egid_in_groups: true,
            ..ExecContext::default()
        }
    }

    #[test]
    fn root_keeps_everything_across_exec() {
        let c = CapSets::ROOT;
        let out = c.after_exec(0, 0, 0, 0, 0, plain()).unwrap();
        assert_eq!(out.caps.permitted, LinuxCaps::FULL);
        assert_eq!(out.caps.effective, LinuxCaps::FULL);
        assert!(!out.secure && !out.gained);
    }

    #[test]
    fn a_user_exec_drops_what_is_not_ambient() {
        // A non-root thread that kept permitted capabilities (KEEP_CAPS)
        // loses them at exec, except the ambient ones; KEEP_CAPS goes.
        let mut c = CapSets::NONE;
        c.permitted = caps(&[CAP_NET_RAW, CAP_KILL]);
        c.inheritable = caps(&[CAP_NET_RAW]);
        c.ambient = caps(&[CAP_NET_RAW]);
        c.securebits = secbit(SECURE_KEEP_CAPS);
        let out = c.after_exec(1000, 1000, 1000, 1000, 1000, plain()).unwrap();
        assert_eq!(out.caps.permitted, caps(&[CAP_NET_RAW]));
        assert_eq!(out.caps.effective, caps(&[CAP_NET_RAW]));
        assert_eq!(out.caps.ambient, caps(&[CAP_NET_RAW]));
        assert_eq!(out.caps.securebits, 0);
        assert!(!out.secure, "ambient alone is not a privilege gain");
    }

    #[test]
    fn setuid_root_program_gets_full_privilege_and_at_secure() {
        let mut c = CapSets::NONE;
        c.ambient = LinuxCaps::EMPTY;
        let out = c.after_exec(1000, 1000, 1000, 0, 1000, plain()).unwrap();
        assert_eq!(out.euid, 0);
        assert_eq!(out.caps.permitted, LinuxCaps::FULL);
        assert_eq!(out.caps.effective, LinuxCaps::FULL);
        assert!(out.secure && out.gained);
        // The bounding set limits what it gets.
        c.bounding = c.bounding.without_cap(CAP_SYS_ADMIN);
        let out = c.after_exec(1000, 1000, 1000, 0, 1000, plain()).unwrap();
        assert!(!out.caps.permitted.has(CAP_SYS_ADMIN));
    }

    #[test]
    fn noroot_turns_off_root_emulation() {
        let mut c = CapSets::ROOT;
        c.securebits = secbit(SECURE_NOROOT);
        let out = c.after_exec(0, 0, 0, 0, 0, plain()).unwrap();
        assert!(out.caps.permitted.is_empty() && out.caps.effective.is_empty());
    }

    #[test]
    fn real_root_only_does_not_raise_effective() {
        // ruid 0, euid 1000 (seteuid before exec): permitted, not effective.
        let mut c = CapSets::ROOT;
        c.effective = LinuxCaps::EMPTY;
        let out = c.after_exec(0, 1000, 0, 1000, 0, plain()).unwrap();
        assert_eq!(out.caps.permitted, LinuxCaps::FULL);
        assert!(out.caps.effective.is_empty());
    }

    #[test]
    fn file_capabilities() {
        let c = CapSets::NONE;
        let f = FileCaps {
            permitted: caps(&[CAP_NET_RAW]),
            inheritable: LinuxCaps::EMPTY,
            effective: true,
        };
        let ctx = ExecContext {
            file: Some(f),
            ..plain()
        };
        let out = c.after_exec(1000, 1000, 1000, 1000, 1000, ctx).unwrap();
        assert_eq!(out.caps.permitted, caps(&[CAP_NET_RAW]));
        assert_eq!(out.caps.effective, caps(&[CAP_NET_RAW]));
        assert!(out.secure && out.gained);
        // Without the effective bit the program raises them itself.
        let ctx = ExecContext {
            file: Some(FileCaps {
                effective: false,
                ..f
            }),
            ..plain()
        };
        let out = c.after_exec(1000, 1000, 1000, 1000, 1000, ctx).unwrap();
        assert!(out.caps.effective.is_empty());
        // A capability-dumb program that would miss one fails.
        let mut bounded = c;
        bounded.bounding = bounded.bounding.without_cap(CAP_NET_RAW);
        let ctx = ExecContext {
            file: Some(f),
            ..plain()
        };
        assert_eq!(
            bounded.after_exec(1000, 1000, 1000, 1000, 1000, ctx),
            Err(EPERM)
        );
    }

    #[test]
    fn file_inheritable_meets_thread_inheritable() {
        let mut c = CapSets::NONE;
        c.inheritable = caps(&[CAP_KILL, CAP_CHOWN]);
        c.ambient = caps(&[CAP_KILL]);
        c.permitted = caps(&[CAP_KILL]);
        let ctx = ExecContext {
            file: Some(FileCaps {
                permitted: LinuxCaps::EMPTY,
                inheritable: caps(&[CAP_CHOWN]),
                effective: false,
            }),
            ..plain()
        };
        let out = c.after_exec(1000, 1000, 1000, 1000, 1000, ctx).unwrap();
        // pI & fI, and file capabilities cancel the ambient set.
        assert_eq!(out.caps.permitted, caps(&[CAP_CHOWN]));
        assert!(out.caps.ambient.is_empty());
    }

    #[test]
    fn setuid_root_with_file_caps_gets_only_the_file_caps() {
        let c = CapSets::NONE;
        let ctx = ExecContext {
            file: Some(FileCaps {
                permitted: caps(&[CAP_NET_RAW]),
                inheritable: LinuxCaps::EMPTY,
                effective: true,
            }),
            ..plain()
        };
        let out = c.after_exec(1000, 1000, 1000, 0, 1000, ctx).unwrap();
        assert_eq!(out.euid, 0);
        assert_eq!(out.caps.permitted, caps(&[CAP_NET_RAW]));
    }

    #[test]
    fn unsafe_exec_gains_nothing() {
        let c = CapSets::NONE;
        let ctx = ExecContext {
            unsafe_exec: true,
            ..plain()
        };
        let out = c.after_exec(1000, 1000, 1000, 0, 1000, ctx).unwrap();
        assert_eq!(out.euid, 1000, "back to the real uid");
        assert!(out.caps.permitted.is_empty());
        let nnp = ExecContext {
            no_new_privs: true,
            ..plain()
        };
        // Root under no_new_privs keeps what it had.
        let out = CapSets::ROOT.after_exec(0, 0, 0, 0, 0, nnp).unwrap();
        assert_eq!(out.caps.permitted, LinuxCaps::FULL);
    }

    #[test]
    fn a_changed_id_cancels_the_ambient_set() {
        let mut c = CapSets::NONE;
        c.permitted = caps(&[CAP_KILL]);
        c.inheritable = caps(&[CAP_KILL]);
        c.ambient = caps(&[CAP_KILL]);
        // Set-group-ID to a group the thread is not in.
        let ctx = ExecContext {
            egid_in_groups: false,
            ..ExecContext::default()
        };
        let out = c.after_exec(1000, 1000, 1000, 1000, 50, ctx).unwrap();
        assert!(out.caps.ambient.is_empty() && out.caps.permitted.is_empty());
        assert!(out.secure);
    }

    #[test]
    fn header_versions() {
        assert_eq!(data_words(LINUX_CAPABILITY_VERSION_1), Some(1));
        assert_eq!(data_words(LINUX_CAPABILITY_VERSION_2), Some(2));
        assert_eq!(data_words(LINUX_CAPABILITY_VERSION_3), Some(2));
        assert_eq!(data_words(0), None);
    }
}

//! User namespaces (ADR 0022 step H4), as Linux 7.3's
//! `kernel/user_namespace.c` and `security/commoncap.c`.
//!
//! A user namespace maps the user and group IDs its processes see onto the
//! kernel's IDs (the initial namespace's), through `uid_map` and `gid_map`
//! written once each in `/proc/<pid>/`. The IDs credentials hold are the
//! kernel's; a capability is held in a user namespace and every one below
//! it ([`UserNs::capable`], Linux's cap_capable). Creating one needs
//! CAP_SYS_ADMIN in the initial namespace unless
//! `/proc/sys/kernel/unprivileged_userns_clone` is 1 (default 0; ADR 0022).

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use spin::{Mutex, Once};

use super::ns::{NsIdent, NsKind};

/// The deepest a user namespace can nest below the initial one (Linux
/// refuses a new one under level 32: ENOSPC).
pub const MAX_LEVEL: u32 = 32;
/// The most extents an ID map holds (UID_GID_MAP_MAX_EXTENTS).
pub const MAX_EXTENTS: usize = 340;
/// The ID an unmapped kernel ID reads as (overflowuid, overflowgid).
pub const OVERFLOW_ID: u32 = 65534;
/// (uid_t)-1, never a valid ID.
pub const INVALID_ID: u32 = u32::MAX;

/// One extent: IDs `first..first+count` inside map to `lower..lower+count`
/// in the kernel's IDs (Linux stores `lower_first` already taken down to
/// the kernel's IDs, as here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub first: u32,
    pub lower: u32,
    pub count: u32,
}

/// An ID map. Empty until written; written once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdMap {
    extents: Vec<Extent>,
}

impl IdMap {
    /// The initial namespace's map: every ID to itself ("0 0 4294967295").
    fn identity() -> Self {
        Self {
            extents: alloc::vec![Extent {
                first: 0,
                lower: 0,
                count: u32::MAX,
            }],
        }
    }

    pub fn extents(&self) -> &[Extent] {
        &self.extents
    }

    pub fn is_empty(&self) -> bool {
        self.extents.is_empty()
    }

    /// The kernel ID `id` maps to (Linux's map_id_down), or None.
    pub fn down(&self, id: u32) -> Option<u32> {
        self.down_range(id, 1)
    }

    /// The kernel ID the range `id..id+count` starts at, if one extent
    /// holds all of it (map_id_range_down).
    pub fn down_range(&self, id: u32, count: u32) -> Option<u32> {
        let last = id.checked_add(count - 1)?;
        self.extents
            .iter()
            .find(|e| id >= e.first && last <= e.first + (e.count - 1))
            .map(|e| e.lower + (id - e.first))
    }

    /// The ID inside the kernel ID `kid` maps from (map_id_up), or None.
    pub fn up(&self, kid: u32) -> Option<u32> {
        self.extents
            .iter()
            .find(|e| kid >= e.lower && kid <= e.lower + (e.count - 1))
            .map(|e| e.first + (kid - e.lower))
    }
}

/// Why a map write was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    /// EINVAL: malformed, overlapping, too many extents, at an offset.
    Invalid,
    /// EPERM: written already, or not permitted.
    Permission,
}

/// A user namespace (Linux's `struct user_namespace`).
#[derive(Debug)]
pub struct UserNs {
    pub ident: NsIdent,
    pub parent: Option<Arc<UserNs>>,
    pub level: u32,
    /// The creator's effective uid and gid (kernel IDs).
    pub owner: u32,
    pub group: u32,
    uid_map: Mutex<IdMap>,
    gid_map: Mutex<IdMap>,
    /// USERNS_SETGROUPS_ALLOWED: setgroups may be used once a gid map
    /// exists ("allow" in `/proc/<pid>/setgroups`); "deny" is permanent.
    setgroups_allowed: AtomicBool,
    /// The creator held CAP_SETFCAP (verify_root_map).
    pub parent_could_setfcap: bool,
}

/// `/proc/sys/kernel/unprivileged_userns_clone` (0 or 1).
pub static UNPRIVILEGED_CLONE: AtomicU32 = AtomicU32::new(0);
/// `/proc/sys/user/max_user_namespaces`: how many user namespaces one user
/// may have made, as Linux's ucounts limit (whose default is half of
/// threads-max, which this matches for a machine of about 4 GiB).
pub static MAX_USER_NAMESPACES: AtomicU32 = AtomicU32::new(DEFAULT_MAX_USER_NAMESPACES);
/// The default of [`MAX_USER_NAMESPACES`].
pub const DEFAULT_MAX_USER_NAMESPACES: u32 = 15_000;
/// How many user namespaces each user (by kernel uid) has made that still
/// exist (Linux's UCOUNT_USER_NAMESPACES).
static COUNTS: Mutex<alloc::collections::BTreeMap<u32, u32>> =
    Mutex::new(alloc::collections::BTreeMap::new());

/// Count one more user namespace for `uid` unless that reaches the limit.
fn charge(uid: u32) -> bool {
    let max = MAX_USER_NAMESPACES.load(Ordering::Relaxed);
    let mut counts = COUNTS.lock();
    let n = counts.entry(uid).or_insert(0);
    if *n >= max {
        if *n == 0 {
            counts.remove(&uid);
        }
        return false;
    }
    *n += 1;
    true
}

/// Count one user namespace of `uid`'s less.
fn uncharge(uid: u32) {
    let mut counts = COUNTS.lock();
    if let Some(n) = counts.get_mut(&uid) {
        *n -= 1;
        if *n == 0 {
            counts.remove(&uid);
        }
    }
}

static INITIAL: Once<Arc<UserNs>> = Once::new();

/// The initial user namespace.
pub fn initial() -> Arc<UserNs> {
    INITIAL
        .call_once(|| {
            Arc::new(UserNs {
                ident: NsIdent::initial(NsKind::User),
                parent: None,
                level: 0,
                owner: 0,
                group: 0,
                uid_map: Mutex::new(IdMap::identity()),
                gid_map: Mutex::new(IdMap::identity()),
                setgroups_allowed: AtomicBool::new(true),
                parent_could_setfcap: true,
            })
        })
        .clone()
}

/// The calling process's user namespace (current_user_ns); the initial
/// one in kernel context.
pub fn current() -> Arc<UserNs> {
    super::current_process().map_or_else(initial, |p| p.user_ns())
}

/// Why a user namespace could not be made (create_user_ns).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateError {
    /// ENOSPC: too deep, or max_user_namespaces reached.
    NoSpace,
    /// EPERM: chrooted, or the creator has no mapping in the parent.
    Permission,
}

impl UserNs {
    /// A new user namespace below `parent` made by a process with
    /// effective `euid`/`egid` (kernel IDs), as create_user_ns: deeper
    /// than [`MAX_LEVEL`] or past the limit ENOSPC, a `chrooted` creator
    /// or one whose IDs `parent` does not map EPERM. The caller gives the
    /// new namespace's credentials their capabilities (set_cred_user_ns).
    pub fn create(
        parent: &Arc<UserNs>,
        euid: u32,
        egid: u32,
        chrooted: bool,
        could_setfcap: bool,
    ) -> Result<Arc<UserNs>, CreateError> {
        if parent.level > MAX_LEVEL {
            return Err(CreateError::NoSpace);
        }
        if !charge(euid) {
            return Err(CreateError::NoSpace);
        }
        if chrooted || parent.uid_up(euid).is_none() || parent.gid_up(egid).is_none() {
            uncharge(euid);
            return Err(CreateError::Permission);
        }
        Ok(Arc::new(UserNs {
            ident: NsIdent::fresh(NsKind::User),
            parent: Some(parent.clone()),
            level: parent.level + 1,
            owner: euid,
            group: egid,
            uid_map: Mutex::new(IdMap::default()),
            gid_map: Mutex::new(IdMap::default()),
            setgroups_allowed: AtomicBool::new(parent.setgroups_allowed.load(Ordering::Acquire)),
            parent_could_setfcap: could_setfcap,
        }))
    }

    pub fn uid_map(&self) -> IdMap {
        self.uid_map.lock().clone()
    }

    pub fn gid_map(&self) -> IdMap {
        self.gid_map.lock().clone()
    }

    /// The kernel uid `uid` inside this namespace is (make_kuid), or None.
    pub fn uid_down(&self, uid: u32) -> Option<u32> {
        self.uid_map.lock().down(uid)
    }

    pub fn gid_down(&self, gid: u32) -> Option<u32> {
        self.gid_map.lock().down(gid)
    }

    /// The uid the kernel uid `kuid` is inside this namespace (from_kuid),
    /// or None.
    pub fn uid_up(&self, kuid: u32) -> Option<u32> {
        self.uid_map.lock().up(kuid)
    }

    pub fn gid_up(&self, kgid: u32) -> Option<u32> {
        self.gid_map.lock().up(kgid)
    }

    /// A uid argument of a set*id call as a kernel uid (make_kuid):
    /// (uid_t)-1, "leave unchanged", stays as it is; one this namespace
    /// does not map is None, which those calls fail with EINVAL.
    pub fn uid_arg(&self, uid: u32) -> Option<u32> {
        if uid == INVALID_ID {
            Some(uid)
        } else {
            self.uid_down(uid)
        }
    }

    pub fn gid_arg(&self, gid: u32) -> Option<u32> {
        if gid == INVALID_ID {
            Some(gid)
        } else {
            self.gid_down(gid)
        }
    }

    /// from_kuid_munged: unmapped reads as [`OVERFLOW_ID`].
    pub fn uid_munged(&self, kuid: u32) -> u32 {
        self.uid_up(kuid).unwrap_or(OVERFLOW_ID)
    }

    pub fn gid_munged(&self, kgid: u32) -> u32 {
        self.gid_up(kgid).unwrap_or(OVERFLOW_ID)
    }

    /// The kernel uid of this namespace's uid 0 (make_kuid(ns, 0)), or
    /// [`INVALID_ID`] when it maps none, which no process has.
    pub fn root_uid(&self) -> u32 {
        self.uid_down(0).unwrap_or(INVALID_ID)
    }

    /// Whether kernel uid `kuid` is uid 0 in this namespace or in one above
    /// it (rootid_owns_currentns): file capabilities stored for that root
    /// apply here.
    pub fn root_owns(&self, kuid: u32) -> bool {
        let mut ns = self;
        loop {
            if ns.uid_up(kuid) == Some(0) {
                return true;
            }
            match &ns.parent {
                Some(parent) => ns = parent,
                None => return false,
            }
        }
    }

    /// Whether `self` is `ancestor` or below it (in_userns).
    pub fn is_in(&self, ancestor: &UserNs) -> bool {
        let mut ns = self;
        while ns.level > ancestor.level {
            match &ns.parent {
                Some(p) => ns = p,
                None => return false,
            }
        }
        ns.ident.id == ancestor.ident.id
    }

    /// Whether credentials in namespace `cred_ns` with effective uid
    /// `euid` and effective capability `has_cap` hold that capability over
    /// `self` (Linux's cap_capable_helper): in their own namespace their
    /// effective set decides; over a namespace below theirs the owner of
    /// the child of theirs on the way down holds every capability, and
    /// so does anyone who holds it in their own namespace; above or
    /// beside it, none.
    pub fn capable(&self, cred_ns: &UserNs, euid: u32, has_cap: bool) -> bool {
        let mut ns = self;
        loop {
            if ns.ident.id == cred_ns.ident.id {
                return has_cap;
            }
            if ns.level <= cred_ns.level {
                return false;
            }
            let Some(parent) = &ns.parent else {
                return false;
            };
            if parent.ident.id == cred_ns.ident.id && ns.owner == euid {
                return true;
            }
            ns = parent;
        }
    }

    /// What `/proc/<pid>/uid_map` (or with `gid` `gid_map`) reads to a
    /// reader in user namespace `reader` (uid_m_show): each extent as
    /// "first lower count" in fields of 10, its lower IDs as `reader` sees
    /// them, or as the parent sees them when `reader` is this namespace;
    /// a lower ID `reader` does not map reads as 4294967295.
    pub fn map_text(&self, gid: bool, reader: &UserNs) -> alloc::string::String {
        let lower_ns = match &self.parent {
            Some(parent) if reader.ident.id == self.ident.id => &**parent,
            _ => reader,
        };
        let map = if gid { self.gid_map() } else { self.uid_map() };
        let mut out = alloc::string::String::new();
        for e in map.extents() {
            let lower = if gid {
                lower_ns.gid_up(e.lower)
            } else {
                lower_ns.uid_up(e.lower)
            }
            .unwrap_or(INVALID_ID);
            out.push_str(&alloc::format!(
                "{:>10} {:>10} {:>10}\n",
                e.first,
                lower,
                e.count
            ));
        }
        out
    }

    /// What `/proc/<pid>/setgroups` reads.
    pub fn setgroups_text(&self) -> &'static str {
        if self.setgroups_allowed.load(Ordering::Acquire) {
            "allow\n"
        } else {
            "deny\n"
        }
    }

    /// A write to `/proc/<pid>/setgroups` (proc_setgroups_write): "allow"
    /// or "deny" with trailing spaces, at offset 0 and under 8 bytes
    /// (EINVAL); "allow" after "deny", or "deny" after the gid map was
    /// written, EPERM.
    pub fn write_setgroups(&self, offset: usize, data: &[u8]) -> Result<usize, MapError> {
        if offset != 0 || data.len() >= 8 {
            return Err(MapError::Invalid);
        }
        let (allow, rest) = if let Some(rest) = data.strip_prefix(b"allow") {
            (true, rest)
        } else if let Some(rest) = data.strip_prefix(b"deny") {
            (false, rest)
        } else {
            return Err(MapError::Invalid);
        };
        if !rest.iter().all(|b| b.is_ascii_whitespace()) {
            return Err(MapError::Invalid);
        }
        // userns_state_mutex: the gid map lock orders this against its write.
        let gid_map = self.gid_map.lock();
        if allow {
            if !self.setgroups_allowed.load(Ordering::Acquire) {
                return Err(MapError::Permission);
            }
        } else {
            if !gid_map.is_empty() {
                return Err(MapError::Permission);
            }
            self.setgroups_allowed.store(false, Ordering::Release);
        }
        Ok(data.len())
    }

    /// Whether setgroups may be used here (userns_may_setgroups): a gid
    /// map exists and setgroups is allowed.
    pub fn may_setgroups(&self) -> bool {
        let gid_map = self.gid_map.lock();
        !gid_map.is_empty() && self.setgroups_allowed.load(Ordering::Acquire)
    }

    /// A write of `data` at `offset` to this namespace's `uid_map` (or
    /// with `gid` its `gid_map`), through a file `opener` opened, by
    /// `writer`, as Linux's proc_uid_map_write, map_write,
    /// new_idmap_permitted and verify_root_map. Most checks are the
    /// opener's (the file's f_cred), so a privileged program a descriptor
    /// is handed to cannot write a map its opener could not. The initial
    /// namespace's maps, and an opener in neither this namespace nor its
    /// parent, EPERM; at an offset or a page or more EINVAL; a second
    /// write, or an opener without CAP_SYS_ADMIN over this namespace,
    /// EPERM; a malformed map EINVAL ([`parse_map`]). Then the map must be
    /// allowed: one ID mapped to the opener's own effective one by the
    /// namespace's owner (for a gid only once setgroups is denied), or
    /// CAP_SETUID or CAP_SETGID over the parent held by both the opener and
    /// the writer; mapping the parent's uid 0 needs the opener's
    /// CAP_SETFCAP (held at creation when writing one's own map). Every
    /// range must be mapped in the parent (EPERM).
    pub fn write_map(
        &self,
        gid: bool,
        opener: &super::creds::Caller,
        writer: &super::creds::Caller,
        offset: usize,
        data: &[u8],
    ) -> Result<usize, MapError> {
        use super::linux_caps::{CAP_SETFCAP, CAP_SETGID, CAP_SETUID, CAP_SYS_ADMIN};
        let Some(parent) = &self.parent else {
            return Err(MapError::Permission);
        };
        let own = opener.userns.ident.id == self.ident.id;
        if !own && opener.userns.ident.id != parent.ident.id {
            return Err(MapError::Permission);
        }
        if offset != 0 || data.len() >= 4096 {
            return Err(MapError::Invalid);
        }
        let mut map = if gid {
            self.gid_map.lock()
        } else {
            self.uid_map.lock()
        };
        if !map.is_empty() || !opener.ns_capable(self, CAP_SYS_ADMIN) {
            return Err(MapError::Permission);
        }
        let mut extents = parse_map(data)?;
        // verify_root_map: the parent's uid 0 mapped.
        if !gid && extents.iter().any(|e| e.lower == 0) {
            let may = if own {
                self.parent_could_setfcap
            } else {
                opener.ns_capable(parent, CAP_SETFCAP)
            };
            if !may {
                return Err(MapError::Permission);
            }
        }
        let down = |id: u32, count: u32| {
            if gid {
                parent.gid_map.lock().down_range(id, count)
            } else {
                parent.uid_map.lock().down_range(id, count)
            }
        };
        let single = extents.len() == 1 && extents[0].count == 1 && self.owner == opener.creds.euid;
        let own_id = single
            && if gid {
                !self.setgroups_allowed.load(Ordering::Acquire)
                    && down(extents[0].lower, 1) == Some(opener.creds.egid)
            } else {
                down(extents[0].lower, 1) == Some(opener.creds.euid)
            };
        let cap = if gid { CAP_SETGID } else { CAP_SETUID };
        let both_capable = opener.ns_capable(parent, cap) && writer.ns_capable(parent, cap);
        if !own_id && !both_capable {
            return Err(MapError::Permission);
        }
        for e in &mut extents {
            e.lower = down(e.lower, e.count).ok_or(MapError::Permission)?;
        }
        map.extents = extents;
        Ok(data.len())
    }
}

impl Drop for UserNs {
    fn drop(&mut self) {
        if self.parent.is_some() {
            uncharge(self.owner);
        }
    }
}

/// Parse a map write (map_write's loop): lines of "first lower count",
/// numbers in decimal with spaces around; neither first nor lower
/// (u32)-1, a count that is not 0 and wraps neither range, no overlap
/// with an earlier line in either range, at most [`MAX_EXTENTS`] lines.
pub fn parse_map(text: &[u8]) -> Result<Vec<Extent>, MapError> {
    let text = core::str::from_utf8(text).map_err(|_| MapError::Invalid)?;
    let mut extents: Vec<Extent> = Vec::new();
    let mut lines = text.split('\n').peekable();
    while let Some(line) = lines.next() {
        // A trailing newline ends the input (next_line becomes NULL).
        if line.is_empty() && lines.peek().is_none() && !extents.is_empty() {
            break;
        }
        let mut words = line.split_ascii_whitespace();
        let num = |w: Option<&str>| -> Result<u32, MapError> {
            w.ok_or(MapError::Invalid)?
                .parse::<u32>()
                .map_err(|_| MapError::Invalid)
        };
        let first = num(words.next())?;
        let lower = num(words.next())?;
        let count = num(words.next())?;
        if words.next().is_some() {
            return Err(MapError::Invalid);
        }
        if first == INVALID_ID || lower == INVALID_ID {
            return Err(MapError::Invalid);
        }
        if first.checked_add(count).is_none_or(|end| end <= first)
            || lower.checked_add(count).is_none_or(|end| end <= lower)
        {
            return Err(MapError::Invalid);
        }
        let new = Extent {
            first,
            lower,
            count,
        };
        let overlap = |a: u32, b: u32, c: u32, d: u32| a <= d && b >= c;
        if extents.iter().any(|e| {
            overlap(e.first, e.first + e.count - 1, first, first + count - 1)
                || overlap(e.lower, e.lower + e.count - 1, lower, lower + count - 1)
        }) {
            return Err(MapError::Invalid);
        }
        if extents.len() == MAX_EXTENTS {
            return Err(MapError::Invalid);
        }
        extents.push(new);
    }
    if extents.is_empty() {
        return Err(MapError::Invalid);
    }
    Ok(extents)
}

/// A child of the initial namespace, made by uid 1000, with `uid_map` and
/// `gid_map` written by the initial namespace's root (tests).
#[cfg(test)]
pub(crate) fn test_child(uid_map: &[u8], gid_map: &[u8]) -> Arc<UserNs> {
    use super::{
        creds::{Caller, Credentials},
        linux_caps::LinuxCaps,
    };
    let init = initial();
    let ns = UserNs::create(&init, 1000, 1000, false, true).unwrap();
    let root = Caller {
        creds: Credentials::new(0, 0),
        caps: LinuxCaps::FULL,
        userns: init,
    };
    ns.write_map(false, &root, &root, 0, uid_map).unwrap();
    ns.write_map(true, &root, &root, 0, gid_map).unwrap();
    ns
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{
        creds::{Caller, Credentials},
        linux_caps::LinuxCaps,
    };

    fn caller(uid: u32, caps: LinuxCaps, ns: &Arc<UserNs>) -> Caller {
        Caller {
            creds: Credentials::new(uid, uid),
            caps,
            userns: ns.clone(),
        }
    }

    #[test]
    fn the_initial_namespace_maps_every_id_to_itself() {
        let init = initial();
        assert_eq!(init.uid_down(1000), Some(1000));
        assert_eq!(init.gid_up(u32::MAX - 1), Some(u32::MAX - 1));
        assert_eq!(init.uid_up(u32::MAX), None);
        assert_eq!(
            init.map_text(false, &init),
            "         0          0 4294967295\n"
        );
    }

    #[test]
    fn creation_follows_create_user_ns() {
        let init = initial();
        let child = UserNs::create(&init, 1000, 1000, false, false).unwrap();
        assert_eq!((child.level, child.owner), (1, 1000));
        assert!(child.uid_map().is_empty());
        // Chrooted, or a creator the parent does not map: EPERM.
        assert_eq!(
            UserNs::create(&init, 1000, 1000, true, false).err(),
            Some(CreateError::Permission)
        );
        assert_eq!(
            UserNs::create(&child, 1000, 1000, false, false).err(),
            Some(CreateError::Permission)
        );
        // Below level 32 only.
        let mut ns = init.clone();
        for _ in 0..=MAX_LEVEL {
            ns = UserNs::create(&ns, 0, 0, false, true).unwrap();
            ns.write_map(
                false,
                &caller(0, LinuxCaps::FULL, &ns),
                &caller(0, LinuxCaps::FULL, &ns),
                0,
                b"0 0 1",
            )
            .unwrap();
            ns.write_setgroups(0, b"deny").unwrap();
            ns.write_map(
                true,
                &caller(0, LinuxCaps::FULL, &ns),
                &caller(0, LinuxCaps::FULL, &ns),
                0,
                b"0 0 1",
            )
            .unwrap();
        }
        assert_eq!(ns.level, MAX_LEVEL + 1);
        assert_eq!(
            UserNs::create(&ns, 0, 0, false, true).err(),
            Some(CreateError::NoSpace)
        );
    }

    #[test]
    fn capabilities_follow_cap_capable() {
        let init = initial();
        let child = UserNs::create(&init, 1000, 1000, false, false).unwrap();
        let grandchild = UserNs::create(&child, 1000, 1000, false, false);
        // The child has no maps yet, so it maps no creator.
        assert!(grandchild.is_err());
        // Full caps in the child are nothing in the initial namespace.
        assert!(!init.capable(&child, 0, true));
        assert!(child.capable(&child, 0, true));
        assert!(!child.capable(&child, 0, false));
        // In the parent: the owner holds everything over it, others what
        // they hold in the parent.
        assert!(child.capable(&init, 1000, false));
        assert!(!child.capable(&init, 1001, false));
        assert!(child.capable(&init, 1001, true));
        // A sibling holds nothing.
        let sibling = UserNs::create(&init, 1000, 1000, false, false).unwrap();
        assert!(!child.capable(&sibling, 1000, true));
        assert!(child.is_in(&init) && !init.is_in(&child) && !child.is_in(&sibling));
    }

    #[test]
    fn parse_map_is_map_writes_parser() {
        assert_eq!(
            parse_map(b"0 1000 1\n").unwrap(),
            [Extent {
                first: 0,
                lower: 1000,
                count: 1
            }]
        );
        assert_eq!(parse_map(b"  0   1000  10  \n1000 0 5").unwrap().len(), 2);
        for bad in [
            &b""[..],
            b"\n",
            b"0 0",
            b"0 0 0",
            b"0 0 1 1",
            b"x 0 1",
            b"4294967295 0 1",
            b"0 4294967295 1",
            b"4294967290 0 10",
            b"0 0 10\n5 100 1",
            b"0 0 10\n100 5 1",
            b"0 0 1\n\n1 1 1",
        ] {
            assert_eq!(parse_map(bad), Err(MapError::Invalid), "{:?}", bad);
        }
        let mut many = alloc::string::String::new();
        for i in 0..MAX_EXTENTS {
            many.push_str(&alloc::format!("{} {} 1\n", i, i));
        }
        assert_eq!(parse_map(many.as_bytes()).unwrap().len(), MAX_EXTENTS);
        many.push_str("999 999 1\n");
        assert_eq!(parse_map(many.as_bytes()), Err(MapError::Invalid));
    }

    #[test]
    fn map_writes_follow_new_idmap_permitted() {
        let init = initial();
        // An unprivileged owner maps exactly itself, once.
        let ns = UserNs::create(&init, 1000, 1000, false, false).unwrap();
        let inside = caller(1000, LinuxCaps::FULL, &ns);
        assert_eq!(
            ns.write_map(false, &inside, &inside, 0, b"0 1001 1"),
            Err(MapError::Permission)
        );
        assert_eq!(
            ns.write_map(false, &inside, &inside, 0, b"0 1000 2"),
            Err(MapError::Permission)
        );
        assert_eq!(
            ns.write_map(false, &inside, &inside, 1, b"0 1000 1"),
            Err(MapError::Invalid)
        );
        assert_eq!(ns.write_map(false, &inside, &inside, 0, b"0 1000 1"), Ok(8));
        assert_eq!(
            ns.write_map(false, &inside, &inside, 0, b"0 1000 1"),
            Err(MapError::Permission)
        );
        assert_eq!(ns.uid_down(0), Some(1000));
        assert_eq!(ns.uid_munged(1001), OVERFLOW_ID);
        // Read from inside, the lower IDs are the parent's; from the
        // parent, the same.
        assert_eq!(
            ns.map_text(false, &ns),
            "         0       1000          1\n"
        );
        assert_eq!(
            ns.map_text(false, &init),
            "         0       1000          1\n"
        );
        // Its own gid needs setgroups denied first; then "allow" is EPERM.
        assert_eq!(
            ns.write_map(true, &inside, &inside, 0, b"0 1000 1"),
            Err(MapError::Permission)
        );
        assert_eq!(ns.setgroups_text(), "allow\n");
        assert_eq!(ns.write_setgroups(0, b"deny"), Ok(4));
        assert_eq!(ns.write_setgroups(0, b"allow"), Err(MapError::Permission));
        assert_eq!(ns.write_map(true, &inside, &inside, 0, b"0 1000 1"), Ok(8));
        assert!(!ns.may_setgroups());
        assert_eq!(ns.write_setgroups(0, b"deny\n"), Err(MapError::Permission));
        assert_eq!(ns.write_setgroups(0, b"maybe"), Err(MapError::Invalid));

        // Root in the parent maps ranges; uid 0 of the parent needs
        // CAP_SETFCAP.
        let ns = UserNs::create(&init, 0, 0, false, false).unwrap();
        let root = caller(0, LinuxCaps::FULL, &init);
        let no_fcap = caller(
            0,
            LinuxCaps::FULL.without_cap(crate::process::linux_caps::CAP_SETFCAP),
            &init,
        );
        assert_eq!(
            ns.write_map(false, &no_fcap, &no_fcap, 0, b"0 0 65536"),
            Err(MapError::Permission)
        );
        assert_eq!(ns.write_map(false, &root, &root, 0, b"0 0 65536"), Ok(9));
        // Unprivileged outsiders, and writers outside ns and its parent.
        let ns = UserNs::create(&init, 0, 0, false, false).unwrap();
        let other = UserNs::create(&init, 0, 0, false, false).unwrap();
        assert_eq!(
            ns.write_map(
                false,
                &caller(0, LinuxCaps::FULL, &other),
                &caller(0, LinuxCaps::FULL, &other),
                0,
                b"0 5 1"
            ),
            Err(MapError::Permission)
        );
        assert_eq!(
            ns.write_map(
                false,
                &caller(5, LinuxCaps::EMPTY, &init),
                &caller(5, LinuxCaps::EMPTY, &init),
                0,
                b"0 5 1"
            ),
            Err(MapError::Permission)
        );
        // The initial namespace's maps cannot be written.
        assert_eq!(
            init.write_map(false, &root, &root, 0, b"0 0 1"),
            Err(MapError::Permission)
        );
        // The opener's credentials decide (CVE-2013-1959): a descriptor an
        // unprivileged owner opened, handed to a privileged writer (a
        // set-uid program's output), maps only what the opener could; and
        // a privileged opener's descriptor written by someone without
        // CAP_SETUID maps no more than that writer could.
        let ns = UserNs::create(&init, 1000, 100, false, false).unwrap();
        let owner = caller(1000, LinuxCaps::EMPTY, &init);
        assert_eq!(
            ns.write_map(false, &owner, &root, 0, b"0 0 4294967295"),
            Err(MapError::Permission)
        );
        assert_eq!(
            ns.write_map(false, &root, &owner, 0, b"0 0 4294967295"),
            Err(MapError::Permission)
        );
        assert_eq!(ns.write_map(false, &owner, &root, 0, b"0 1000 1"), Ok(8));
    }

    /// max_user_namespaces counts per creating user, as Linux's ucounts:
    /// one user at the limit leaves others free, and a dropped namespace
    /// gives its place back.
    #[test]
    fn the_namespace_limit_is_per_user() {
        let init = initial();
        assert!(charge(4_000_001));
        uncharge(4_000_001);
        assert!(!COUNTS.lock().contains_key(&4_000_001));
        let max = MAX_USER_NAMESPACES.load(Ordering::Relaxed);
        COUNTS.lock().insert(4_000_002, max);
        assert_eq!(
            UserNs::create(&init, 4_000_002, 0, false, false).err(),
            Some(CreateError::NoSpace)
        );
        let other = UserNs::create(&init, 4_000_003, 0, false, false).unwrap();
        assert_eq!(COUNTS.lock().get(&4_000_003), Some(&1));
        drop(other);
        assert!(!COUNTS.lock().contains_key(&4_000_003));
        COUNTS.lock().remove(&4_000_002);
    }

    /// IDs cross into and out of a namespace as Linux's make_kuid and
    /// from_kuid_munged: (uid_t)-1 stays "unchanged", an unmapped ID is
    /// refused going in and reads as the overflow ID coming out.
    #[test]
    fn ids_translate_at_the_boundary() {
        let ns = test_child(b"0 1000 1\n1 100000 10", b"0 1000 1");
        assert_eq!(ns.uid_arg(0), Some(1000));
        assert_eq!(ns.uid_arg(5), Some(100_004));
        assert_eq!(ns.uid_arg(11), None);
        assert_eq!(ns.uid_arg(u32::MAX), Some(u32::MAX));
        assert_eq!(ns.gid_arg(1), None);
        assert_eq!(ns.uid_munged(100_009), 10);
        assert_eq!(ns.uid_munged(0), OVERFLOW_ID);
        assert_eq!(ns.gid_munged(1000), 0);
        assert_eq!(ns.root_uid(), 1000);
        assert_eq!(initial().root_uid(), 0);
        let unmapped = UserNs::create(&initial(), 1000, 1000, false, false).unwrap();
        assert_eq!(unmapped.root_uid(), INVALID_ID);
    }

    /// rootid_owns_currentns: the root id is uid 0 here or above.
    #[test]
    fn a_root_id_owns_the_namespaces_it_is_root_of() {
        let ns = test_child(b"0 1000 1", b"0 1000 1");
        assert!(ns.root_owns(1000), "this namespace's root");
        assert!(ns.root_owns(0), "the initial namespace's root");
        assert!(!ns.root_owns(2000));
        assert!(!initial().root_owns(1000));
    }
}

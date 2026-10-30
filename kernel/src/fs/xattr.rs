//! Extended attributes (ADR 0020, decision 6).
//!
//! Names, limits and flags are Linux's (`include/uapi/linux/xattr.h`,
//! `limits.h`), and so are the namespace rules: `fs/xattr.c`'s
//! `xattr_permission` and `xattr_resolve_name`, and the capability module's
//! `cap_inode_setxattr` / `cap_inode_removexattr`. [`XattrSet`] holds one
//! node's attributes: tmpfs and ramfs keep it in the node, BlockFS encodes it
//! in one attribute block per inode (`XattrSet::encode`).

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use crate::{
    error::{FsError, KernelError},
    fs::{permission, Metadata, NodeType, MAY_READ, MAY_WRITE},
    process::{
        creds::Caller,
        linux_caps::{CAP_SETFCAP, CAP_SYS_ADMIN},
    },
};

/// Longest attribute name, prefix included (XATTR_NAME_MAX).
pub const XATTR_NAME_MAX: usize = 255;

/// Largest attribute value (XATTR_SIZE_MAX).
pub const XATTR_SIZE_MAX: usize = 65536;

/// Largest name list `listxattr` returns (XATTR_LIST_MAX).
pub const XATTR_LIST_MAX: usize = 65536;

/// setxattr: fail if the attribute exists (EEXIST).
pub const XATTR_CREATE: u32 = 1;

/// setxattr: fail if the attribute does not exist (ENODATA).
pub const XATTR_REPLACE: u32 = 2;

/// The file capability attribute (step 9 of ADR 0020).
pub const XATTR_NAME_CAPS: &str = "security.capability";

/// The namespaces Linux's handlers serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    User,
    Trusted,
    Security,
    /// POSIX ACLs (`system.posix_acl_*`), which VeridianOS does not keep:
    /// every operation on them is EOPNOTSUPP.
    System,
}

/// The namespace of `name` (Linux's xattr_resolve_name): EOPNOTSUPP for an
/// unknown prefix, EINVAL for a prefix with nothing after it.
pub fn namespace(name: &str) -> Result<Namespace, KernelError> {
    const PREFIXES: [(&str, Namespace); 4] = [
        ("user.", Namespace::User),
        ("trusted.", Namespace::Trusted),
        ("security.", Namespace::Security),
        ("system.", Namespace::System),
    ];
    for (prefix, ns) in PREFIXES {
        if let Some(rest) = name.strip_prefix(prefix) {
            if rest.is_empty() {
                return Err(KernelError::InvalidArgument {
                    name: "xattr name",
                    value: "nothing after the namespace",
                });
            }
            return Ok(ns);
        }
    }
    Err(KernelError::FsError(FsError::NotSupported))
}

/// Whether `value` may be stored as attribute `name` at all, whoever asks:
/// a file capability must be a v2 or v3 value (cap_convert_nscap). The
/// system calls check it before their permission check; the kernel shell's
/// `xattr set` runs it too, so it cannot store a malformed capability.
pub fn check_value(name: &str, value: &[u8]) -> Result<(), KernelError> {
    if name == XATTR_NAME_CAPS && !crate::process::linux_caps::FileCaps::valid_to_write(value) {
        return Err(KernelError::InvalidArgument {
            name: "security.capability",
            value: "not a v2 or v3 file capability",
        });
    }
    Ok(())
}

/// The error for an access the namespace rules refuse: EPERM for a write,
/// ENODATA (as if absent) for a read (xattr_permission_error).
fn refused(write: bool) -> KernelError {
    KernelError::FsError(if write {
        FsError::OperationNotPermitted
    } else {
        FsError::NoData
    })
}

/// Whether `caller` may read (`write` false) or set and remove (`write`
/// true) attribute `name` of a node with `meta`: Linux's xattr_permission,
/// then the capability module's checks on `security.*`.
pub fn check_access(
    caller: &Caller,
    meta: &Metadata,
    name: &str,
    write: bool,
) -> Result<(), KernelError> {
    match namespace(name)? {
        Namespace::System => Err(KernelError::FsError(FsError::NotSupported)),
        // No check from the VFS; writing needs CAP_SYS_ADMIN, except the
        // file capability attribute, which CAP_SETFCAP sets and removes.
        Namespace::Security => {
            let cap = if name == XATTR_NAME_CAPS {
                CAP_SETFCAP
            } else {
                CAP_SYS_ADMIN
            };
            if write && !caller.capable(cap) {
                return Err(KernelError::FsError(FsError::OperationNotPermitted));
            }
            Ok(())
        }
        Namespace::Trusted => {
            if caller.capable(CAP_SYS_ADMIN) {
                Ok(())
            } else {
                Err(refused(write))
            }
        }
        // Only regular files, directories and sockets; in a sticky directory
        // only its owner or CAP_FOWNER may write. Then the file's own
        // permission bits.
        Namespace::User => {
            match meta.node_type {
                NodeType::File | NodeType::Socket => {}
                NodeType::Directory => {
                    if write && meta.permissions.sticky && !caller.owner_or_capable(meta.uid) {
                        return Err(KernelError::FsError(FsError::OperationNotPermitted));
                    }
                }
                _ => return Err(refused(write)),
            }
            let mask = if write { MAY_WRITE } else { MAY_READ };
            if permission(caller, meta, mask) {
                Ok(())
            } else {
                Err(KernelError::FsError(FsError::PermissionDenied))
            }
        }
    }
}

/// Whether `listxattr` shows `name` to `caller`: `trusted.*` only with
/// CAP_SYS_ADMIN, as ext4's and tmpfs's trusted handlers list them.
pub fn listed(caller: &Caller, name: &str) -> bool {
    !name.starts_with("trusted.") || caller.capable(CAP_SYS_ADMIN)
}

/// Magic number of a BlockFS attribute block ("XATR").
const BLOCK_MAGIC: u32 = 0x5254_4158;

/// Bytes before the first entry of an attribute block: magic, count and two
/// reserved bytes.
const BLOCK_HEADER: usize = 8;

/// Bytes before an entry's name: name length, a reserved byte and the
/// value length.
const ENTRY_HEADER: usize = 4;

/// The attributes of one node, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XattrSet {
    entries: BTreeMap<String, Vec<u8>>,
}

impl XattrSet {
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The value of `name`.
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.entries.get(name).map(Vec::as_slice)
    }

    /// The names, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Set `name` to `value` (setxattr's `flags`: XATTR_CREATE fails with
    /// EEXIST if it exists, XATTR_REPLACE with ENODATA if it does not).
    pub fn set(&mut self, name: &str, value: &[u8], flags: u32) -> Result<(), KernelError> {
        // The limits the system calls check before getting here, kept by the
        // set itself so no caller (the kernel shell's `xattr`) can store an
        // entry the encoding's length fields cannot hold (PR #33 review).
        if name.is_empty() || name.len() > XATTR_NAME_MAX {
            return Err(KernelError::InvalidArgument {
                name: "xattr name",
                value: "empty or longer than XATTR_NAME_MAX",
            });
        }
        // What `decode` accepts from disk, so a stored set always reads
        // back.
        if name.contains('\0') {
            return Err(KernelError::InvalidArgument {
                name: "xattr name",
                value: "contains NUL",
            });
        }
        namespace(name)?;
        if value.len() > XATTR_SIZE_MAX {
            return Err(KernelError::InvalidArgument {
                name: "xattr value",
                value: "longer than XATTR_SIZE_MAX",
            });
        }
        let exists = self.entries.contains_key(name);
        if flags & XATTR_CREATE != 0 && exists {
            return Err(KernelError::FsError(FsError::AlreadyExists));
        }
        if flags & XATTR_REPLACE != 0 && !exists {
            return Err(KernelError::FsError(FsError::NoData));
        }
        self.entries.insert(String::from(name), Vec::from(value));
        Ok(())
    }

    /// Remove `name` (ENODATA if absent).
    pub fn remove(&mut self, name: &str) -> Result<(), KernelError> {
        self.entries
            .remove(name)
            .map(drop)
            .ok_or(KernelError::FsError(FsError::NoData))
    }

    /// Drop every attribute.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The bytes `encode` needs.
    pub fn encoded_size(&self) -> usize {
        BLOCK_HEADER
            + self
                .entries
                .iter()
                .map(|(n, v)| (ENTRY_HEADER + n.len() + v.len()).next_multiple_of(4))
                .sum::<usize>()
    }

    /// Encode into `buf` (a BlockFS attribute block): ENOSPC if it does
    /// not fit. Zeros follow the last entry.
    pub fn encode(&self, buf: &mut [u8]) -> Result<(), KernelError> {
        if self.encoded_size() > buf.len() {
            return Err(KernelError::FsError(FsError::NoSpace));
        }
        // A name length is one byte and a value length two: anything longer
        // would be written truncated and corrupt the block.
        if self.entries.len() > usize::from(u16::MAX)
            || self
                .entries
                .iter()
                .any(|(n, v)| n.len() > usize::from(u8::MAX) || v.len() > usize::from(u16::MAX))
        {
            return Err(KernelError::FsError(FsError::NoSpace));
        }
        buf.fill(0);
        buf[0..4].copy_from_slice(&BLOCK_MAGIC.to_le_bytes());
        buf[4..6].copy_from_slice(&(self.entries.len() as u16).to_le_bytes());
        let mut at = BLOCK_HEADER;
        for (name, value) in &self.entries {
            buf[at] = name.len() as u8;
            buf[at + 2..at + 4].copy_from_slice(&(value.len() as u16).to_le_bytes());
            let name_at = at + ENTRY_HEADER;
            buf[name_at..name_at + name.len()].copy_from_slice(name.as_bytes());
            let value_at = name_at + name.len();
            buf[value_at..value_at + value.len()].copy_from_slice(value);
            at += (ENTRY_HEADER + name.len() + value.len()).next_multiple_of(4);
        }
        Ok(())
    }

    /// Decode an attribute block. A block that is not one, or whose
    /// entries run past its end, is corrupt (EIO).
    pub fn decode(buf: &[u8]) -> Result<Self, KernelError> {
        let corrupt = KernelError::FsError(FsError::CorruptedData);
        if buf.len() < BLOCK_HEADER
            || u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) != BLOCK_MAGIC
        {
            return Err(corrupt);
        }
        let count = u16::from_le_bytes([buf[4], buf[5]]) as usize;
        let mut set = Self::new();
        let mut at = BLOCK_HEADER;
        for _ in 0..count {
            let header = buf.get(at..at + ENTRY_HEADER).ok_or(corrupt)?;
            let name_len = header[0] as usize;
            let value_len = u16::from_le_bytes([header[2], header[3]]) as usize;
            let name_at = at + ENTRY_HEADER;
            let name = buf
                .get(name_at..name_at + name_len)
                .and_then(|n| core::str::from_utf8(n).ok())
                // Only a name `set` could have stored: a NUL would split
                // one listxattr entry in two (PR #33 review).
                .filter(|n| !n.contains('\0') && namespace(n).is_ok())
                .ok_or(corrupt)?;
            let value = buf
                .get(name_at + name_len..name_at + name_len + value_len)
                .ok_or(corrupt)?;
            if set
                .entries
                .insert(String::from(name), Vec::from(value))
                .is_some()
            {
                return Err(corrupt);
            }
            at += (ENTRY_HEADER + name_len + value_len).next_multiple_of(4);
        }
        Ok(set)
    }
}

/// The most a memory filesystem keeps for one node, names and values
/// together, as the encoding counts them (its attributes always fit one
/// list buffer).
pub const MEMORY_XATTR_LIMIT: usize = XATTR_LIST_MAX;

/// Apply `f` to `set`, a memory filesystem node's attributes, keeping the
/// old ones if `f` fails or the result is over [`MEMORY_XATTR_LIMIT`]
/// (ENOSPC).
pub fn update_in_memory(
    set: &mut XattrSet,
    f: &mut dyn FnMut(&mut XattrSet) -> Result<(), KernelError>,
) -> Result<(), KernelError> {
    let mut new = set.clone();
    f(&mut new)?;
    if new.encoded_size() > MEMORY_XATTR_LIMIT {
        return Err(KernelError::FsError(FsError::NoSpace));
    }
    *set = new;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The set keeps Linux's limits itself, and encoding never truncates a
    /// length field (PR #33 review).
    #[test]
    fn set_refuses_what_the_encoding_cannot_hold() {
        let mut s = XattrSet::new();
        let long = alloc::format!("user.{}", "n".repeat(XATTR_NAME_MAX - 4));
        assert!(s.set(&long, b"v", 0).is_err(), "256-byte name");
        let max = alloc::format!("user.{}", "n".repeat(XATTR_NAME_MAX - 5));
        assert!(s.set(&max, b"v", 0).is_ok(), "255-byte name");
        assert!(s.set("", b"v", 0).is_err());
        // What decode would refuse from disk is refused here too.
        assert!(s.set("nonamespace", b"v", 0).is_err());
        assert!(s.set("user.a\0b", b"v", 0).is_err());
        assert!(s
            .set("user.big", &alloc::vec![0; XATTR_SIZE_MAX + 1], 0)
            .is_err());
        // A value of XATTR_SIZE_MAX is a valid attribute but not encodable
        // (its length is two bytes): encode refuses it, it does not wrap.
        let mut big = XattrSet::new();
        big.set("user.big", &alloc::vec![0; XATTR_SIZE_MAX], 0)
            .unwrap();
        let mut buf = alloc::vec![0u8; XATTR_SIZE_MAX + 64];
        assert_eq!(
            big.encode(&mut buf),
            Err(KernelError::FsError(FsError::NoSpace))
        );
        // A malformed file capability is refused for every caller.
        assert!(check_value(XATTR_NAME_CAPS, b"junk").is_err());
        assert!(check_value("user.x", b"junk").is_ok());
    }
    use crate::{
        fs::Permissions,
        process::{
            creds::Credentials,
            linux_caps::{LinuxCaps, CAP_FOWNER},
        },
    };

    fn meta(node_type: NodeType, mode: u32, uid: u32) -> Metadata {
        Metadata {
            node_type,
            size: 0,
            permissions: Permissions::from_mode(mode),
            uid,
            gid: uid,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 1,
        }
    }

    fn user(caps: &[u32]) -> Caller {
        Caller {
            creds: Credentials::new(1000, 1000),
            caps: caps.iter().fold(LinuxCaps::EMPTY, |s, &c| s.with(c)),
        }
    }

    fn fs_err(e: KernelError) -> FsError {
        match e {
            KernelError::FsError(f) => f,
            other => panic!("not an FsError: {:?}", other),
        }
    }

    /// Linux's xattr_resolve_name: the four prefixes, EOPNOTSUPP for any
    /// other, EINVAL for a bare prefix.
    #[test]
    fn names_resolve_to_linux_namespaces() {
        assert_eq!(namespace("user.mime").unwrap(), Namespace::User);
        assert_eq!(namespace("trusted.x").unwrap(), Namespace::Trusted);
        assert_eq!(
            namespace("security.capability").unwrap(),
            Namespace::Security
        );
        assert_eq!(
            namespace("system.posix_acl_access").unwrap(),
            Namespace::System
        );
        assert_eq!(
            fs_err(namespace("os2.x").unwrap_err()),
            FsError::NotSupported
        );
        assert!(matches!(
            namespace("user."),
            Err(KernelError::InvalidArgument { .. })
        ));
    }

    /// user.*: regular files, directories and sockets, by permission bits;
    /// a sticky directory's attributes only for its owner or CAP_FOWNER;
    /// other types refused (EPERM to write, ENODATA to read).
    #[test]
    fn user_attributes_follow_the_file_permissions() {
        let u = user(&[]);
        let mine = meta(NodeType::File, 0o644, 1000);
        let theirs = meta(NodeType::File, 0o644, 0);
        assert!(check_access(&u, &mine, "user.a", true).is_ok());
        assert!(check_access(&u, &theirs, "user.a", false).is_ok());
        assert_eq!(
            fs_err(check_access(&u, &theirs, "user.a", true).unwrap_err()),
            FsError::PermissionDenied
        );
        let link = meta(NodeType::Symlink, 0o777, 1000);
        assert_eq!(
            fs_err(check_access(&u, &link, "user.a", true).unwrap_err()),
            FsError::OperationNotPermitted
        );
        assert_eq!(
            fs_err(check_access(&u, &link, "user.a", false).unwrap_err()),
            FsError::NoData
        );
        let sticky = meta(NodeType::Directory, 0o1777, 0);
        assert_eq!(
            fs_err(check_access(&u, &sticky, "user.a", true).unwrap_err()),
            FsError::OperationNotPermitted
        );
        assert!(check_access(&u, &sticky, "user.a", false).is_ok());
        assert!(check_access(&user(&[CAP_FOWNER]), &sticky, "user.a", true).is_ok());
    }

    /// trusted.* needs CAP_SYS_ADMIN, and is not listed without it;
    /// security.* is readable by anyone and written with CAP_SYS_ADMIN, or
    /// CAP_SETFCAP for the file capability; system.* is not supported.
    #[test]
    fn privileged_namespaces_need_their_capabilities() {
        let f = meta(NodeType::File, 0o666, 1000);
        let u = user(&[]);
        let admin = user(&[CAP_SYS_ADMIN]);
        assert_eq!(
            fs_err(check_access(&u, &f, "trusted.a", false).unwrap_err()),
            FsError::NoData
        );
        assert_eq!(
            fs_err(check_access(&u, &f, "trusted.a", true).unwrap_err()),
            FsError::OperationNotPermitted
        );
        assert!(check_access(&admin, &f, "trusted.a", true).is_ok());
        assert!(!listed(&u, "trusted.a") && listed(&admin, "trusted.a") && listed(&u, "user.a"));
        assert!(check_access(&u, &f, "security.selinux", false).is_ok());
        assert!(check_access(&u, &f, "security.selinux", true).is_err());
        assert!(check_access(&admin, &f, "security.selinux", true).is_ok());
        assert!(check_access(&admin, &f, XATTR_NAME_CAPS, true).is_err());
        assert!(check_access(&user(&[CAP_SETFCAP]), &f, XATTR_NAME_CAPS, true).is_ok());
        assert_eq!(
            fs_err(check_access(&admin, &f, "system.posix_acl_access", false).unwrap_err()),
            FsError::NotSupported
        );
    }

    /// setxattr's flags and removexattr's errors.
    #[test]
    fn set_and_remove_follow_the_flags() {
        let mut s = XattrSet::new();
        s.set("user.a", b"1", 0).unwrap();
        assert_eq!(
            fs_err(s.set("user.a", b"2", XATTR_CREATE).unwrap_err()),
            FsError::AlreadyExists
        );
        s.set("user.a", b"2", XATTR_REPLACE).unwrap();
        assert_eq!(s.get("user.a"), Some(&b"2"[..]));
        assert_eq!(
            fs_err(s.set("user.b", b"", XATTR_REPLACE).unwrap_err()),
            FsError::NoData
        );
        s.set("user.empty", b"", 0).unwrap();
        assert_eq!(s.get("user.empty"), Some(&b""[..]));
        s.remove("user.a").unwrap();
        assert_eq!(fs_err(s.remove("user.a").unwrap_err()), FsError::NoData);
        assert_eq!(s.names().collect::<Vec<_>>(), ["user.empty"]);
    }

    /// The block encoding round-trips, refuses what does not fit (ENOSPC)
    /// and rejects a block that is not one or runs past its end.
    #[test]
    fn attribute_blocks_round_trip() {
        let mut s = XattrSet::new();
        s.set("user.mime_type", b"text/plain", 0).unwrap();
        s.set("security.capability", &[1, 2, 3, 4, 5], 0).unwrap();
        s.set("trusted.e", b"", 0).unwrap();
        let mut block = [0xAAu8; 4096];
        s.encode(&mut block).unwrap();
        assert_eq!(XattrSet::decode(&block).unwrap(), s);
        assert_eq!(
            XattrSet::decode(&[0u8; 4096]).unwrap_err(),
            KernelError::FsError(FsError::CorruptedData)
        );
        // A count past the entries there are.
        block[4] = 9;
        assert!(XattrSet::decode(&block).is_err());

        let mut big = XattrSet::new();
        big.set("user.big", &[7u8; 4090], 0).unwrap();
        assert_eq!(
            fs_err(big.encode(&mut [0u8; 4096]).unwrap_err()),
            FsError::NoSpace
        );
        assert!(XattrSet::new().encoded_size() <= 4096);
    }

    /// A block from disk holds only names `set` would have stored: one with
    /// a NUL (which would split a listxattr entry), none outside the four
    /// namespaces, and no name twice (PR #33 review).
    #[test]
    fn decode_rejects_names_set_would_refuse() {
        let encoded = |names: &[&str]| {
            let mut block = alloc::vec![0u8; 4096];
            block[0..4].copy_from_slice(&BLOCK_MAGIC.to_le_bytes());
            block[4..6].copy_from_slice(&(names.len() as u16).to_le_bytes());
            let mut at = BLOCK_HEADER;
            for name in names {
                block[at] = name.len() as u8;
                block[at + 2] = 1;
                let name_at = at + ENTRY_HEADER;
                block[name_at..name_at + name.len()].copy_from_slice(name.as_bytes());
                at += (ENTRY_HEADER + name.len() + 1).next_multiple_of(4);
            }
            block
        };
        assert!(XattrSet::decode(&encoded(&["user.a", "user.b"])).is_ok());
        for bad in [
            &["user.a\0user.b"][..],
            &["nonamespace"],
            &["user."],
            &["user.a", "user.a"],
        ] {
            assert_eq!(
                XattrSet::decode(&encoded(bad)).unwrap_err(),
                KernelError::FsError(FsError::CorruptedData),
                "{bad:?}"
            );
        }
    }

    /// A failed or oversized update leaves a memory node's attributes as
    /// they were.
    #[test]
    fn memory_updates_are_all_or_nothing() {
        let mut s = XattrSet::new();
        s.set("user.keep", b"1", 0).unwrap();
        let mut too_big = |set: &mut XattrSet| set.set("user.x", &[0; MEMORY_XATTR_LIMIT], 0);
        assert_eq!(
            fs_err(update_in_memory(&mut s, &mut too_big).unwrap_err()),
            FsError::NoSpace
        );
        let mut fails = |set: &mut XattrSet| {
            set.remove("user.keep")?;
            set.remove("user.keep")
        };
        assert!(update_in_memory(&mut s, &mut fails).is_err());
        assert_eq!(s.get("user.keep"), Some(&b"1"[..]));
    }
}

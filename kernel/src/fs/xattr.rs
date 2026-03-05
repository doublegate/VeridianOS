//! Extended Attributes (xattr) -- Per-Inode Metadata Store
//!
//! Provides POSIX-compatible extended file attributes with namespace support.
//! Attributes are stored in-memory (suitable for RamFS/tmpfs) using a global
//! store keyed by inode number. Each attribute has a namespaced name (e.g.,
//! "user.mime_type" or "system.selinux") and an arbitrary byte value.
//!
//! Syscall-level functions: [`getxattr`], [`setxattr`], [`listxattr`],
//! [`removexattr`]. Call [`cleanup_inode_xattrs`] when an inode is deleted.

use alloc::{collections::BTreeMap, string::String, vec::Vec};

#[cfg(not(target_arch = "aarch64"))]
use spin::RwLock;

#[cfg(target_arch = "aarch64")]
use super::bare_lock::RwLock;
use crate::error::{FsError, KernelError};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum size of a single attribute value (64 KB).
pub const XATTR_MAX_VALUE_SIZE: usize = 65536;

/// Maximum number of attributes per inode.
pub const XATTR_MAX_ATTRS_PER_INODE: usize = 256;

/// Maximum length of an attribute name (including namespace prefix).
pub const XATTR_MAX_NAME_LEN: usize = 255;

/// Flag: fail if the attribute already exists (exclusive create).
pub const XATTR_CREATE: u32 = 1;

/// Flag: fail if the attribute does not already exist (replace only).
pub const XATTR_REPLACE: u32 = 2;

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Global extended attribute store.
///
/// Maps inode number -> (attribute name -> value).  All access is serialised
/// through a single `RwLock` which is acceptable for the current single-CPU
/// boot context.  A per-inode lock design can be adopted later if contention
/// becomes measurable.
struct XattrStore {
    /// inode -> { name -> value }
    attrs: BTreeMap<u64, BTreeMap<String, Vec<u8>>>,
}

impl XattrStore {
    const fn new() -> Self {
        Self {
            attrs: BTreeMap::new(),
        }
    }
}

/// Global singleton for the extended attribute store.
static XATTR_STORE: RwLock<XattrStore> = RwLock::new(XattrStore::new());

// ---------------------------------------------------------------------------
// Namespace validation
// ---------------------------------------------------------------------------

/// Recognised xattr namespace prefixes.
const VALID_NAMESPACES: &[&str] = &["user.", "system."];

/// Validate that `name` begins with a supported namespace prefix and is
/// otherwise well-formed.
fn validate_name(name: &str) -> Result<(), KernelError> {
    if name.is_empty() {
        return Err(KernelError::InvalidArgument {
            name: "xattr_name",
            value: "empty name",
        });
    }

    if name.len() > XATTR_MAX_NAME_LEN {
        return Err(KernelError::InvalidArgument {
            name: "xattr_name",
            value: "name too long",
        });
    }

    for ns in VALID_NAMESPACES {
        if name.starts_with(ns) {
            // The attribute-specific part must not be empty.
            if name.len() == ns.len() {
                return Err(KernelError::InvalidArgument {
                    name: "xattr_name",
                    value: "empty attribute after namespace",
                });
            }
            return Ok(());
        }
    }

    Err(KernelError::InvalidArgument {
        name: "xattr_name",
        value: "unsupported namespace",
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Retrieve the value of an extended attribute.
///
/// Returns the attribute value as a byte vector, or an error if the inode or
/// attribute name does not exist.
pub fn getxattr(inode: u64, name: &str) -> Result<Vec<u8>, KernelError> {
    validate_name(name)?;

    let store = XATTR_STORE.read();
    let inode_attrs = store
        .attrs
        .get(&inode)
        .ok_or(KernelError::FsError(FsError::NotFound))?;

    inode_attrs
        .get(name)
        .cloned()
        .ok_or(KernelError::FsError(FsError::NotFound))
}

/// Set (create or replace) an extended attribute.
///
/// `flags` controls behaviour when the attribute already exists or not:
///
/// | flags | Exists | Does not exist |
/// |-------|--------|----------------|
/// | 0     | Replace | Create        |
/// | `XATTR_CREATE`  | Error | Create |
/// | `XATTR_REPLACE` | Replace | Error |
///
/// Returns an error if value exceeds [`XATTR_MAX_VALUE_SIZE`] or the inode
/// already has [`XATTR_MAX_ATTRS_PER_INODE`] attributes.
pub fn setxattr(inode: u64, name: &str, value: &[u8], flags: u32) -> Result<(), KernelError> {
    validate_name(name)?;

    if value.len() > XATTR_MAX_VALUE_SIZE {
        return Err(KernelError::InvalidArgument {
            name: "xattr_value",
            value: "value too large",
        });
    }

    let mut store = XATTR_STORE.write();
    let inode_attrs = store.attrs.entry(inode).or_default();

    let exists = inode_attrs.contains_key(name);

    // Enforce XATTR_CREATE / XATTR_REPLACE semantics.
    if flags & XATTR_CREATE != 0 && exists {
        return Err(KernelError::FsError(FsError::AlreadyExists));
    }
    if flags & XATTR_REPLACE != 0 && !exists {
        return Err(KernelError::FsError(FsError::NotFound));
    }

    // Enforce per-inode limit (only when inserting a new key).
    if !exists && inode_attrs.len() >= XATTR_MAX_ATTRS_PER_INODE {
        return Err(KernelError::ResourceExhausted {
            resource: "xattr slots",
        });
    }

    inode_attrs.insert(String::from(name), Vec::from(value));
    Ok(())
}

/// List all extended attribute names for an inode.
///
/// Returns an empty vector if the inode has no attributes.
pub fn listxattr(inode: u64) -> Result<Vec<String>, KernelError> {
    let store = XATTR_STORE.read();
    match store.attrs.get(&inode) {
        Some(inode_attrs) => Ok(inode_attrs.keys().cloned().collect()),
        None => Ok(Vec::new()),
    }
}

/// Remove a single extended attribute.
///
/// Returns an error if the attribute does not exist.
pub fn removexattr(inode: u64, name: &str) -> Result<(), KernelError> {
    validate_name(name)?;

    let mut store = XATTR_STORE.write();
    let inode_attrs = store
        .attrs
        .get_mut(&inode)
        .ok_or(KernelError::FsError(FsError::NotFound))?;

    if inode_attrs.remove(name).is_none() {
        return Err(KernelError::FsError(FsError::NotFound));
    }

    // If no attributes remain, drop the inode entry to save memory.
    if inode_attrs.is_empty() {
        store.attrs.remove(&inode);
    }

    Ok(())
}

/// Remove all extended attributes for an inode.
///
/// Intended to be called when an inode is deleted.  This is a no-op if the
/// inode has no attributes.
pub fn cleanup_inode_xattrs(inode: u64) {
    let mut store = XATTR_STORE.write();
    store.attrs.remove(&inode);
}

/// Return the number of attributes currently stored for `inode`.
///
/// Returns 0 if the inode has no attributes.
pub fn count_xattrs(inode: u64) -> usize {
    let store = XATTR_STORE.read();
    store.attrs.get(&inode).map_or(0, BTreeMap::len)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    // Use a high inode range to avoid collisions between parallel tests
    // (each test uses its own inode number).
    const BASE_INODE: u64 = 0xFFFF_0000;

    // -- validate_name -------------------------------------------------------

    #[test]
    fn test_validate_name_user_ns() {
        assert!(validate_name("user.mime_type").is_ok());
    }

    #[test]
    fn test_validate_name_system_ns() {
        assert!(validate_name("system.selinux").is_ok());
    }

    #[test]
    fn test_validate_name_bad_ns() {
        assert!(validate_name("trusted.key").is_err());
        assert!(validate_name("security.ima").is_err());
    }

    #[test]
    fn test_validate_name_empty() {
        assert!(validate_name("").is_err());
    }

    #[test]
    fn test_validate_name_no_attr_after_ns() {
        assert!(validate_name("user.").is_err());
        assert!(validate_name("system.").is_err());
    }

    #[test]
    fn test_validate_name_too_long() {
        let long_name = alloc::format!("user.{}", "a".repeat(XATTR_MAX_NAME_LEN));
        assert!(validate_name(&long_name).is_err());
    }

    // -- setxattr / getxattr -------------------------------------------------

    #[test]
    fn test_set_and_get() {
        let inode = BASE_INODE + 1;
        // Clean up from potential previous test runs.
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.color", b"blue", 0).unwrap();
        let val = getxattr(inode, "user.color").unwrap();
        assert_eq!(val, b"blue");

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_set_replaces_existing() {
        let inode = BASE_INODE + 2;
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.key", b"v1", 0).unwrap();
        setxattr(inode, "user.key", b"v2", 0).unwrap();
        assert_eq!(getxattr(inode, "user.key").unwrap(), b"v2");

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_set_create_flag_rejects_existing() {
        let inode = BASE_INODE + 3;
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.key", b"v1", 0).unwrap();
        let result = setxattr(inode, "user.key", b"v2", XATTR_CREATE);
        assert_eq!(
            result.unwrap_err(),
            KernelError::FsError(FsError::AlreadyExists)
        );

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_set_replace_flag_rejects_new() {
        let inode = BASE_INODE + 4;
        cleanup_inode_xattrs(inode);

        let result = setxattr(inode, "user.key", b"v1", XATTR_REPLACE);
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NotFound));

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_set_value_too_large() {
        let inode = BASE_INODE + 5;
        cleanup_inode_xattrs(inode);

        let big = vec![0u8; XATTR_MAX_VALUE_SIZE + 1];
        let result = setxattr(inode, "user.big", &big, 0);
        assert!(result.is_err());

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_set_max_attrs_per_inode() {
        let inode = BASE_INODE + 6;
        cleanup_inode_xattrs(inode);

        for i in 0..XATTR_MAX_ATTRS_PER_INODE {
            let name = alloc::format!("user.attr{}", i);
            setxattr(inode, &name, b"x", 0).unwrap();
        }

        // The 257th should fail.
        let result = setxattr(inode, "user.overflow", b"x", 0);
        assert_eq!(
            result.unwrap_err(),
            KernelError::ResourceExhausted {
                resource: "xattr slots",
            }
        );

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_set_invalid_namespace() {
        let inode = BASE_INODE + 7;
        let result = setxattr(inode, "trusted.secret", b"val", 0);
        assert!(result.is_err());
    }

    // -- getxattr errors -----------------------------------------------------

    #[test]
    fn test_get_nonexistent_inode() {
        let inode = BASE_INODE + 8;
        cleanup_inode_xattrs(inode);

        let result = getxattr(inode, "user.nope");
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NotFound));
    }

    #[test]
    fn test_get_nonexistent_attr() {
        let inode = BASE_INODE + 9;
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.exists", b"yes", 0).unwrap();
        let result = getxattr(inode, "user.missing");
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NotFound));

        cleanup_inode_xattrs(inode);
    }

    // -- listxattr -----------------------------------------------------------

    #[test]
    fn test_list_empty() {
        let inode = BASE_INODE + 10;
        cleanup_inode_xattrs(inode);

        let names = listxattr(inode).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn test_list_multiple() {
        let inode = BASE_INODE + 11;
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.a", b"1", 0).unwrap();
        setxattr(inode, "system.b", b"2", 0).unwrap();
        setxattr(inode, "user.c", b"3", 0).unwrap();

        let mut names = listxattr(inode).unwrap();
        names.sort();
        assert_eq!(names.len(), 3);
        assert_eq!(names[0], "system.b");
        assert_eq!(names[1], "user.a");
        assert_eq!(names[2], "user.c");

        cleanup_inode_xattrs(inode);
    }

    // -- removexattr ---------------------------------------------------------

    #[test]
    fn test_remove() {
        let inode = BASE_INODE + 12;
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.del", b"bye", 0).unwrap();
        removexattr(inode, "user.del").unwrap();

        let result = getxattr(inode, "user.del");
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NotFound));
    }

    #[test]
    fn test_remove_nonexistent() {
        let inode = BASE_INODE + 13;
        cleanup_inode_xattrs(inode);

        let result = removexattr(inode, "user.ghost");
        assert_eq!(result.unwrap_err(), KernelError::FsError(FsError::NotFound));
    }

    #[test]
    fn test_remove_cleans_inode_entry() {
        let inode = BASE_INODE + 14;
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.only", b"val", 0).unwrap();
        removexattr(inode, "user.only").unwrap();

        // After removing the last attribute the inode entry should be gone,
        // so listxattr returns an empty vec (not an error).
        let names = listxattr(inode).unwrap();
        assert!(names.is_empty());
    }

    // -- cleanup_inode_xattrs ------------------------------------------------

    #[test]
    fn test_cleanup() {
        let inode = BASE_INODE + 15;
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.a", b"1", 0).unwrap();
        setxattr(inode, "user.b", b"2", 0).unwrap();
        assert_eq!(count_xattrs(inode), 2);

        cleanup_inode_xattrs(inode);
        assert_eq!(count_xattrs(inode), 0);
    }

    // -- count_xattrs --------------------------------------------------------

    #[test]
    fn test_count() {
        let inode = BASE_INODE + 16;
        cleanup_inode_xattrs(inode);

        assert_eq!(count_xattrs(inode), 0);
        setxattr(inode, "user.x", b"val", 0).unwrap();
        assert_eq!(count_xattrs(inode), 1);
        setxattr(inode, "system.y", b"val", 0).unwrap();
        assert_eq!(count_xattrs(inode), 2);

        cleanup_inode_xattrs(inode);
    }

    // -- edge cases ----------------------------------------------------------

    #[test]
    fn test_empty_value() {
        let inode = BASE_INODE + 17;
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.empty", b"", 0).unwrap();
        let val = getxattr(inode, "user.empty").unwrap();
        assert!(val.is_empty());

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_max_value_size() {
        let inode = BASE_INODE + 18;
        cleanup_inode_xattrs(inode);

        let data = vec![0xABu8; XATTR_MAX_VALUE_SIZE];
        setxattr(inode, "user.big", &data, 0).unwrap();
        let val = getxattr(inode, "user.big").unwrap();
        assert_eq!(val.len(), XATTR_MAX_VALUE_SIZE);

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_binary_value() {
        let inode = BASE_INODE + 19;
        cleanup_inode_xattrs(inode);

        let binary: Vec<u8> = (0..=255u8).collect();
        setxattr(inode, "user.bin", &binary, 0).unwrap();
        assert_eq!(getxattr(inode, "user.bin").unwrap(), binary);

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_replace_flag_succeeds_on_existing() {
        let inode = BASE_INODE + 20;
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.key", b"old", 0).unwrap();
        setxattr(inode, "user.key", b"new", XATTR_REPLACE).unwrap();
        assert_eq!(getxattr(inode, "user.key").unwrap(), b"new");

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_create_flag_succeeds_on_new() {
        let inode = BASE_INODE + 21;
        cleanup_inode_xattrs(inode);

        setxattr(inode, "user.fresh", b"val", XATTR_CREATE).unwrap();
        assert_eq!(getxattr(inode, "user.fresh").unwrap(), b"val");

        cleanup_inode_xattrs(inode);
    }

    #[test]
    fn test_multiple_inodes_isolated() {
        let inode_a = BASE_INODE + 22;
        let inode_b = BASE_INODE + 23;
        cleanup_inode_xattrs(inode_a);
        cleanup_inode_xattrs(inode_b);

        setxattr(inode_a, "user.key", b"from_a", 0).unwrap();
        setxattr(inode_b, "user.key", b"from_b", 0).unwrap();

        assert_eq!(getxattr(inode_a, "user.key").unwrap(), b"from_a");
        assert_eq!(getxattr(inode_b, "user.key").unwrap(), b"from_b");

        cleanup_inode_xattrs(inode_a);
        cleanup_inode_xattrs(inode_b);
    }
}

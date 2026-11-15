//! landlock_create_ruleset, landlock_add_rule and landlock_restrict_self,
//! as Linux 7.3's `security/landlock/syscalls.c` (ABI 11; ADR 0029). The
//! rules and checks are `veridian-landlock`'s; the kernel's side is
//! `security::landlock`.

use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use super::{userspace, SyscallError, SyscallResult};
use crate::{
    fs::{
        file::{File, OpenFlags},
        VfsNode,
    },
    process::{creds::Caller, linux_caps::CAP_SYS_ADMIN, thread::ThreadState},
    security::landlock::{self as l, ll, RulesetNode},
};

/// landlock_create_ruleset's flags.
const CREATE_RULESET_VERSION: u32 = 1 << 0;
const CREATE_RULESET_ERRATA: u32 = 1 << 1;

/// The rule types.
const RULE_PATH_BENEATH: usize = 1;
const RULE_NET_PORT: usize = 2;

/// `struct landlock_ruleset_attr`: six u64 masks.
const RULESET_ATTR_SIZE: usize = 48;
/// What a caller must pass at least: `handled_access_fs`.
const RULESET_ATTR_MIN: usize = 8;
const PAGE_SIZE: usize = 4096;

/// What a crate refusal is for a system call.
fn from_ll(e: ll::Error) -> SyscallError {
    match e {
        ll::Error::Invalid => SyscallError::InvalidArgument,
        ll::Error::NoMessage => SyscallError::NoMessage,
        ll::Error::TooBig => SyscallError::ArgumentListTooLong,
        ll::Error::Denied => SyscallError::PermissionDenied,
        ll::Error::CrossDevice => SyscallError::CrossDevice,
    }
}

/// A mask from user space, which must fit the 32 bits rights use (EINVAL
/// for anything above: no right is defined there).
fn mask(v: u64) -> Result<u32, SyscallError> {
    u32::try_from(v).map_err(|_| SyscallError::InvalidArgument)
}

/// landlock_create_ruleset(attr, size, flags).
pub fn sys_landlock_create_ruleset(attr: usize, size: usize, flags: usize) -> SyscallResult {
    let flags = flags as u32;
    if flags != 0 {
        if attr != 0 || size != 0 {
            return Err(SyscallError::InvalidArgument);
        }
        return match flags {
            CREATE_RULESET_VERSION => Ok(ll::ABI_VERSION as usize),
            CREATE_RULESET_ERRATA => Ok(ll::ERRATA as usize),
            _ => Err(SyscallError::InvalidArgument),
        };
    }
    // copy_min_struct_from_user.
    if attr == 0 {
        return Err(SyscallError::InvalidPointer);
    }
    if size < RULESET_ATTR_MIN {
        return Err(SyscallError::InvalidArgument);
    }
    if size > PAGE_SIZE {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let mut raw = alloc::vec![0u8; size.max(RULESET_ATTR_SIZE)];
    userspace::read_user_bytes(attr, &mut raw[..size])?;
    // copy_struct_from_user: what this kernel does not know must be zero.
    if raw[RULESET_ATTR_SIZE..].iter().any(|&b| b != 0) {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let word = |i: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&raw[i * 8..i * 8 + 8]);
        u64::from_ne_bytes(b)
    };
    let attr = ll::RulesetAttr {
        handled: ll::AccessMasks {
            fs: mask(word(0))?,
            net: mask(word(1))?,
            scope: mask(word(2))?,
        },
        quiet: ll::AccessMasks {
            fs: mask(word(3))?,
            net: mask(word(4))?,
            scope: mask(word(5))?,
        },
    };
    let ruleset = ll::Ruleset::new(attr).map_err(from_ll)?;
    let node: Arc<dyn VfsNode> = Arc::new(RulesetNode::new(ruleset));
    let file = Arc::new(File::new(node, OpenFlags::read_write()));
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let fd = process
        .file_table
        .lock()
        .open_with_flags(file, true)
        .map_err(super::map_kernel_error)?;
    Ok(fd)
}

/// The ruleset `fd` names (get_ruleset_from_fd): EBADF without one, EBADFD
/// for another kind of file, EPERM without the access `write` (add_rule)
/// or reading (restrict_self) needs.
fn ruleset_file(fd: usize, write: bool) -> Result<Arc<File>, SyscallError> {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = process
        .file_table
        .lock()
        .get(fd as u32 as usize)
        .ok_or(SyscallError::BadFileDescriptor)?;
    if ruleset_of(&file).is_none() {
        return Err(SyscallError::BadFdState);
    }
    if (write && !file.flags.write) || (!write && !file.flags.read) {
        return Err(SyscallError::OperationNotPermitted);
    }
    Ok(file)
}

fn ruleset_of(file: &File) -> Option<&RulesetNode> {
    file.node.as_any()?.downcast_ref::<RulesetNode>()
}

/// landlock_add_rule(ruleset_fd, rule_type, rule_attr, flags).
pub fn sys_landlock_add_rule(
    ruleset_fd: usize,
    rule_type: usize,
    rule_attr: usize,
    flags: usize,
) -> SyscallResult {
    let flags = flags as u32;
    if flags != 0 && flags != ll::ADD_RULE_QUIET {
        return Err(SyscallError::InvalidArgument);
    }
    let file = ruleset_file(ruleset_fd, true)?;
    let ruleset = ruleset_of(&file).ok_or(SyscallError::BadFdState)?;
    match rule_type as u32 as usize {
        RULE_PATH_BENEATH => add_path_beneath(ruleset, rule_attr, flags),
        RULE_NET_PORT => add_net_port(ruleset, rule_attr, flags),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// `struct landlock_path_beneath_attr` (packed): `u64 allowed_access;
/// s32 parent_fd`.
fn add_path_beneath(ruleset: &RulesetNode, attr: usize, flags: u32) -> SyscallResult {
    let mut raw = [0u8; 12];
    userspace::read_user_bytes(attr, &mut raw)?;
    let allowed = u64::from_ne_bytes(raw[0..8].try_into().unwrap_or([0; 8]));
    let parent_fd = i32::from_ne_bytes(raw[8..12].try_into().unwrap_or([0; 4]));
    if flags == 0 && allowed == 0 {
        return Err(SyscallError::NoMessage);
    }
    let allowed = mask(allowed)?;
    ruleset.check_fs_rule(allowed, flags).map_err(from_ll)?;
    let node = rule_anchor(parent_fd)?;
    ruleset.add_path(&node, allowed, flags).map_err(from_ll)?;
    Ok(0)
}

/// The object a path rule is anchored on: what `fd` was opened on
/// (get_path_from_fd, O_PATH descriptors too). EBADF without a
/// descriptor; EBADFD for a ruleset, a file with no path (pipe, socket,
/// anonymous inode) or an internal filesystem's file (nsfs).
fn rule_anchor(fd: i32) -> Result<Arc<dyn VfsNode>, SyscallError> {
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = process
        .file_table
        .lock()
        .get_path(fd as u32 as usize)
        .ok_or(SyscallError::BadFileDescriptor)?;
    if ruleset_of(&file).is_some() {
        return Err(SyscallError::BadFdState);
    }
    let Some(path) = file.path.as_deref() else {
        return Err(SyscallError::BadFdState);
    };
    if file
        .node
        .as_any()
        .is_some_and(|a| a.is::<crate::fs::nsfs::NsFileNode>())
    {
        return Err(SyscallError::BadFdState);
    }
    // An opened device or FIFO's node is what the driver gave the open,
    // not the inode with the name: the name's node is the anchor, as on
    // Linux, where a rule names the dentry's inode.
    use crate::fs::NodeType;
    let named = matches!(
        file.node.node_type(),
        NodeType::Directory | NodeType::File | NodeType::Symlink
    );
    if named {
        return Ok(file.node.clone());
    }
    let view = super::filesystem::vfs()?;
    Ok(view
        .canonical_node(path)
        .unwrap_or_else(|| file.node.clone()))
}

/// `struct landlock_net_port_attr`: `u64 allowed_access; u64 port`.
fn add_net_port(ruleset: &RulesetNode, attr: usize, flags: u32) -> SyscallResult {
    let [allowed, port]: [u64; 2] = userspace::read_user(attr)?;
    if flags == 0 && allowed == 0 {
        return Err(SyscallError::NoMessage);
    }
    let allowed = mask(allowed)?;
    ruleset.check_net_rule(allowed, flags).map_err(from_ll)?;
    ruleset.add_port(port, allowed, flags).map_err(from_ll)?;
    Ok(0)
}

/// A thread past the point of no return of its exit (PF_EXITING), which
/// TSYNC leaves alone.
fn exiting(t: &crate::process::thread::Thread) -> bool {
    matches!(t.get_state(), ThreadState::Zombie | ThreadState::Dead)
}

/// landlock_restrict_self(ruleset_fd, flags).
pub fn sys_landlock_restrict_self(ruleset_fd: usize, flags: usize) -> SyscallResult {
    use ll::restrict as r;
    let flags = flags as u32;
    if flags & !r::MASK != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
    // As seccomp's condition, but EPERM; NO_NEW_PRIVS fulfils it.
    let me = Caller::current();
    if flags & r::NO_NEW_PRIVS == 0
        && !thread.no_new_privs.load(Ordering::Acquire)
        && !me.ns_capable(&me.userns, CAP_SYS_ADMIN)
    {
        return Err(SyscallError::OperationNotPermitted);
    }
    // Only LOG_SUBDOMAINS_OFF (with TSYNC or not) goes without a ruleset:
    // it changes logging, of which there is none (no audit subsystem).
    let fd = ruleset_fd as u32 as i32;
    let ruleset = if fd == -1 && flags & !r::TSYNC == r::LOG_SUBDOMAINS_OFF {
        None
    } else {
        Some(ruleset_file(ruleset_fd, false)?)
    };
    let tsync = flags & r::TSYNC != 0;
    // Under the thread map, as seccomp's TSYNC (ADR 0027 decision 27), and
    // for TSYNC the credential guard, so no exec decides privileges
    // halfway.
    let _cred = tsync.then(|| process.cred_guard.lock());
    let threads = process.threads.lock();
    let domain = match &ruleset {
        Some(file) => {
            let node = ruleset_of(file).ok_or(SyscallError::BadFdState)?;
            let current = thread.landlock.get();
            Some(Arc::new(
                l::restrict(current.as_deref(), node).map_err(from_ll)?,
            ))
        }
        None => thread.landlock.get(),
    };
    if tsync {
        // landlock_restrict_sibling_threads: every live thread takes the
        // new domain, and no_new_privs when the caller has it or asks it.
        let nnp = flags & r::NO_NEW_PRIVS != 0 || thread.no_new_privs.load(Ordering::Acquire);
        for t in threads.values() {
            if Arc::ptr_eq(t, &thread) || exiting(t) {
                continue;
            }
            if nnp {
                t.no_new_privs.store(true, Ordering::Release);
            }
            if let Some(d) = &domain {
                t.landlock.set(d.clone());
            }
        }
    }
    // no_new_privs past the last point of failure.
    if flags & r::NO_NEW_PRIVS != 0 {
        thread.no_new_privs.store(true, Ordering::Release);
    }
    if let Some(d) = domain {
        thread.landlock.set(d);
    }
    Ok(0)
}

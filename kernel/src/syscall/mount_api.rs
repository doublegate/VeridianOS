//! The new mount API, as Linux 7.3 (fs/fsopen.c, fs/namespace.c): fsopen,
//! fsconfig, fsmount and fspick make and reconfigure filesystems through a
//! context descriptor ([`crate::fs::fscontext`]); open_tree and fsmount
//! return a descriptor for a detached tree, which move_mount attaches;
//! mount_setattr changes mount flags and propagation types.
//!
//! A detached tree is a [`mount::Mounts`] of its own (Linux's anonymous
//! mount namespace), named in paths by [`mount::DETACHED`]. Its descriptor
//! holds it; the last close drops it, unmounting what was never attached.
//!
//! Privilege is mount(2)'s ([`may_mount`]): CAP_SYS_ADMIN, then (ADR 0024)
//! MODIFY on the caller's mount namespace.

use alloc::{string::String, sync::Arc, vec::Vec};

use super::{
    filesystem::{read_user_path_or_empty, resolve_at_path, vfs},
    userspace::{read_user_bytes, read_user_cstr},
    SyscallError, SyscallResult,
};
use crate::{
    fs::{
        file::{status, File},
        fscontext::{CtxError, FsContext, FsContextNode, Phase, Purpose, Target, Value},
        mount::{self, flags as mnt, MountError, NewFsError, Propagation},
        NodeType, OpenFlags, VfsNode,
    },
    process::{self, creds::Caller, linux_caps::CAP_SYS_ADMIN},
};

/// AT_FDCWD.
const AT_FDCWD: i32 = -100;
const AT_SYMLINK_NOFOLLOW: usize = 0x100;
const AT_NO_AUTOMOUNT: usize = 0x800;
const AT_EMPTY_PATH: usize = 0x1000;
const AT_RECURSIVE: usize = 0x8000;
const O_CLOEXEC: usize = 0o2_000_000;

/// fsopen's flags.
const FSOPEN_CLOEXEC: usize = 1;

/// fsconfig's commands (enum fsconfig_command).
mod cmd {
    pub const SET_FLAG: usize = 0;
    pub const SET_STRING: usize = 1;
    pub const SET_BINARY: usize = 2;
    pub const SET_PATH: usize = 3;
    pub const SET_PATH_EMPTY: usize = 4;
    pub const SET_FD: usize = 5;
    pub const CREATE: usize = 6;
    pub const RECONFIGURE: usize = 7;
    pub const CREATE_EXCL: usize = 8;
}

/// fsmount's flags.
const FSMOUNT_CLOEXEC: usize = 1;
const FSMOUNT_NAMESPACE: usize = 2;

/// fspick's flags.
const FSPICK_CLOEXEC: usize = 1;
const FSPICK_SYMLINK_NOFOLLOW: usize = 2;
const FSPICK_NO_AUTOMOUNT: usize = 4;
const FSPICK_EMPTY_PATH: usize = 8;

/// move_mount's flags.
mod mm {
    pub const F_SYMLINKS: usize = 0x01;
    pub const F_EMPTY_PATH: usize = 0x04;
    pub const T_SYMLINKS: usize = 0x10;
    pub const T_EMPTY_PATH: usize = 0x40;
    pub const SET_GROUP: usize = 0x100;
    pub const BENEATH: usize = 0x200;
    pub const MASK: usize = 0x377;
}

/// open_tree's own flags.
const OPEN_TREE_CLONE: usize = 1;
const OPEN_TREE_NAMESPACE: usize = 2;

/// MOUNT_ATTR_* (include/uapi/linux/mount.h).
pub(crate) mod attr {
    pub const RDONLY: u64 = 0x01;
    pub const NOSUID: u64 = 0x02;
    pub const NODEV: u64 = 0x04;
    pub const NOEXEC: u64 = 0x08;
    pub const ATIME: u64 = 0x70;
    pub const RELATIME: u64 = 0x00;
    pub const NOATIME: u64 = 0x10;
    pub const STRICTATIME: u64 = 0x20;
    pub const NODIRATIME: u64 = 0x80;
    pub const IDMAP: u64 = 0x10_0000;
    pub const NOSYMFOLLOW: u64 = 0x20_0000;
    /// What fsmount takes (FSMOUNT_VALID_FLAGS).
    pub const FSMOUNT_VALID: u64 =
        RDONLY | NOSUID | NODEV | NOEXEC | ATIME | NODIRATIME | NOSYMFOLLOW;
}

/// The propagation types mount_setattr takes (MS_*).
mod ms {
    pub const UNBINDABLE: u64 = 1 << 17;
    pub const PRIVATE: u64 = 1 << 18;
    pub const SLAVE: u64 = 1 << 19;
    pub const SHARED: u64 = 1 << 20;
}

/// Linux's may_mount, with ADR 0024's namespace capability.
/// The check for FSMOUNT_NAMESPACE and OPEN_TREE_NAMESPACE (Linux:
/// ns_capable(current_user_ns(), CAP_SYS_ADMIN), the new namespace being
/// the caller's user namespace's), with the object capability unshare's
/// CLONE_NEWNS takes (ADR 0024, step 3). EPERM.
fn may_make_mount_ns() -> Result<(), SyscallError> {
    let caller = Caller::current();
    let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
    if !caller.ns_capable(&caller.userns, CAP_SYS_ADMIN)
        || !crate::cap::authority::current_may_create_namespaces(
            &thread.namespaces(),
            &caller.userns,
            crate::process::ns::CLONE_NEWNS,
        )
    {
        return Err(SyscallError::OperationNotPermitted);
    }
    Ok(())
}

/// A new mount namespace holding `tree`, owned by the caller's user
/// namespace, as a descriptor for setns (create_new_namespace). Linux puts
/// the tree on an empty nullfs root; here the tree is the root itself, so
/// listmount and statmount show one mount fewer (KNOWN-LIMITATIONS).
fn open_new_namespace(tree: Arc<mount::Mounts>, cloexec: bool) -> SyscallResult {
    let ns = crate::process::ns::MntNs::new(Caller::current().userns.clone(), tree);
    super::namespaces::open_ns_with(crate::process::ns::NsRef::Mnt(ns), cloexec)
}

/// The owner of the caller's mount namespace.
fn current_mnt_owner() -> Result<Arc<crate::process::userns::UserNs>, SyscallError> {
    let thread = crate::process::current_thread().ok_or(SyscallError::InvalidState)?;
    Ok(thread.namespaces().mnt.owner.clone())
}

fn may_mount() -> Result<(), SyscallError> {
    if !Caller::current().capable(CAP_SYS_ADMIN)
        || !crate::cap::authority::current_may_change_mounts()
    {
        return Err(SyscallError::OperationNotPermitted);
    }
    Ok(())
}

fn mount_error(e: MountError) -> SyscallError {
    match e {
        MountError::Invalid => SyscallError::InvalidArgument,
        MountError::Busy => SyscallError::Busy,
        MountError::NotDirectory => SyscallError::NotADirectory,
        MountError::Loop => SyscallError::SymlinkLoop,
        MountError::Again => SyscallError::WouldBlock,
        MountError::NoSpace => SyscallError::NoSpace,
        MountError::Perm => SyscallError::OperationNotPermitted,
    }
}

fn ctx_error(e: CtxError) -> SyscallError {
    match e {
        CtxError::Invalid => SyscallError::InvalidArgument,
        CtxError::Busy => SyscallError::Busy,
    }
}

/// The open file `fd` names, O_PATH ones included.
fn file_of(fd: usize) -> Result<Arc<File>, SyscallError> {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let file = proc.file_table.lock().get_path(fd as u32 as usize);
    file.ok_or(SyscallError::BadFileDescriptor)
}

/// The context `fd` names: EBADF if none is open, EINVAL if it is no
/// filesystem context.
fn context_of(fd: usize) -> Result<Arc<File>, SyscallError> {
    let file = file_of(fd)?;
    let is_context = file
        .node
        .as_any()
        .is_some_and(|any| any.is::<FsContextNode>());
    if !is_context {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(file)
}

fn context_node(file: &File) -> &FsContextNode {
    file.node
        .as_any()
        .and_then(|any| any.downcast_ref::<FsContextNode>())
        .expect("checked by context_of")
}

/// Install `file` in the caller's table.
fn install(file: File, cloexec: bool) -> SyscallResult {
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    let fd = proc
        .file_table
        .lock()
        .open_with_flags(Arc::new(file), cloexec)
        .map_err(super::map_kernel_error)?;
    Ok(fd)
}

/// An O_PATH descriptor for `node` at `path` (Linux's dentry_open with
/// O_PATH), holding `held` while it is open.
fn path_file(
    node: Arc<dyn VfsNode>,
    path: String,
    held: Option<Arc<dyn core::any::Any + Send + Sync>>,
) -> File {
    let mut file = File::new_with_path(node, OpenFlags::path_only(), path);
    file.opener = Some(alloc::boxed::Box::new(Caller::current()));
    file.status
        .fetch_or(status::PATH, core::sync::atomic::Ordering::AcqRel);
    if let Some(held) = held {
        file.hold(held);
    }
    file
}

/// What `path` names from `dirfd` (Linux's user_path_at), the node and
/// its canonical path: with `empty` an empty (or with `null_ok` a NULL)
/// path names `dirfd` itself, or with AT_FDCWD the working directory.
fn lookup_at(
    dirfd: usize,
    path_ptr: usize,
    empty: bool,
    null_ok: bool,
    follow: bool,
) -> Result<(Arc<dyn VfsNode>, String), SyscallError> {
    let path = if empty && null_ok && path_ptr == 0 {
        String::new()
    } else if empty {
        read_user_path_or_empty(path_ptr)?
    } else {
        super::filesystem::read_user_path(path_ptr)?
    };
    let view = vfs()?;
    if path.is_empty() {
        if dirfd as u32 as i32 == AT_FDCWD {
            return view.locate(".", true).map_err(super::map_kernel_error);
        }
        let file = file_of(dirfd)?;
        let path = file.path.clone().unwrap_or_default();
        // A detached tree's file, once the tree is attached, is where the
        // tree went.
        if path.starts_with(mount::DETACHED) {
            if let Ok((_, now)) = view.locate(&path, false) {
                return Ok((file.node.clone(), now));
            }
        }
        return Ok((file.node.clone(), path));
    }
    let abs = resolve_at_path(dirfd, &path)?;
    view.locate(&abs, follow).map_err(super::map_kernel_error)
}

/// The tree a canonical path is in and the path in it: a detached tree's
/// own path, or the caller's namespace's.
fn tree_and_path(
    canonical: &str,
) -> Result<(Arc<mount::Mounts>, String, Option<u64>), SyscallError> {
    if let Some((id, rest)) = mount::parse_detached(canonical) {
        let tree = mount::detached_tree(id).ok_or(SyscallError::ResourceNotFound)?;
        let rest = if rest.is_empty() { "/" } else { rest };
        return Ok((tree, String::from(rest), Some(id)));
    }
    Ok((vfs()?.mount_tree().clone(), String::from(canonical), None))
}

/// Whether `path` in `tree` is a mount's root (Linux's path_mounted).
fn is_mount_root(tree: &mount::Mounts, path: &str) -> bool {
    tree.mount_point_of(path) == path
}

/// fsopen(fsname, flags): a context for a new filesystem of type `fsname`.
pub fn sys_fsopen(fsname: usize, flags: usize) -> SyscallResult {
    may_mount()?;
    if flags & !FSOPEN_CLOEXEC != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let name = read_user_cstr(fsname, 4095)?;
    if !mount::MOUNTABLE.contains(&name.as_str()) {
        return Err(SyscallError::NoDevice);
    }
    let caller = Caller::current();
    let ctx = FsContext::new(
        Purpose::New { ty: name },
        (caller.creds.fsuid, caller.creds.fsgid),
    );
    let file = File::new(Arc::new(FsContextNode::new(ctx)), OpenFlags::read_write());
    install(file, flags & FSOPEN_CLOEXEC != 0)
}

/// fsconfig(fd, cmd, key, value, aux), in Linux's order: the fd's sign,
/// the arguments the command takes, the descriptor, the strings, then the
/// command under the context's lock.
pub fn sys_fsconfig(
    fd: usize,
    command: usize,
    key: usize,
    value: usize,
    aux: usize,
) -> SyscallResult {
    let aux = aux as i32;
    if (fd as i32) < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let bad = |cond: bool| {
        if cond {
            Err(SyscallError::InvalidArgument)
        } else {
            Ok(())
        }
    };
    match command {
        cmd::SET_FLAG => bad(key == 0 || value != 0 || aux != 0)?,
        cmd::SET_STRING => bad(key == 0 || value == 0 || aux != 0)?,
        cmd::SET_BINARY => bad(key == 0 || value == 0 || aux <= 0 || aux > 1024 * 1024)?,
        cmd::SET_PATH | cmd::SET_PATH_EMPTY => {
            bad(key == 0 || value == 0 || (aux != AT_FDCWD && aux < 0))?
        }
        cmd::SET_FD => bad(key == 0 || value != 0 || aux < 0)?,
        cmd::CREATE | cmd::RECONFIGURE | cmd::CREATE_EXCL => {
            bad(key != 0 || value != 0 || aux != 0)?
        }
        _ => return Err(SyscallError::NotSupported),
    }
    let file = context_of(fd)?;
    let node = context_node(&file);
    // The strings (strndup_user with 256: 255 bytes and the NUL).
    let key = if key != 0 {
        Some(read_user_cstr(key, 255)?)
    } else {
        None
    };
    let string = match command {
        cmd::SET_STRING => Some(read_user_cstr(value, 255)?),
        cmd::SET_BINARY => {
            let mut buf = alloc::vec![0u8; aux as usize];
            read_user_bytes(value, &mut buf)?;
            None
        }
        cmd::SET_PATH | cmd::SET_PATH_EMPTY => {
            if command == cmd::SET_PATH_EMPTY {
                read_user_path_or_empty(value)?;
            } else {
                super::filesystem::read_user_path(value)?;
            }
            None
        }
        cmd::SET_FD => {
            file_of(aux as usize)?;
            None
        }
        _ => None,
    };
    let mut ctx = node.ctx.lock();
    match command {
        cmd::CREATE | cmd::CREATE_EXCL => create(&mut ctx),
        cmd::RECONFIGURE => reconfigure(&mut ctx),
        _ => {
            let key = key.unwrap_or_default();
            let value = match (command, &string) {
                (cmd::SET_FLAG, _) => Value::Flag,
                (cmd::SET_STRING, Some(s)) => Value::String(s),
                _ => Value::Other,
            };
            ctx.set(&key, value).map(|()| 0).map_err(ctx_error)
        }
    }
}

/// FSCONFIG_CMD_CREATE (vfs_cmd_create): the filesystem, from the
/// options set; a failure leaves the context failed.
fn create(ctx: &mut FsContext) -> SyscallResult {
    if ctx.phase != Phase::CreateParams {
        return Err(SyscallError::Busy);
    }
    if !Caller::current().capable(CAP_SYS_ADMIN) {
        return Err(SyscallError::OperationNotPermitted);
    }
    match mount::new_filesystem(ctx.fs_type(), &ctx.options, ctx.owner) {
        Ok(fs) => {
            ctx.fs = Some(fs);
            ctx.phase = Phase::AwaitingMount;
            Ok(0)
        }
        Err(e) => {
            ctx.phase = Phase::Failed;
            Err(match e {
                NewFsError::UnknownType => SyscallError::NoDevice,
                NewFsError::BadOption => SyscallError::InvalidArgument,
            })
        }
    }
}

/// FSCONFIG_CMD_RECONFIGURE (vfs_cmd_reconfigure): the superblock's
/// read-only state, as `ro` or `rw` set it (the other options are not
/// applied, as with a remount); then the context takes parameters again.
fn reconfigure(ctx: &mut FsContext) -> SyscallResult {
    if ctx.phase != Phase::ReconfParams {
        return Err(SyscallError::Busy);
    }
    let Purpose::Reconfigure { target, .. } = &ctx.purpose else {
        return Err(SyscallError::Busy);
    };
    if !Caller::current().capable(CAP_SYS_ADMIN) {
        ctx.phase = Phase::Failed;
        return Err(SyscallError::OperationNotPermitted);
    }
    let place = match target {
        Target::At { tree, point } => Some((tree.clone(), point.clone())),
        Target::Detached(id) => mount::detached_place(*id)
            .map(|(tree, at)| (tree, at.unwrap_or_else(|| String::from("/")))),
    };
    let Some((tree, point)) = place else {
        ctx.phase = Phase::Failed;
        return Err(SyscallError::InvalidArgument);
    };
    let Some(mount) = tree.mount_at(&point) else {
        ctx.phase = Phase::Failed;
        return Err(SyscallError::InvalidArgument);
    };
    let writers = || super::mount::writing_under(&point, &tree);
    if let Err(e) = tree.reconfigure(&point, mount.flags(), ctx.sb_readonly, writers) {
        ctx.phase = Phase::Failed;
        return Err(mount_error(e));
    }
    ctx.options.clear();
    ctx.sb_readonly = None;
    Ok(0)
}

/// The mount flags for MOUNT_ATTR_* `attr` (attr_flags_to_mnt_flags):
/// relatime unless another atime type is asked for.
fn mnt_flags(attr: u64) -> u32 {
    let mut out = 0;
    for (a, m) in [
        (attr::RDONLY, mnt::READONLY),
        (attr::NOSUID, mnt::NOSUID),
        (attr::NODEV, mnt::NODEV),
        (attr::NOEXEC, mnt::NOEXEC),
        (attr::NODIRATIME, mnt::NODIRATIME),
        (attr::NOSYMFOLLOW, mnt::NOSYMFOLLOW),
    ] {
        if attr & a != 0 {
            out |= m;
        }
    }
    match attr & attr::ATIME {
        attr::NOATIME => out | mnt::NOATIME,
        attr::STRICTATIME => out,
        _ => out | mnt::RELATIME,
    }
}

/// fsmount(fs_fd, flags, attr_flags): mount what the context created, as
/// a detached tree, and return an O_PATH descriptor for its root.
pub fn sys_fsmount(fs_fd: usize, flags: usize, attr_flags: usize) -> SyscallResult {
    let attr_flags = attr_flags as u32 as u64;
    if flags & !(FSMOUNT_CLOEXEC | FSMOUNT_NAMESPACE) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    // FSMOUNT_NAMESPACE: the caller's privilege over its own user
    // namespace, not may_mount (the new namespace is that user namespace's
    // and touches no other); the context descriptor, made by someone who
    // could mount, is the authority over its filesystem. A filesystem that
    // shows the whole system (proc) must not reveal what a locked mount
    // hides (mount_too_revealing, below).
    let new_ns = flags & FSMOUNT_NAMESPACE != 0;
    if new_ns {
        may_make_mount_ns()?;
    } else {
        may_mount()?;
    }
    if attr_flags & !attr::FSMOUNT_VALID != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if !matches!(
        attr_flags & attr::ATIME,
        attr::RELATIME | attr::NOATIME | attr::STRICTATIME
    ) {
        return Err(SyscallError::InvalidArgument);
    }
    let file = context_of(fs_fd)?;
    let mut ctx = context_node(&file).ctx.lock();
    // No root yet (Linux's fc->root): a new filesystem's context before
    // FSCONFIG_CMD_CREATE. A reconfiguring one has its root.
    if ctx.fs.is_none() && matches!(ctx.purpose, Purpose::New { .. }) {
        return Err(SyscallError::InvalidArgument);
    }
    if ctx.phase != Phase::AwaitingMount {
        return Err(SyscallError::Busy);
    }
    let fs = ctx.fs.clone().ok_or(SyscallError::Busy)?;
    let source = ctx.source.clone().unwrap_or_else(|| String::from("none"));
    let sb_readonly = ctx.sb_readonly == Some(true);
    let mnt = mnt_flags(attr_flags);
    // mount_too_revealing: in a namespace not owned by the initial user
    // namespace, a new proc mount needs one here that shows all of proc,
    // and keeps its locked flags (EPERM otherwise).
    let revealing_locks = if crate::fs::mount::linux_fs_type(fs.name()) == "proc" {
        let ns = crate::process::current_thread()
            .ok_or(SyscallError::InvalidState)?
            .namespaces();
        if Arc::ptr_eq(&ns.mnt.owner, &crate::process::userns::initial()) {
            0
        } else {
            let readonly = if sb_readonly {
                mnt | mount::flags::READONLY
            } else {
                mnt
            };
            ns.mnt
                .mounts
                .fully_visible("proc", readonly)
                .ok_or(SyscallError::OperationNotPermitted)?
        }
    } else {
        0
    };
    let tree = Arc::new(
        vfs()?
            .mount_tree()
            .detached_new(fs.clone(), source, mnt, sb_readonly),
    );
    tree.lock_root(revealing_locks);
    // The context goes on to reconfigure what it mounted (vfs_clean_context):
    // the new namespace's root, or the detached tree's.
    let (target, id) = if new_ns {
        (
            Target::At {
                tree: tree.clone(),
                point: String::from("/"),
            },
            None,
        )
    } else {
        let id = mount::register_detached(&tree);
        (Target::Detached(id), Some(id))
    };
    let ty = String::from(ctx.fs_type());
    ctx.purpose = Purpose::Reconfigure { ty, target };
    ctx.phase = Phase::ReconfParams;
    ctx.options.clear();
    ctx.sb_readonly = None;
    drop(ctx);
    let Some(id) = id else {
        return open_new_namespace(tree, flags & FSMOUNT_CLOEXEC != 0);
    };
    let held: Arc<dyn core::any::Any + Send + Sync> = tree;
    install(
        path_file(fs.root(), mount::detached_path(id, "/"), Some(held)),
        flags & FSMOUNT_CLOEXEC != 0,
    )
}

/// fspick(dfd, path, flags): a context reconfiguring the filesystem whose
/// mount's root `path` is.
pub fn sys_fspick(dfd: usize, path: usize, flags: usize) -> SyscallResult {
    may_mount()?;
    if flags & !(FSPICK_CLOEXEC | FSPICK_SYMLINK_NOFOLLOW | FSPICK_NO_AUTOMOUNT | FSPICK_EMPTY_PATH)
        != 0
    {
        return Err(SyscallError::InvalidArgument);
    }
    let (_, canonical) = lookup_at(
        dfd,
        path,
        flags & FSPICK_EMPTY_PATH != 0,
        false,
        flags & FSPICK_SYMLINK_NOFOLLOW == 0,
    )?;
    let (tree, at, id) = tree_and_path(&canonical)?;
    if !is_mount_root(&tree, &at) {
        return Err(SyscallError::InvalidArgument);
    }
    let mount = tree.mount_at(&at).ok_or(SyscallError::InvalidArgument)?;
    let ty = String::from(mount::linux_fs_type(mount.fs().name()));
    let target = match id {
        Some(id) if at == "/" => Target::Detached(id),
        _ => Target::At { tree, point: at },
    };
    let caller = Caller::current();
    let ctx = FsContext::new(
        Purpose::Reconfigure { ty, target },
        (caller.creds.fsuid, caller.creds.fsgid),
    );
    let file = File::new(Arc::new(FsContextNode::new(ctx)), OpenFlags::read_write());
    install(file, flags & FSPICK_CLOEXEC != 0)
}

/// move_mount(from_dfd, from_path, to_dfd, to_path, flags): move a mount
/// of the caller's namespace, or attach a detached tree, at `to` (Linux's
/// do_move_mount).
pub fn sys_move_mount(
    from_dfd: usize,
    from_path: usize,
    to_dfd: usize,
    to_path: usize,
    flags: usize,
) -> SyscallResult {
    may_mount()?;
    if flags & !mm::MASK != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & (mm::BENEATH | mm::SET_GROUP) == mm::BENEATH | mm::SET_GROUP {
        return Err(SyscallError::InvalidArgument);
    }
    let (to_node, to) = lookup_at(
        to_dfd,
        to_path,
        flags & mm::T_EMPTY_PATH != 0,
        true,
        flags & mm::T_SYMLINKS != 0,
    )?;
    let (from_node, from) = lookup_at(
        from_dfd,
        from_path,
        flags & mm::F_EMPTY_PATH != 0,
        true,
        flags & mm::F_SYMLINKS != 0,
    )?;
    // Sharing groups need shared or slave mounts, which there are none
    // of (do_set_group's "from should not be private"); mounting beneath
    // is not done.
    if flags & (mm::SET_GROUP | mm::BENEATH) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let (from_tree, from_at, from_id) = tree_and_path(&from)?;
    if !is_mount_root(&from_tree, &from_at) {
        return Err(SyscallError::InvalidArgument);
    }
    let from_is_dir = from_node.node_type() == NodeType::Directory;
    let to_is_dir = to_node.node_type() == NodeType::Directory;
    if from_is_dir != to_is_dir {
        return Err(SyscallError::InvalidArgument);
    }
    let (to_tree, to_at, to_id) = tree_and_path(&to)?;
    match from_id {
        // A mount of the caller's namespace: moved within it.
        None => {
            if to_id.is_some() {
                return Err(SyscallError::InvalidArgument);
            }
            from_tree
                .move_mount(&from_at, from_is_dir, &to_at, to_is_dir)
                .map_err(mount_error)?;
            mount::moved_in(&from_tree, &from_at, &to_at);
        }
        // A detached tree's root: attached, into the caller's namespace or
        // another detached tree.
        Some(id) => {
            if from_at != "/" || to_id == Some(id) {
                return Err(SyscallError::InvalidArgument);
            }
            to_tree
                .attach_tree(&from_tree, &to_at, to_is_dir)
                .map_err(mount_error)?;
            mount::mark_attached(id, &to_tree, &to_at);
        }
    }
    Ok(0)
}

/// open_tree(dfd, path, flags): an O_PATH descriptor for `path`, or with
/// OPEN_TREE_CLONE for a detached copy of the mount there (and with
/// AT_RECURSIVE of the mounts under it).
pub fn sys_open_tree(dfd: usize, path: usize, flags: usize) -> SyscallResult {
    let clone = flags & OPEN_TREE_CLONE != 0;
    if flags
        & !(OPEN_TREE_CLONE
            | OPEN_TREE_NAMESPACE
            | O_CLOEXEC
            | AT_SYMLINK_NOFOLLOW
            | AT_NO_AUTOMOUNT
            | AT_EMPTY_PATH
            | AT_RECURSIVE)
        != 0
    {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & AT_RECURSIVE != 0 && flags & (OPEN_TREE_CLONE | OPEN_TREE_NAMESPACE) == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & (OPEN_TREE_CLONE | OPEN_TREE_NAMESPACE) == OPEN_TREE_CLONE | OPEN_TREE_NAMESPACE {
        return Err(SyscallError::InvalidArgument);
    }
    let new_ns = flags & OPEN_TREE_NAMESPACE != 0;
    if new_ns {
        may_make_mount_ns()?;
    }
    if clone {
        may_mount()?;
    }
    let (node, canonical) = lookup_at(
        dfd,
        path,
        flags & AT_EMPTY_PATH != 0,
        false,
        flags & AT_SYMLINK_NOFOLLOW == 0,
    )?;
    let cloexec = flags & O_CLOEXEC != 0;
    if new_ns {
        // create_new_namespace: a directory (ENOTDIR); the mount there,
        // with its submounts under AT_RECURSIVE. A caller from another
        // user namespace than the mount namespace's owner may not leave
        // submounts out, which would show what they cover (EINVAL).
        if node.node_type() != NodeType::Directory {
            return Err(SyscallError::NotADirectory);
        }
        let recursive = flags & AT_RECURSIVE != 0;
        let (tree, at, _) = tree_and_path(&canonical)?;
        // Linux 7.3 create_new_namespace (with the fix for leaving out
        // covering mounts): a caller whose user namespace does not own the
        // namespace the copy is made from may leave out nothing mounted
        // below (EINVAL), and gets unbindable mounts copied too and the
        // copy locked; anyone is refused a copy without a locked submount
        // (clone_tree, has_locked_children).
        let foreign = !Arc::ptr_eq(&Caller::current().userns, &current_mnt_owner()?);
        if foreign && !recursive && tree.has_submounts(&at) {
            return Err(SyscallError::InvalidArgument);
        }
        let copy = tree
            .clone_tree(&at, node.clone(), recursive, foreign)
            .map_err(mount_error)?;
        return open_new_namespace(Arc::new(copy), cloexec);
    }
    if !clone {
        return install(path_file(node, canonical, None), cloexec);
    }
    let (tree, at, _) = tree_and_path(&canonical)?;
    let copy = Arc::new(
        tree.detached_clone(&at, node.clone(), flags & AT_RECURSIVE != 0)
            .map_err(mount_error)?,
    );
    let id = mount::register_detached(&copy);
    let held: Arc<dyn core::any::Any + Send + Sync> = copy;
    install(
        path_file(node, mount::detached_path(id, "/"), Some(held)),
        cloexec,
    )
}

/// struct mount_attr: the first version's size, and the most read.
const MOUNT_ATTR_SIZE_VER0: usize = 32;
const PAGE_SIZE: usize = 4096;

/// What mount_setattr was asked to change (Linux's mount_kattr).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Kattr {
    set: u32,
    clr: u32,
    propagation: Option<Propagation>,
}

/// struct mount_attr's fields checked and turned into mount flags
/// (build_mount_kattr), with no ID mapping.
fn build_kattr(set: u64, clr: u64, propagation: u64) -> Result<Kattr, SyscallError> {
    let propagation = match propagation {
        0 => None,
        ms::UNBINDABLE => Some(Propagation::Unbindable),
        ms::PRIVATE => Some(Propagation::Private),
        ms::SLAVE => Some(Propagation::Slave),
        ms::SHARED => Some(Propagation::Shared),
        _ => return Err(SyscallError::InvalidArgument),
    };
    let valid = attr::FSMOUNT_VALID | attr::IDMAP;
    if (set | clr) & !valid != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if (set | clr) & attr::IDMAP != 0 {
        // ID-mapped mounts are not made here.
        return Err(SyscallError::InvalidArgument);
    }
    let mut kclr = mnt_flags(clr & !attr::ATIME) & !mnt::RELATIME;
    let mut kset = mnt_flags(set & !attr::ATIME) & !mnt::RELATIME;
    if clr & attr::ATIME != 0 {
        // Changing the atime type: every atime bit cleared, one type set.
        if clr & attr::ATIME != attr::ATIME {
            return Err(SyscallError::InvalidArgument);
        }
        kclr |= mnt::RELATIME | mnt::NOATIME;
        match set & attr::ATIME {
            attr::RELATIME => kset |= mnt::RELATIME,
            attr::NOATIME => kset |= mnt::NOATIME,
            attr::STRICTATIME => {}
            _ => return Err(SyscallError::InvalidArgument),
        }
    } else if set & attr::ATIME != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(Kattr {
        set: kset,
        clr: kclr,
        propagation,
    })
}

/// mount_setattr(dfd, path, flags, attr, size): change the flags and
/// propagation type of the mount whose root `path` is, and with
/// AT_RECURSIVE of every mount under it.
pub fn sys_mount_setattr(
    dfd: usize,
    path: usize,
    flags: usize,
    uattr: usize,
    usize_: usize,
) -> SyscallResult {
    let flags = flags as u32 as usize;
    if flags & !(AT_EMPTY_PATH | AT_RECURSIVE | AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if usize_ > PAGE_SIZE {
        return Err(SyscallError::ArgumentListTooLong);
    }
    if usize_ < MOUNT_ATTR_SIZE_VER0 {
        return Err(SyscallError::InvalidArgument);
    }
    may_mount()?;
    let kattr = read_mount_attr(uattr, usize_)?;
    let Some(kattr) = kattr else {
        return Ok(0);
    };
    let (_, canonical) = lookup_at(
        dfd,
        path,
        flags & AT_EMPTY_PATH != 0,
        false,
        flags & AT_SYMLINK_NOFOLLOW == 0,
    )?;
    setattr_at(&canonical, kattr, flags & AT_RECURSIVE != 0)
}

/// struct mount_attr from user memory (copy_struct_from_user): None when
/// it asks for nothing, else what it asks for, checked. Bytes past the
/// fields this kernel knows must be zero (E2BIG).
fn read_mount_attr(uattr: usize, size: usize) -> Result<Option<Kattr>, SyscallError> {
    let mut buf = alloc::vec![0u8; size];
    read_user_bytes(uattr, &mut buf)?;
    if buf[MOUNT_ATTR_SIZE_VER0..].iter().any(|&b| b != 0) {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let field = |i: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&buf[i * 8..i * 8 + 8]);
        u64::from_ne_bytes(b)
    };
    let (set, clr, propagation) = (field(0), field(1), field(2));
    if set == 0 && clr == 0 && propagation == 0 {
        return Ok(None);
    }
    build_kattr(set, clr, propagation).map(Some)
}

/// do_mount_setattr on the mount whose root is at `canonical`: EINVAL if
/// it is not one.
fn setattr_at(canonical: &str, kattr: Kattr, recursive: bool) -> SyscallResult {
    let (tree, at, _) = tree_and_path(canonical)?;
    if !is_mount_root(&tree, &at) {
        return Err(SyscallError::InvalidArgument);
    }
    let writers = |p: &str| super::mount::writing_under(p, &tree);
    tree.setattr(
        &at,
        kattr.set,
        kattr.clr,
        kattr.propagation,
        recursive,
        writers,
    )
    .map(|()| 0)
    .map_err(mount_error)
}

/// open_tree_attr(dfd, path, flags, attr, size): open_tree, then
/// mount_setattr on the new descriptor's mount; a failure closes it.
pub fn sys_open_tree_attr(
    dfd: usize,
    path: usize,
    flags: usize,
    uattr: usize,
    usize_: usize,
) -> SyscallResult {
    if uattr == 0 && usize_ != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let fd = sys_open_tree(dfd, path, flags)?;
    if uattr == 0 {
        return Ok(fd);
    }
    let result = (|| {
        if usize_ > PAGE_SIZE {
            return Err(SyscallError::ArgumentListTooLong);
        }
        if usize_ < MOUNT_ATTR_SIZE_VER0 {
            return Err(SyscallError::InvalidArgument);
        }
        let Some(kattr) = read_mount_attr(uattr, usize_)? else {
            return Ok(0);
        };
        let file = file_of(fd)?;
        let canonical = file.path.clone().unwrap_or_default();
        setattr_at(&canonical, kattr, flags & AT_RECURSIVE != 0)
    })();
    match result {
        Ok(_) => Ok(fd),
        Err(e) => {
            if let Some(proc) = process::current_process() {
                let _ = proc.file_table.lock().close(fd);
            }
            Err(e)
        }
    }
}

/// statmount's flag: the request names a descriptor's mount.
const STATMOUNT_BY_FD: u32 = 1;
/// listmount's flag: descending ids.
const LISTMOUNT_REVERSE: u32 = 1;
/// listmount's parent id meaning the caller's root mount.
const LSMT_ROOT: u64 = u64::MAX;
/// struct mnt_id_req's sizes.
const MNT_ID_REQ_SIZE_VER0: usize = 24;
const MNT_ID_REQ_SIZE_VER1: usize = 32;
/// Unique mount ids start above this (MNT_UNIQUE_ID_OFFSET).
const UNIQUE_OFFSET: u64 = 1 << 31;
/// The most ids listmount returns in one call.
const LISTMOUNT_MAX: usize = 1_000_000;

/// STATMOUNT_* request and result bits.
mod sm {
    pub const SB_BASIC: u64 = 0x1;
    pub const MNT_BASIC: u64 = 0x2;
    pub const PROPAGATE_FROM: u64 = 0x4;
    pub const MNT_ROOT: u64 = 0x8;
    pub const MNT_POINT: u64 = 0x10;
    pub const FS_TYPE: u64 = 0x20;
    pub const MNT_NS_ID: u64 = 0x40;
    pub const MNT_OPTS: u64 = 0x80;
    pub const FS_SUBTYPE: u64 = 0x100;
    pub const SB_SOURCE: u64 = 0x200;
    pub const OPT_ARRAY: u64 = 0x400;
    pub const OPT_SEC_ARRAY: u64 = 0x800;
    pub const SUPPORTED_MASK: u64 = 0x1000;
    pub const MNT_UIDMAP: u64 = 0x2000;
    pub const MNT_GIDMAP: u64 = 0x4000;
    /// Everything this kernel reports (STATMOUNT_SUPPORTED).
    pub const SUPPORTED: u64 = 0x7fff;
    /// The bits that ask for strings (STATMOUNT_STRING_REQ).
    pub const STRING_REQ: u64 = MNT_ROOT
        | MNT_POINT
        | FS_TYPE
        | MNT_OPTS
        | FS_SUBTYPE
        | SB_SOURCE
        | OPT_ARRAY
        | OPT_SEC_ARRAY
        | MNT_UIDMAP
        | MNT_GIDMAP;
    /// struct statmount's size: the strings follow it.
    pub const HEADER: usize = 512;
}

/// struct mnt_id_req, as read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IdReq {
    fd: u32,
    mnt_id: u64,
    param: u64,
    mnt_ns_id: u64,
}

/// struct mnt_id_req from user memory (copy_mnt_id_req): its size first
/// (E2BIG past a page, EINVAL under the first version's), then the fields;
/// bytes past the ones this kernel knows must be zero.
fn read_id_req(ptr: usize, by_fd: bool) -> Result<IdReq, SyscallError> {
    let size = super::userspace::read_user::<u32>(ptr)? as usize;
    if size > PAGE_SIZE {
        return Err(SyscallError::ArgumentListTooLong);
    }
    if size < MNT_ID_REQ_SIZE_VER0 {
        return Err(SyscallError::InvalidArgument);
    }
    let mut buf = alloc::vec![0u8; size.max(MNT_ID_REQ_SIZE_VER1)];
    read_user_bytes(ptr, &mut buf[..size])?;
    if buf[MNT_ID_REQ_SIZE_VER1..].iter().any(|&b| b != 0) {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let req = parse_id_req(&buf);
    check_id_req(&req, by_fd)?;
    Ok(req)
}

fn parse_id_req(buf: &[u8]) -> IdReq {
    let u64_at = |i: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&buf[i..i + 8]);
        u64::from_ne_bytes(b)
    };
    let mut fd = [0u8; 4];
    fd.copy_from_slice(&buf[4..8]);
    IdReq {
        fd: u32::from_ne_bytes(fd),
        mnt_id: u64_at(8),
        param: u64_at(16),
        mnt_ns_id: u64_at(24),
    }
}

/// copy_mnt_id_req's checks of the fields.
fn check_id_req(req: &IdReq, by_fd: bool) -> Result<(), SyscallError> {
    if by_fd {
        if req.mnt_id != 0 || req.mnt_ns_id != 0 {
            return Err(SyscallError::InvalidArgument);
        }
    } else {
        if req.fd != 0 && req.mnt_ns_id != 0 {
            return Err(SyscallError::InvalidArgument);
        }
        if req.mnt_id <= UNIQUE_OFFSET {
            return Err(SyscallError::InvalidArgument);
        }
    }
    Ok(())
}

/// The mount namespace a request names (grab_requested_mnt_ns): by id,
/// by a namespace descriptor, or the caller's; its tree, the root the
/// listing is seen from and its id. Only the caller's namespace is found
/// by id here (ENOENT for another: no table of namespaces by id is kept).
fn requested_tree(req: &IdReq) -> Result<(Arc<mount::Mounts>, String, u64), SyscallError> {
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let own = thread.namespaces().mnt.clone();
    let caller_root = || -> Result<String, SyscallError> {
        let root = vfs()?.get_root();
        Ok(if root.starts_with('/') {
            root
        } else {
            String::from("/")
        })
    };
    if req.mnt_ns_id != 0 {
        if req.mnt_ns_id != own.ident.id {
            return Err(SyscallError::ResourceNotFound);
        }
        return Ok((own.mounts.clone(), caller_root()?, own.ident.id));
    }
    if req.fd != 0 {
        let file = file_of(req.fd as usize)?;
        let ns = file
            .node
            .as_any()
            .and_then(|any| any.downcast_ref::<crate::fs::nsfs::NsFileNode>())
            .map(|n| n.ns().clone())
            .ok_or(SyscallError::InvalidArgument)?;
        let crate::process::ns::NsRef::Mnt(ns) = ns else {
            return Err(SyscallError::InvalidArgument);
        };
        let root = if Arc::ptr_eq(&ns, &own) {
            caller_root()?
        } else {
            String::from("/")
        };
        return Ok((ns.mounts.clone(), root, ns.ident.id));
    }
    Ok((own.mounts.clone(), caller_root()?, own.ident.id))
}

/// mnt_to_attr_flags: MOUNT_ATTR_* for mount flags `m`.
fn attr_flags(m: u32) -> u64 {
    let mut out = 0;
    for (f, a) in [
        (mnt::READONLY, attr::RDONLY),
        (mnt::NOSUID, attr::NOSUID),
        (mnt::NODEV, attr::NODEV),
        (mnt::NOEXEC, attr::NOEXEC),
        (mnt::NODIRATIME, attr::NODIRATIME),
        (mnt::NOSYMFOLLOW, attr::NOSYMFOLLOW),
    ] {
        if m & f != 0 {
            out |= a;
        }
    }
    if m & mnt::NOATIME != 0 {
        out |= attr::NOATIME;
    } else if m & mnt::RELATIME == 0 {
        out |= attr::STRICTATIME;
    }
    out
}

/// What statmount reports of one mount.
struct MountFacts<'a> {
    mount: &'a mount::Mount,
    parent_unique: u64,
    parent_id: u32,
    /// Its mount point as the caller's root sees it, if it can.
    point: Option<String>,
    /// Its namespace's id, if it is in one.
    ns_id: Option<u64>,
}

/// struct statmount and its strings for `facts`, as `mask` asks
/// (do_statmount), for a buffer of `bufsize` bytes: the header and the
/// strings, or EOVERFLOW when a string does not fit.
fn statmount_bytes(
    facts: &MountFacts<'_>,
    mask: u64,
    bufsize: usize,
) -> Result<([u8; sm::HEADER], Vec<u8>), SyscallError> {
    let mut h = [0u8; sm::HEADER];
    let put32 = |h: &mut [u8; sm::HEADER], at: usize, v: u32| {
        h[at..at + 4].copy_from_slice(&v.to_ne_bytes())
    };
    let put64 = |h: &mut [u8; sm::HEADER], at: usize, v: u64| {
        h[at..at + 8].copy_from_slice(&v.to_ne_bytes())
    };
    let m = facts.mount;
    let mut out_mask = 0u64;
    let flags = m.flags();
    if mask & sm::MNT_BASIC != 0 {
        out_mask |= sm::MNT_BASIC;
        put64(&mut h, 40, m.ids().unique);
        put64(&mut h, 48, facts.parent_unique);
        put32(&mut h, 56, m.ids().id);
        put32(&mut h, 60, facts.parent_id);
        put64(&mut h, 64, attr_flags(flags));
        let prop = if flags & mnt::UNBINDABLE != 0 {
            ms::UNBINDABLE
        } else {
            ms::PRIVATE
        };
        put64(&mut h, 72, prop);
    }
    if mask & sm::SB_BASIC != 0 {
        out_mask |= sm::SB_BASIC;
        put32(&mut h, 16, 0);
        put32(&mut h, 20, m.ids().dev);
        let magic = m.root().statfs().map(|s| s.fs_type).unwrap_or(0);
        put64(&mut h, 24, magic);
        put32(&mut h, 32, u32::from(m.sb_readonly()));
    }
    if mask & sm::PROPAGATE_FROM != 0 {
        out_mask |= sm::PROPAGATE_FROM;
    }
    let mut strs: Vec<u8> = Vec::new();
    let mut string = |h: &mut [u8; sm::HEADER],
                      out_mask: &mut u64,
                      flag: u64,
                      at: usize,
                      value: &str|
     -> Result<(), SyscallError> {
        if mask & flag == 0 {
            return Ok(());
        }
        // An empty string at the start for the offsets left unset.
        if strs.is_empty() {
            strs.push(0);
        }
        if value.is_empty() {
            return Ok(());
        }
        let start = strs.len();
        strs.extend_from_slice(value.as_bytes());
        if sm::HEADER + strs.len() >= bufsize {
            return Err(SyscallError::Overflow);
        }
        strs.push(0);
        *out_mask |= flag;
        h[at..at + 4].copy_from_slice(&(start as u32).to_ne_bytes());
        Ok(())
    };
    let fs_type = mount::linux_fs_type(m.fs().name());
    string(&mut h, &mut out_mask, sm::FS_TYPE, 36, fs_type)?;
    string(&mut h, &mut out_mask, sm::MNT_ROOT, 104, m.root_path())?;
    if let Some(point) = &facts.point {
        string(&mut h, &mut out_mask, sm::MNT_POINT, 108, point)?;
    }
    // No filesystem here shows options, has a subtype or an ID mapping.
    string(&mut h, &mut out_mask, sm::SB_SOURCE, 124, m.source())?;
    if mask & sm::MNT_NS_ID != 0 {
        if let Some(id) = facts.ns_id {
            out_mask |= sm::MNT_NS_ID;
            put64(&mut h, 112, id);
        }
    }
    if mask & sm::SUPPORTED_MASK != 0 {
        out_mask |= sm::SUPPORTED_MASK;
        put64(&mut h, 144, sm::SUPPORTED);
    }
    put64(&mut h, 8, out_mask);
    let copysize = bufsize.min(sm::HEADER);
    put32(&mut h, 0, (copysize + strs.len()) as u32);
    Ok((h, strs))
}

/// statmount(req, buf, bufsize, flags): what a mount is, by unique id
/// in a namespace or (STATMOUNT_BY_FD) by a descriptor of a file on it.
pub fn sys_statmount(req: usize, buf: usize, bufsize: usize, flags: usize) -> SyscallResult {
    let flags = flags as u32;
    if flags & !STATMOUNT_BY_FD != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let by_fd = flags & STATMOUNT_BY_FD != 0;
    let r = read_id_req(req, by_fd)?;
    let mut mask = r.param;
    let (tree, root, ns_id, unique) = if by_fd {
        let file = file_of(r.fd as usize)?;
        let path = file.path.clone().ok_or(SyscallError::InvalidArgument)?;
        let canonical = vfs()?.locate(&path, false).map(|(_, c)| c).unwrap_or(path);
        let (tree, at, detached) = tree_and_path(&canonical)?;
        let point = tree.mount_point_of(&at);
        let unique = tree
            .mount_at(&point)
            .ok_or(SyscallError::InvalidArgument)?
            .ids()
            .unique;
        let (_, root, id) = requested_tree(&IdReq {
            fd: 0,
            mnt_id: 0,
            param: 0,
            mnt_ns_id: 0,
        })?;
        if detached.is_some() {
            // In no namespace: no mount point or namespace id.
            mask &= !(sm::MNT_POINT | sm::MNT_NS_ID);
            (tree, String::from("/"), None, unique)
        } else {
            (tree, root, Some(id), unique)
        }
    } else {
        let (tree, root, id) = requested_tree(&r)?;
        (tree, root, Some(id), r.mnt_id)
    };
    let full = tree.listing("/");
    let (_, mount, parent_id) = full
        .iter()
        .find(|(_, m, _)| m.ids().unique == unique)
        .cloned()
        .ok_or(SyscallError::ResourceNotFound)?;
    let parent_unique = full
        .iter()
        .find(|(_, m, _)| m.ids().id == parent_id)
        .map_or(mount.ids().unique, |(_, m, _)| m.ids().unique);
    let point = tree
        .listing(&root)
        .into_iter()
        .find(|(_, m, _)| m.ids().unique == unique)
        .map(|(p, _, _)| p);
    if !by_fd && point.is_none() && !Caller::current().capable(CAP_SYS_ADMIN) {
        return Err(SyscallError::OperationNotPermitted);
    }
    if mask & sm::STRING_REQ != 0 && bufsize == sm::HEADER {
        return Err(SyscallError::Overflow);
    }
    let facts = MountFacts {
        mount: &mount,
        parent_unique,
        parent_id,
        point,
        ns_id,
    };
    let (header, strs) = statmount_bytes(&facts, mask, bufsize)?;
    if !strs.is_empty() {
        super::userspace::write_user_bytes(buf + sm::HEADER, &strs)?;
    }
    super::userspace::write_user_bytes(buf, &header[..bufsize.min(sm::HEADER)])?;
    Ok(0)
}

/// The unique ids listmount returns from `full` (a namespace's mounts as
/// `(point, unique id)`): every mount under `origin`'s place but
/// `origin`, ascending (descending with `reverse`), after `last` (0 for
/// the first call), at most `nr`.
fn list_ids(full: &[(String, u64)], origin: u64, last: u64, reverse: bool, nr: usize) -> Vec<u64> {
    let Some((at, _)) = full.iter().find(|(_, id)| *id == origin) else {
        return Vec::new();
    };
    let mut ids: Vec<u64> = full
        .iter()
        .filter(|(p, id)| *id != origin && crate::fs::path_is_under(p, at))
        .map(|(_, id)| *id)
        .filter(|&id| last == 0 || if reverse { id < last } else { id > last })
        .collect();
    ids.sort_unstable();
    if reverse {
        ids.reverse();
    }
    ids.truncate(nr);
    ids
}

/// listmount(req, mnt_ids, nr_mnt_ids, flags): the unique ids of the
/// mounts under one (or under the caller's root), in id order.
pub fn sys_listmount(req: usize, ids: usize, nr: usize, flags: usize) -> SyscallResult {
    let flags = flags as u32;
    if flags & !LISTMOUNT_REVERSE != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if nr > LISTMOUNT_MAX {
        return Err(SyscallError::Overflow);
    }
    let r = read_id_req(req, false)?;
    if r.param != 0 && r.param <= UNIQUE_OFFSET {
        return Err(SyscallError::InvalidArgument);
    }
    let (tree, root, _) = requested_tree(&r)?;
    let full: Vec<(String, u64)> = tree
        .listing("/")
        .into_iter()
        .map(|(p, m, _)| (p, m.ids().unique))
        .collect();
    let origin = if r.mnt_id == LSMT_ROOT {
        let point = tree.mount_point_of(&root);
        tree.mount_at(&point)
            .ok_or(SyscallError::ResourceNotFound)?
            .ids()
            .unique
    } else {
        if !full.iter().any(|(_, id)| *id == r.mnt_id) {
            return Err(SyscallError::ResourceNotFound);
        }
        let reachable = tree
            .listing(&root)
            .iter()
            .any(|(_, m, _)| m.ids().unique == r.mnt_id);
        if !reachable && !Caller::current().capable(CAP_SYS_ADMIN) {
            return Err(SyscallError::OperationNotPermitted);
        }
        r.mnt_id
    };
    let out = list_ids(&full, origin, r.param, flags & LISTMOUNT_REVERSE != 0, nr);
    if !out.is_empty() {
        super::userspace::write_user_slice(ids, &out)?;
    }
    Ok(out.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fsmount's attribute flags become mount flags, relatime unless
    /// another atime type is asked for.
    #[test]
    fn attribute_flags_become_mount_flags() {
        assert_eq!(mnt_flags(0), mnt::RELATIME);
        assert_eq!(
            mnt_flags(attr::RDONLY | attr::NOSUID | attr::NOATIME),
            mnt::READONLY | mnt::NOSUID | mnt::NOATIME
        );
        assert_eq!(mnt_flags(attr::STRICTATIME | attr::NOEXEC), mnt::NOEXEC);
        assert_eq!(
            mnt_flags(attr::NODIRATIME | attr::NOSYMFOLLOW),
            mnt::RELATIME | mnt::NODIRATIME | mnt::NOSYMFOLLOW
        );
    }

    /// build_mount_kattr's checks: one propagation type, known flags, and
    /// an atime type changed only by clearing all of the atime bits.
    #[test]
    fn mount_attr_is_checked_as_linux_checks_it() {
        assert_eq!(
            build_kattr(attr::RDONLY, attr::NOEXEC, ms::PRIVATE),
            Ok(Kattr {
                set: mnt::READONLY,
                clr: mnt::NOEXEC,
                propagation: Some(Propagation::Private),
            })
        );
        let inval = Err(SyscallError::InvalidArgument);
        assert_eq!(build_kattr(0, 0, ms::PRIVATE | ms::SLAVE), inval);
        assert_eq!(build_kattr(1 << 40, 0, 0), inval);
        assert_eq!(build_kattr(attr::NOATIME, 0, 0), inval);
        assert_eq!(build_kattr(attr::NOATIME, attr::NOATIME, 0), inval);
        assert_eq!(build_kattr(0x30, attr::ATIME, 0), inval);
        assert_eq!(
            build_kattr(attr::NOATIME, attr::ATIME, 0),
            Ok(Kattr {
                set: mnt::NOATIME,
                clr: mnt::RELATIME | mnt::NOATIME,
                propagation: None,
            })
        );
        assert_eq!(
            build_kattr(attr::STRICTATIME, attr::ATIME, 0).map(|k| k.set),
            Ok(0)
        );
        assert_eq!(build_kattr(attr::IDMAP, 0, 0), inval);
    }

    /// copy_mnt_id_req's field checks and the layout read.
    #[test]
    fn mount_id_requests_are_read_and_checked() {
        let mut buf = [0u8; 32];
        buf[0..4].copy_from_slice(&32u32.to_ne_bytes());
        buf[4..8].copy_from_slice(&5u32.to_ne_bytes());
        buf[8..16].copy_from_slice(&(UNIQUE_OFFSET + 3).to_ne_bytes());
        buf[16..24].copy_from_slice(&sm::MNT_BASIC.to_ne_bytes());
        let r = parse_id_req(&buf);
        assert_eq!(
            r,
            IdReq {
                fd: 5,
                mnt_id: UNIQUE_OFFSET + 3,
                param: sm::MNT_BASIC,
                mnt_ns_id: 0,
            }
        );
        assert_eq!(check_id_req(&r, false), Ok(()));
        let inval = Err(SyscallError::InvalidArgument);
        assert_eq!(check_id_req(&r, true), inval);
        assert_eq!(
            check_id_req(
                &IdReq {
                    mnt_id: UNIQUE_OFFSET,
                    ..r
                },
                false
            ),
            inval
        );
        assert_eq!(check_id_req(&IdReq { mnt_ns_id: 9, ..r }, false), inval);
        assert_eq!(check_id_req(&IdReq { mnt_id: 0, ..r }, true), Ok(()));
    }

    /// listmount: the mounts under the origin's place, not the origin, in
    /// id order either way, after the last id, at most `nr`.
    #[test]
    fn listmount_lists_below_the_origin_in_id_order() {
        let s = |p: &str, id: u64| (String::from(p), id);
        let full = [
            s("/", 10),
            s("/a", 12),
            s("/a/b", 11),
            s("/c", 14),
            s("/a", 13),
        ];
        assert_eq!(
            list_ids(&full, 10, 0, false, 10),
            alloc::vec![11, 12, 13, 14]
        );
        assert_eq!(list_ids(&full, 12, 0, false, 10), alloc::vec![11, 13]);
        assert_eq!(list_ids(&full, 10, 12, false, 10), alloc::vec![13, 14]);
        assert_eq!(list_ids(&full, 10, 0, true, 2), alloc::vec![14, 13]);
        assert_eq!(list_ids(&full, 10, 13, true, 10), alloc::vec![12, 11]);
        assert!(list_ids(&full, 99, 0, false, 10).is_empty());
    }

    /// mnt_to_attr_flags.
    #[test]
    fn mount_flags_become_attribute_flags() {
        assert_eq!(attr_flags(mnt::RELATIME), 0);
        assert_eq!(attr_flags(0), attr::STRICTATIME);
        assert_eq!(
            attr_flags(mnt::READONLY | mnt::NOATIME | mnt::NOSYMFOLLOW),
            attr::RDONLY | attr::NOATIME | attr::NOSYMFOLLOW
        );
    }
}

//! clone(2) and clone3(2), as Linux 7.3's `kernel/fork.c`: both build one
//! [`CloneRequest`] (Linux's `kernel_clone_args`) and go through
//! [`kernel_clone`], which checks it in copy_process's order and creates a
//! thread of the caller's process (CLONE_THREAD) or a new process.

#[cfg(feature = "alloc")]
use crate::process::thread::ThreadFs;
use crate::{
    arch::context::ThreadContext,
    process::{self, ns, thread::ThreadBuilder},
    syscall::{userspace::validate_user_ptr, SyscallError},
};

// clone's flags (its low 32 bits).
/// clone's low byte: the signal the parent gets when a new process exits.
const CSIGNAL: u64 = 0xFF;
/// clone3 only (a CSIGNAL bit clone3 reuses): a new time namespace.
const CLONE_NEWTIME: u64 = 0x0000_0080;
const CLONE_VM: u64 = 0x0000_0100;
const CLONE_FS: u64 = 0x0000_0200;
const CLONE_FILES: u64 = 0x0000_0400;
const CLONE_SIGHAND: u64 = 0x0000_0800;
/// Return a pidfd for the child (a PIDFD_THREAD one for a thread), as
/// Linux 5.2+: through `parent_tid` for clone, `pidfd` for clone3.
const CLONE_PIDFD: u64 = 0x0000_1000;
const CLONE_VFORK: u64 = 0x0000_4000;
const CLONE_PARENT: u64 = 0x0000_8000;
const CLONE_THREAD: u64 = 0x0001_0000;
const CLONE_NEWNS: u64 = 0x0002_0000;
/// Share SysV semaphore undo lists (musl always sets this for pthreads).
/// Accepted but ignored -- VeridianOS has no SysV semaphore undo tracking.
const CLONE_SYSVSEM: u64 = 0x0004_0000;
const CLONE_SETTLS: u64 = 0x0008_0000;
const CLONE_PARENT_SETTID: u64 = 0x0010_0000;
const CLONE_CHILD_CLEARTID: u64 = 0x0020_0000;
/// Deprecated flag set by musl's pthread_create (ignored on Linux 2.6+).
/// Accepted but ignored by clone; clone3 refuses it.
const CLONE_DETACHED: u64 = 0x0040_0000;
const CLONE_CHILD_SETTID: u64 = 0x0100_0000;
/// Share the I/O context: accepted, as no task has one (kcmp's KCMP_IO
/// finds every pair equal).
const CLONE_IO: u64 = 0x8000_0000;
/// Every flag clone(2) can name (Linux's CLONE_LEGACY_FLAGS).
const CLONE_LEGACY_FLAGS: u64 = 0xFFFF_FFFF;

// clone3's flags past clone's 32 bits.
/// Handlers not SIG_IGN start as SIG_DFL in the child.
const CLONE_CLEAR_SIGHAND: u64 = 1 << 32;
/// Start the child in the cgroup `cgroup` names.
const CLONE_INTO_CGROUP: u64 = 1 << 33;
/// The child is reaped when it exits: no zombie, no signal (Linux 7.0).
const CLONE_AUTOREAP: u64 = 1 << 34;
/// The child has no_new_privs set (Linux 7.0).
const CLONE_NNP: u64 = 1 << 35;
/// SIGKILL the child when its clone pidfd is last closed (Linux 7.0).
const CLONE_PIDFD_AUTOKILL: u64 = 1 << 36;
/// A new, empty mount namespace (Linux 7.0; implies CLONE_NEWNS).
const CLONE_EMPTY_MNTNS: u64 = 1 << 37;

/// `struct clone_args` sizes: its first version (to `tls`), the second
/// (to `set_tid_size`) and the third (to `cgroup`), Linux's
/// CLONE_ARGS_SIZE_VER0..2.
const CLONE_ARGS_SIZE_VER0: usize = 64;
const CLONE_ARGS_SIZE_VER2: usize = 88;
/// Linux's MAX_PID_NS_LEVEL: the most set_tid entries clone3 takes.
const MAX_PID_NS_LEVEL: u64 = 32;
/// The largest size clone3 reads (PAGE_SIZE).
const CLONE_ARGS_MAX: usize = 4096;

/// Upper bound of the user-space address range.  Any stack pointer must be
/// strictly below this address.  The value matches the canonical user-space
/// limit on x86_64; AArch64 and RISC-V use the same logical limit via
/// `validate_user_ptr`.
use crate::mm::user_layout::user_space_end;

/// What one clone asks for (Linux's `kernel_clone_args`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CloneRequest {
    /// The flags, without clone's exit signal byte.
    flags: u64,
    /// The signal the parent gets when a new process exits (0: none).
    exit_signal: u64,
    /// The child's stack pointer; 0 runs it on the caller's.
    stack: usize,
    parent_tid: usize,
    child_tid: usize,
    tls: usize,
    /// Where CLONE_PIDFD stores the pidfd.
    pidfd: usize,
    /// clone3's set_tid: the ID the child is to have in its PID namespace
    /// (`set_tid[0]`) and in each one above it, the first `set_tid_levels`.
    set_tid: [i32; MAX_PID_NS_LEVEL as usize],
    /// How many set_tid entries there are.
    set_tid_levels: u64,
    /// clone3's CLONE_INTO_CGROUP descriptor.
    cgroup: u64,
}

impl CloneRequest {
    fn on(&self, flag: u64) -> bool {
        self.flags & flag != 0
    }
}

/// clone(flags, newsp, parent_tid, child_tid, tls) (the dispatcher puts
/// AArch64's and RISC-V's tls/child_tid order right): its low 32 bits are
/// the flags, their low byte the exit signal, and CLONE_PIDFD's pidfd goes
/// to `parent_tid`, as Linux's sys_clone.
pub fn sys_thread_clone(
    flags: usize,
    newsp: usize,
    parent_tid_ptr: usize,
    child_tid_ptr: usize,
    tls: usize,
) -> Result<usize, SyscallError> {
    let flags = flags as u64 & CLONE_LEGACY_FLAGS;
    kernel_clone(CloneRequest {
        flags: flags & !CSIGNAL,
        exit_signal: flags & CSIGNAL,
        stack: newsp,
        parent_tid: parent_tid_ptr,
        child_tid: child_tid_ptr,
        tls,
        pidfd: parent_tid_ptr,
        ..CloneRequest::default()
    })
}

/// `struct clone_args` as read, before its checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CloneArgs {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

impl CloneArgs {
    /// The fields of a `struct clone_args` whose first `raw.len()` bytes
    /// are `raw` (the rest zero, as copy_struct_from_user fills them).
    fn decode(raw: &[u8]) -> Self {
        let mut full = [0u8; CLONE_ARGS_SIZE_VER2];
        let n = raw.len().min(CLONE_ARGS_SIZE_VER2);
        full[..n].copy_from_slice(&raw[..n]);
        let word = |i: usize| {
            let mut b = [0u8; 8];
            b.copy_from_slice(&full[i * 8..i * 8 + 8]);
            u64::from_ne_bytes(b)
        };
        Self {
            flags: word(0),
            pidfd: word(1),
            child_tid: word(2),
            parent_tid: word(3),
            exit_signal: word(4),
            stack: word(5),
            stack_size: word(6),
            tls: word(7),
            set_tid: word(8),
            set_tid_size: word(9),
            cgroup: word(10),
        }
    }

    /// copy_clone_args_from_user's checks of the copied struct (`size` is
    /// what the caller passed): set_tid_size above MAX_PID_NS_LEVEL, a
    /// set_tid array without a size or a size without one, an exit signal
    /// past CSIGNAL, and CLONE_INTO_CGROUP with a descriptor past INT_MAX
    /// or a struct too short to have one are EINVAL.
    fn check(&self, size: usize) -> Result<(), SyscallError> {
        let bad = self.set_tid_size > MAX_PID_NS_LEVEL
            || (self.set_tid == 0 && self.set_tid_size > 0)
            || (self.set_tid != 0 && self.set_tid_size == 0)
            || self.exit_signal & !CSIGNAL != 0
            || (self.flags & CLONE_INTO_CGROUP != 0
                && (self.cgroup > i32::MAX as u64 || size < CLONE_ARGS_SIZE_VER2));
        if bad {
            Err(SyscallError::InvalidArgument)
        } else {
            Ok(())
        }
    }
}

/// clone3_args_valid, which also turns clone3's stack (its lowest address
/// and size) into the stack pointer the child starts with: unknown flags,
/// CLONE_DETACHED and the CSIGNAL bits other than CLONE_NEWTIME (clone3
/// keeps them for reuse), CLONE_SIGHAND with CLONE_CLEAR_SIGHAND, an exit
/// signal for a thread or a CLONE_PARENT child, a stack without a size or
/// a size without a stack, and a stack not wholly in user space are EINVAL.
fn clone3_request(
    args: &CloneArgs,
    set_tid: [i32; MAX_PID_NS_LEVEL as usize],
) -> Result<CloneRequest, SyscallError> {
    let known = CLONE_LEGACY_FLAGS
        | CLONE_CLEAR_SIGHAND
        | CLONE_INTO_CGROUP
        | CLONE_AUTOREAP
        | CLONE_NNP
        | CLONE_PIDFD_AUTOKILL
        | CLONE_EMPTY_MNTNS;
    let flags = args.flags;
    let stack = match (args.stack, args.stack_size) {
        (0, 0) => Some(0),
        (0, _) | (_, 0) => None,
        (base, size) => base
            .checked_add(size)
            .filter(|&top| top <= user_space_end() as u64)
            .map(|top| top as usize),
    };
    let valid = flags & !known == 0
        && flags & (CLONE_DETACHED | (CSIGNAL & !CLONE_NEWTIME)) == 0
        && flags & (CLONE_SIGHAND | CLONE_CLEAR_SIGHAND) != CLONE_SIGHAND | CLONE_CLEAR_SIGHAND
        && !(flags & (CLONE_THREAD | CLONE_PARENT) != 0 && args.exit_signal != 0);
    match stack {
        Some(stack) if valid => Ok(CloneRequest {
            flags,
            exit_signal: args.exit_signal,
            stack,
            parent_tid: args.parent_tid as usize,
            child_tid: args.child_tid as usize,
            tls: args.tls as usize,
            pidfd: args.pidfd as usize,
            set_tid,
            set_tid_levels: args.set_tid_size,
            cgroup: args.cgroup,
        }),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// clone3(uargs, size), as Linux 7.3's sys_clone3: `struct clone_args` is
/// read as copy_struct_from_user reads an extensible struct (E2BIG above a
/// page or for nonzero bytes past the fields known here, EINVAL below its
/// first version, EFAULT), checked ([`CloneArgs::check`]), its set_tid
/// array read (EFAULT), then checked as clone3_args_valid
/// ([`clone3_request`]) and cloned as clone is.
pub fn sys_clone3(uargs: usize, size: usize) -> Result<usize, SyscallError> {
    if size > CLONE_ARGS_MAX {
        return Err(SyscallError::ArgumentListTooLong);
    }
    if size < CLONE_ARGS_SIZE_VER0 {
        return Err(SyscallError::InvalidArgument);
    }
    // The bytes past the struct first, as check_zeroed_user.
    if size > CLONE_ARGS_SIZE_VER2 {
        let mut tail = alloc::vec![0u8; size - CLONE_ARGS_SIZE_VER2];
        let past = uargs
            .checked_add(CLONE_ARGS_SIZE_VER2)
            .ok_or(SyscallError::InvalidPointer)?;
        super::userspace::read_user_bytes(past, &mut tail)?;
        if tail.iter().any(|&b| b != 0) {
            return Err(SyscallError::ArgumentListTooLong);
        }
    }
    let mut raw = [0u8; CLONE_ARGS_SIZE_VER2];
    let len = size.min(CLONE_ARGS_SIZE_VER2);
    super::userspace::read_user_bytes(uargs, &mut raw[..len])?;
    let args = CloneArgs::decode(&raw[..len]);
    args.check(size)?;
    // The whole array is read (EFAULT); its first entry is for the
    // child's PID namespace, the next for the one above, and so on.
    let mut set_tid = [0i32; MAX_PID_NS_LEVEL as usize];
    if args.set_tid_size != 0 {
        let mut ids = alloc::vec![0u8; args.set_tid_size as usize * 4];
        super::userspace::read_user_bytes(args.set_tid as usize, &mut ids)?;
        for (slot, b) in set_tid.iter_mut().zip(ids.chunks_exact(4)) {
            *slot = i32::from_ne_bytes([b[0], b[1], b[2], b[3]]);
        }
    }
    kernel_clone(clone3_request(&args, set_tid)?)
}

/// copy_process's flag rules, in its order: CLONE_NEWNS with CLONE_FS, a
/// thread without CLONE_SIGHAND, CLONE_SIGHAND without CLONE_VM,
/// CLONE_PIDFD with CLONE_DETACHED, CLONE_AUTOREAP for a thread, with
/// CLONE_PARENT or with an exit signal, CLONE_NNP for a thread, and
/// CLONE_PIDFD_AUTOKILL without CLONE_PIDFD or CLONE_AUTOREAP or for a
/// thread are EINVAL; CLONE_PIDFD_AUTOKILL without CLONE_NNP needs
/// CAP_SYS_ADMIN in the caller's own user namespace (`admin`; EPERM),
/// since the child could gain privilege after it starts.
fn copy_process_checks(req: &CloneRequest, admin: bool) -> Result<(), SyscallError> {
    let on = |flag| req.on(flag);
    let invalid = (on(CLONE_NEWNS) && on(CLONE_FS))
        || (on(CLONE_THREAD) && !on(CLONE_SIGHAND))
        || (on(CLONE_SIGHAND) && !on(CLONE_VM))
        || (on(CLONE_THREAD) && on(ns::CLONE_NEWUSER | ns::CLONE_NEWPID))
        || (on(CLONE_PIDFD) && on(CLONE_DETACHED))
        || (on(CLONE_AUTOREAP) && (on(CLONE_THREAD) || on(CLONE_PARENT) || req.exit_signal != 0))
        || (on(CLONE_NNP) && on(CLONE_THREAD))
        || (on(CLONE_PIDFD_AUTOKILL)
            && (!on(CLONE_PIDFD) || !on(CLONE_AUTOREAP) || on(CLONE_THREAD)));
    if invalid {
        return Err(SyscallError::InvalidArgument);
    }
    if on(CLONE_PIDFD_AUTOKILL) && !on(CLONE_NNP) && !admin {
        return Err(SyscallError::OperationNotPermitted);
    }
    Ok(())
}

/// The flags each kind of clone supports here; anything else
/// (CLONE_PTRACE, CLONE_UNTRACED) is EINVAL.
const PROCESS_FLAGS: u64 = CLONE_VM
    | CLONE_FS
    | CLONE_FILES
    | CLONE_SIGHAND
    | CLONE_PARENT
    | CLONE_VFORK
    | CLONE_SYSVSEM
    | CLONE_SETTLS
    | CLONE_PARENT_SETTID
    | CLONE_CHILD_SETTID
    | CLONE_CHILD_CLEARTID
    | CLONE_DETACHED
    | CLONE_PIDFD
    | CLONE_IO
    | CLONE_CLEAR_SIGHAND
    | CLONE_INTO_CGROUP
    | CLONE_AUTOREAP
    | CLONE_NNP
    | CLONE_PIDFD_AUTOKILL
    | CLONE_EMPTY_MNTNS
    | ns::CLONE_NS_ALL;
const THREAD_FLAGS: u64 = CLONE_VM
    | CLONE_FS
    | CLONE_FILES
    | CLONE_SIGHAND
    | CLONE_THREAD
    | CLONE_PARENT
    | CLONE_SYSVSEM
    | CLONE_SETTLS
    | CLONE_PARENT_SETTID
    | CLONE_CHILD_CLEARTID
    | CLONE_DETACHED
    | CLONE_CHILD_SETTID
    | CLONE_PIDFD
    | CLONE_IO
    | CLONE_INTO_CGROUP
    | CLONE_EMPTY_MNTNS
    | ns::CLONE_NS_ALL;

/// kernel_clone's own checks, then copy_process's: CLONE_EMPTY_MNTNS
/// implies CLONE_NEWNS; CLONE_PIDFD and CLONE_PARENT_SETTID storing to one
/// address, and an exit signal past the last signal, are EINVAL.
fn request_checks(req: &mut CloneRequest, admin: bool) -> Result<(), SyscallError> {
    if req.on(CLONE_EMPTY_MNTNS) {
        req.flags |= CLONE_NEWNS;
    }
    if req.on(CLONE_PIDFD) && req.on(CLONE_PARENT_SETTID) && req.pidfd == req.parent_tid {
        return Err(SyscallError::InvalidArgument);
    }
    if req.exit_signal > crate::process::signals::NSIG as u64 {
        return Err(SyscallError::InvalidArgument);
    }
    copy_process_checks(req, admin)?;
    let supported = if req.on(CLONE_THREAD) {
        THREAD_FLAGS
    } else {
        PROCESS_FLAGS
    };
    if req.flags & !supported != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(())
}

/// set_tid's rules for the initial PID namespace's entry, the kernel id
/// (Linux's alloc_pid at level 0): an ID below 1 or not below pid_max is
/// EINVAL; choosing one needs CAP_CHECKPOINT_RESTORE or CAP_SYS_ADMIN over
/// the initial user namespace (checkpoint_restore_ns_capable;
/// `may_choose`; EPERM). The other levels' are `pidns::reserve`'s.
fn set_tid_checks(tid: i32, may_choose: bool) -> Result<u64, SyscallError> {
    if tid < 1 || tid as u64 >= crate::process::PID_MAX {
        return Err(SyscallError::InvalidArgument);
    }
    if !may_choose {
        return Err(SyscallError::OperationNotPermitted);
    }
    Ok(tid as u64)
}

/// Linux's kernel_clone: check `req` ([`request_checks`]; a thread of a
/// process whose children go to another PID namespace than its own is
/// EINVAL), make the namespaces (each new one needing CREATE on the one it
/// is made from, EPERM; ADR 0024, step 3), reserve the child's number in each
/// PID namespace level below the initial one and claim its kernel id, with
/// set_tid's choices ([`pid_numbers`]), refuse CLONE_INTO_CGROUP (EBADF:
/// there is no cgroup filesystem, so no descriptor names a cgroup; Linux
/// checks it after the ID and the pidfd), and make the thread or process.
fn kernel_clone(mut req: CloneRequest) -> Result<usize, SyscallError> {
    use crate::process::linux_caps::{CAP_CHECKPOINT_RESTORE, CAP_SYS_ADMIN};
    let caller = process::creds::Caller::current();
    request_checks(&mut req, caller.ns_capable(&caller.userns, CAP_SYS_ADMIN))?;
    // copy_process: an init (SIGNAL_UNKILLABLE), or a process with no
    // parent, makes no siblings, which nothing would reap.
    if req.on(CLONE_PARENT) {
        let current = process::current_process().ok_or(SyscallError::InvalidState)?;
        if current.is_ns_init() || current.parent().is_none() {
            return Err(SyscallError::InvalidArgument);
        }
    }
    // copy_process refuses a new user namespace sharing the filesystem
    // state; then copy_creds makes it (create_user_ns), and copy_namespaces
    // the others, which belong to it and which its root may make.
    if req.on(ns::CLONE_NEWUSER) && req.on(CLONE_FS) {
        return Err(SyscallError::InvalidArgument);
    }
    let current_ns = process::current_thread()
        .ok_or(SyscallError::InvalidState)?
        .namespaces();
    // The object capabilities on what each new namespace is made from
    // (ADR 0024, step 3), before anything is made.
    if !crate::cap::authority::current_may_create_namespaces(
        &current_ns,
        &caller.userns,
        req.flags & ns::CLONE_NS_ALL,
    ) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let user_ns = if req.on(ns::CLONE_NEWUSER) {
        Some(super::namespaces::new_user_ns(&caller)?)
    } else {
        None
    };
    let owner = user_ns.clone().unwrap_or_else(|| caller.userns.clone());
    let active = process::pidns::current();
    if req.on(CLONE_THREAD) && active.ident.id != current_ns.pid_for_children.ident.id {
        return Err(SyscallError::InvalidArgument);
    }
    // copy_namespaces: over the user namespace the child's credentials
    // are in, whose root a new one's creator is.
    let admin = caller.ns_capable(&owner, CAP_SYS_ADMIN);
    let namespaces = ns::for_clone(
        &current_ns,
        req.flags,
        admin,
        &owner,
        &active,
        caller.creds.euid,
    )
    .map_err(|e| match e {
        ns::CloneNsError::Permission => SyscallError::OperationNotPermitted,
        ns::CloneNsError::Invalid => SyscallError::InvalidArgument,
        ns::CloneNsError::NoSpace => SyscallError::NoSpace,
    })?;
    let may_choose = |owner: &process::userns::UserNs| {
        caller.ns_capable(owner, CAP_CHECKPOINT_RESTORE) || caller.ns_capable(owner, CAP_SYS_ADMIN)
    };
    let (numbers, claim) = pid_numbers(&req, &namespaces.pid_for_children, may_choose)?;
    if req.on(CLONE_INTO_CGROUP) {
        return Err(SyscallError::BadFileDescriptor);
    }
    let id = claim.as_ref().map(|c| c.id());
    if req.on(CLONE_THREAD) {
        thread_clone(&req, id, numbers, namespaces)
    } else {
        process_clone(&req, id, numbers, namespaces, user_ns)
    }
}

/// The child's numbers (Linux's alloc_pid): with set_tid, more entries
/// than PID namespace levels are EINVAL; then each level from the child's
/// namespace up, as `pidns::reserve` and, for the initial namespace's
/// entry, [`set_tid_checks`] and the kernel id's claim (EEXIST when taken).
fn pid_numbers(
    req: &CloneRequest,
    ns: &alloc::sync::Arc<process::pidns::PidNs>,
    may_choose: impl Fn(&process::userns::UserNs) -> bool,
) -> Result<(process::pidns::Reservation, Option<process::IdClaim>), SyscallError> {
    let levels = req.set_tid_levels as usize;
    if levels > ns.level + 1 {
        return Err(SyscallError::InvalidArgument);
    }
    // Choosing a number also needs MODIFY on each PID namespace it is
    // chosen in (ADR 0024, step 5); EPERM before any number is looked at,
    // as alloc_pid's capability check precedes its EEXIST.
    if !crate::cap::authority::current_may_choose_pids(ns, levels) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let chosen: alloc::vec::Vec<u64> = req.set_tid[..levels.min(ns.level)]
        .iter()
        .map(|&t| t.max(0) as u64)
        .collect();
    let numbers = process::pidns::reserve(ns, &chosen, &may_choose).map_err(|e| {
        use process::pidns::AllocError;
        match e {
            AllocError::Invalid => SyscallError::InvalidArgument,
            AllocError::Permission => SyscallError::OperationNotPermitted,
            AllocError::Exists => SyscallError::FileExists,
            AllocError::Dead => SyscallError::OutOfMemory,
            AllocError::Exhausted => SyscallError::WouldBlock,
        }
    })?;
    let claim = if levels == ns.level + 1 {
        let initial = process::userns::initial();
        let id = set_tid_checks(req.set_tid[ns.level], may_choose(&initial))?;
        Some(process::IdClaim::new(id).ok_or(SyscallError::FileExists)?)
    } else {
        None
    };
    Ok((numbers, claim))
}

/// A clone without CLONE_THREAD: a new process, as fork or vfork (N-210).
/// musl's and glibc's posix_spawn -- and so system and popen -- call
/// clone(CLONE_VM | CLONE_VFORK | SIGCHLD) with a stack of their own; it
/// was EINVAL. CLONE_VM shares the parent's memory page by page
/// (`VirtualAddressSpace::share_from`). `id` is set_tid's, claimed.
fn process_clone(
    req: &CloneRequest,
    id: Option<u64>,
    numbers: process::pidns::Reservation,
    namespaces: alloc::sync::Arc<ns::Namespaces>,
    user_ns: Option<alloc::sync::Arc<process::userns::UserNs>>,
) -> Result<usize, SyscallError> {
    #[cfg(target_arch = "x86_64")]
    if req.on(CLONE_SETTLS) && !crate::arch::x86_64::trap::is_user_address(req.tls as u64) {
        return Err(SyscallError::InvalidArgument);
    }
    // 0 keeps the caller's stack (fork); otherwise it must be user memory.
    if req.stack > user_space_end() {
        return Err(SyscallError::InvalidPointer);
    }
    for (flag, ptr) in [
        (CLONE_PARENT_SETTID, req.parent_tid),
        (CLONE_CHILD_SETTID, req.child_tid),
        (CLONE_CHILD_CLEARTID, req.child_tid),
    ] {
        if req.on(flag) {
            validate_user_ptr(ptr as *const u32, core::mem::size_of::<u32>())?;
        }
    }
    // The creator of a new user namespace -- the parent, which also writes
    // the child's maps -- gets authority over it, and the child inherits
    // that with the rest of the space (ADR 0024, decision 7).
    if let Some(ns) = &user_ns {
        let parent = process::current_process().ok_or(SyscallError::InvalidState)?;
        crate::cap::authority::grant_user_ns(&parent.capability_space.lock(), ns)
            .map_err(|_| SyscallError::OutOfMemory)?;
    }
    let on = |flag: u64| req.on(flag);
    super::process::sys_fork_with(&process::fork::ForkOptions {
        share_vm: on(CLONE_VM),
        share_fs: on(CLONE_FS),
        share_files: on(CLONE_FILES),
        share_sighand: on(CLONE_SIGHAND),
        clone_parent: on(CLONE_PARENT),
        vfork: on(CLONE_VFORK),
        stack: (req.stack != 0).then_some(req.stack),
        tls: on(CLONE_SETTLS).then_some(req.tls),
        exit_signal: req.exit_signal as u32,
        parent_settid: on(CLONE_PARENT_SETTID).then_some(req.parent_tid),
        child_settid: on(CLONE_CHILD_SETTID).then_some(req.child_tid),
        child_cleartid: on(CLONE_CHILD_CLEARTID).then_some(req.child_tid),
        pidfd: on(CLONE_PIDFD).then_some(req.pidfd),
        pidfd_autokill: on(CLONE_PIDFD_AUTOKILL),
        pid: id.map(process::ProcessId),
        pid_numbers: Some(process::pidns::Pending::new(numbers)),
        clear_sighand: on(CLONE_CLEAR_SIGHAND),
        no_new_privs: on(CLONE_NNP),
        autoreap: on(CLONE_AUTOREAP),
        namespaces: Some(namespaces),
        fs_at_root: on(CLONE_EMPTY_MNTNS),
        user_ns,
    })
}

/// Create a new thread sharing the current process's address space and
/// resources, following the Linux `clone(2)` semantics for thread creation.
///
/// The new thread shares the parent's virtual memory (`CLONE_VM`), file
/// descriptor table (`CLONE_FILES`), signal handlers (`CLONE_SIGHAND`), and
/// thread group (`CLONE_THREAD`).  These four flags are mandatory.  Optional
/// flags include:
///
/// - `CLONE_FS` — share current working directory and umask.  If not set the
///   child gets an independent copy.
/// - `CLONE_SETTLS` — set the TLS base register (FS on x86_64, TPIDR_EL0 on
///   AArch64, `tp` on RISC-V) to `tls`.
/// - `CLONE_PARENT_SETTID` — write the child's TID to `*parent_tid` in the
///   parent's address space before the child starts running.
/// - `CLONE_CHILD_SETTID` — write the child's TID to `*child_tid` in the
///   child's address space.
/// - `CLONE_CHILD_CLEARTID` — register `child_tid` so that when the child
///   exits, the kernel zeroes `*child_tid` and performs a `FUTEX_WAKE` on that
///   address (used by `pthread_join`).
/// - `CLONE_PIDFD` — a PIDFD_THREAD pidfd for the thread, stored at `pidfd`.
///
/// `id` is set_tid's, claimed: the thread's ID. Returns the new thread's
/// TID.
fn thread_clone(
    req: &CloneRequest,
    id: Option<u64>,
    numbers: process::pidns::Reservation,
    namespaces: alloc::sync::Arc<ns::Namespaces>,
) -> Result<usize, SyscallError> {
    let (newsp, tls) = (req.stack, req.tls);
    let (parent_tid_ptr, child_tid_ptr) = (req.parent_tid, req.child_tid);
    // The thread shares the fd table too (one per process here).
    if !req.on(CLONE_FILES) {
        return Err(SyscallError::InvalidArgument);
    }
    // Share FS? (CWD and umask). If not set, clone gets its own copy.
    let share_fs = req.on(CLONE_FS);

    // The TLS base is loaded into IA32_FS_BASE whenever the thread runs; a
    // non-canonical value would fault (#GP) in ring 0 at every switch.
    #[cfg(target_arch = "x86_64")]
    if req.on(CLONE_SETTLS) && !crate::arch::x86_64::trap::is_user_address(tls as u64) {
        return Err(SyscallError::InvalidArgument);
    }

    // Basic user stack validation
    if newsp == 0 {
        return Err(SyscallError::InvalidPointer);
    }
    // Ensure the stack pointer is within the user-space address range (a
    // stack top may equal the exclusive end). Kernel addresses must be
    // rejected to prevent a malicious caller from running a thread with
    // a kernel-space stack.
    if newsp > user_space_end() {
        return Err(SyscallError::InvalidPointer);
    }

    // Validate TID pointers if requested
    if req.on(CLONE_PARENT_SETTID) {
        validate_user_ptr(parent_tid_ptr as *const u32, core::mem::size_of::<u32>())?;
    }
    if req.on(CLONE_CHILD_SETTID) {
        validate_user_ptr(child_tid_ptr as *const u32, core::mem::size_of::<u32>())?;
    }
    if req.on(CLONE_CHILD_CLEARTID) {
        validate_user_ptr(child_tid_ptr as *const u32, core::mem::size_of::<u32>())?;
    }
    let proc = process::current_process().ok_or(SyscallError::InvalidState)?;
    // RLIMIT_NPROC counts threads too (EAGAIN).
    // Held until the thread is added.
    let _nproc = process::rlimit::check_nproc(&proc)?;

    // Clone current user context so child resumes at same PC with retval=0.
    // The caller pushed start_routine/arg on the user stack before invoking clone.
    let current_thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let current_ctx = current_thread.context.lock();
    let mut builder = ThreadBuilder::new(
        proc.pid,
        "clone-thread".into(),
        current_ctx.get_instruction_pointer(),
    )
    .kernel_stack_size(process::creation::DEFAULT_KERNEL_STACK_SIZE)
    // Scheduling parameters as Linux's sched_fork (N-221).
    .sched(current_thread.sched.lock().for_child())
    // And its CPU mask (Linux copies it too); every CPU was allowed.
    .cpu_affinity(current_thread.get_affinity())
    // The thread runs on the caller's stack (`newsp`, required above): no
    // kernel-chosen stack is mapped for it. One used to be, 256 KiB per
    // thread that nothing ran on (N-114).
    .user_stack_size(0);
    if let Some(id) = id {
        builder = builder.tid(process::ThreadId(id));
    }

    #[cfg(feature = "alloc")]
    {
        let parent_fs = current_thread.fs();
        let child_fs = if share_fs {
            ThreadFs::clone_shared(&parent_fs)
        } else {
            let fs = ThreadFs::clone_copy(&parent_fs);
            if req.on(CLONE_EMPTY_MNTNS) {
                fs.move_to_root();
            }
            fs
        };
        builder = builder.fs(child_fs);
    }

    if req.on(CLONE_SETTLS) {
        builder = builder.tls_base(tls);
    }
    if req.on(CLONE_CHILD_CLEARTID) {
        builder = builder.clear_tid(child_tid_ptr);
    }

    let thread = builder.build().map_err(|_| SyscallError::InvalidState)?;
    let tid = thread.tid;
    // The thread's number in each PID namespace level; released again by
    // `cleanup_thread`, or below if the thread never joins the process.
    numbers.commit(tid.0);
    let vtid = process::pidns::nr_in(tid.0, &proc.pid_ns()) as u32;
    let release_numbers = || process::pidns::release(tid.0);
    // A new thread starts with its creator's blocked mask (N-109), command
    // name (N-227), no_new_privs and capability sets (ADR 0020), as Linux
    // copies them. The sets are copied again once the thread is in the
    // process (below).
    thread.sigmask.store(
        current_thread
            .sigmask
            .load(core::sync::atomic::Ordering::Acquire),
        core::sync::atomic::Ordering::Release,
    );
    *thread.comm.lock() = *current_thread.comm.lock();
    thread.personality.store(
        current_thread
            .personality
            .load(core::sync::atomic::Ordering::Acquire),
        core::sync::atomic::Ordering::Release,
    );
    thread.no_new_privs.store(
        current_thread
            .no_new_privs
            .load(core::sync::atomic::Ordering::Acquire),
        core::sync::atomic::Ordering::Release,
    );
    let sets = *current_thread.caps.lock();
    *thread.caps.lock() = sets;
    #[cfg(feature = "alloc")]
    thread.set_namespaces(namespaces);

    // Override context with cloned registers so the child returns 0 from clone.
    //
    // The thread's context was initialized from the exec/load entry point, but
    // the parent is mid-syscall: the *live* registers (RIP = return address
    // after SYSCALL, callee-saved GPRs) are on the kernel stack in the
    // SyscallFrame, NOT in the stale ThreadContext.  We clone the stale
    // context as a baseline, then overlay the live values from SyscallFrame.
    {
        let mut child_ctx = thread.context.lock();
        ThreadContext::clone_from(&mut *child_ctx, &*current_ctx);

        // Overlay live registers from the SyscallFrame saved by syscall_entry.
        // Without this, the child's RIP is the process entry point (from exec)
        // instead of the instruction after the clone() syscall.
        #[cfg(target_arch = "x86_64")]
        if let Some(frame) = crate::arch::x86_64::syscall::get_syscall_frame() {
            child_ctx.rip = frame.rip;
            child_ctx.rflags = frame.rflags;
            child_ctx.rbx = frame.rbx;
            child_ctx.rbp = frame.rbp;
            child_ctx.r12 = frame.r12;
            child_ctx.r13 = frame.r13;
            child_ctx.r14 = frame.r14;
            child_ctx.r15 = frame.r15;
            child_ctx.rdi = frame.rdi;
            child_ctx.rsi = frame.rsi;
            child_ctx.rdx = frame.rdx;
            child_ctx.r8 = frame.r8;
            child_ctx.r9 = frame.r9;
            child_ctx.r10 = frame.r10;
        }
        // Gated as the function is defined: bare-metal AArch64 and RISC-V.
        #[cfg(all(
            target_os = "none",
            any(target_arch = "aarch64", target_arch = "riscv64")
        ))]
        crate::process::load_live_user_registers(&mut child_ctx);

        child_ctx.set_stack_pointer(newsp);
        child_ctx.set_return_value(0); // child sees 0 return
                                       // Apply requested TLS base after cloning parent context
        if req.on(CLONE_SETTLS) {
            child_ctx.set_tls_base(tls as u64);
        }
    }
    drop(current_ctx);

    // Both TID writes happen before the thread joins the process and before
    // its task exists: either can fault, and failing after the enqueue left
    // a runnable child behind an error return (review of the v0.26.0 stack,
    // PR #14). Here a failure only drops the unregistered `thread`.
    //
    // CLONE_PARENT_SETTID: write the child's TID into the parent's address
    // space at `parent_tid_ptr`, so the parent can observe the TID
    // immediately after clone returns.
    if req.on(CLONE_PARENT_SETTID) {
        // SAFETY: `parent_tid_ptr` was validated above via `validate_user_ptr`.
        // `copy_to_user` performs its own bounds check as a defence-in-depth
        // measure.  The write is a single u32 which is naturally aligned.
        unsafe {
            crate::syscall::userspace::copy_to_user(parent_tid_ptr, &vtid).map_err(|_| {
                release_numbers();
                SyscallError::InvalidPointer
            })?;
        }
    }

    // CLONE_CHILD_SETTID: write the child's TID into `child_tid_ptr`.
    // TODO(tier7): On Linux this write happens in the *child's* address
    // space after the child begins execution.  Because CLONE_VM is
    // mandatory here (shared address space), writing from the parent
    // context is equivalent.  For full fork() support (separate address
    // spaces) this would need to be deferred to the child's first
    // scheduling quantum.
    if req.on(CLONE_CHILD_SETTID) {
        // SAFETY: `child_tid_ptr` was validated above via `validate_user_ptr`.
        // The pointer targets shared user memory (CLONE_VM is set).
        unsafe {
            crate::syscall::userspace::copy_to_user(child_tid_ptr, &vtid).map_err(|_| {
                release_numbers();
                SyscallError::InvalidPointer
            })?;
        }
    }

    // CLONE_PIDFD: a PIDFD_THREAD pidfd for the new thread, stored before
    // it runs (EFAULT if the store faults; the thread never starts).
    #[cfg(feature = "alloc")]
    let pidfd = if req.on(CLONE_PIDFD) {
        Some(
            super::pidfd::install_at(&proc, Some(tid), req.pidfd, false).inspect_err(|_| {
                release_numbers();
            })?,
        )
    } else {
        None
    };
    #[cfg(feature = "alloc")]
    let undo_pidfd = || {
        if let Some(fd) = pidfd {
            super::pidfd::uninstall(fd);
        }
    };
    #[cfg(not(feature = "alloc"))]
    let undo_pidfd = || {};

    // Add to the process first: the task keeps a pointer to the thread, so
    // it must be created from the thread's final, shared (Arc) location.
    // It used to be created from this local, which add_thread() then moved,
    // leaving the task's thread_ref pointing at a dead stack slot.
    let thread = proc.add_thread(thread).map_err(|_| {
        undo_pidfd();
        release_numbers();
        SyscallError::InvalidState
    })?;
    // A set*id call on another thread fixes every thread's sets under the
    // thread map (`Process::change_uids`). Copied again under the map, the
    // new thread's sets either went through that fixup or are taken from a
    // creator that already has.
    {
        let _threads = proc.threads.lock();
        let sets = *current_thread.caps.lock();
        *thread.caps.lock() = sets;
    }

    // The new thread runs as its own dispatcher task (stage D2), starting
    // from the registers prepared above and a copy of the caller's vector
    // state. (CLONE_CHILD_CLEARTID is honoured by exit_dispatched.)
    if crate::process::start_thread(&proc, &thread, true).is_err() {
        // The thread never ran: take it out again (with its kernel
        // stack), or it counted as alive and its process could never
        // finish exiting (N-46).
        drop(thread);
        let _ = crate::process::exit::cleanup_thread(&proc, tid);
        undo_pidfd();
        return Err(SyscallError::OutOfMemory);
    }
    Ok(vtid as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EINVAL: Result<(), SyscallError> = Err(SyscallError::InvalidArgument);

    const NO_IDS: [i32; MAX_PID_NS_LEVEL as usize] = [0; MAX_PID_NS_LEVEL as usize];

    fn args(flags: u64) -> CloneArgs {
        CloneArgs {
            flags,
            ..CloneArgs::default()
        }
    }

    fn request(flags: u64) -> CloneRequest {
        CloneRequest {
            flags,
            ..CloneRequest::default()
        }
    }

    #[test]
    fn clone_args_decode_fills_a_short_struct_with_zeros() {
        let mut raw = [0u8; CLONE_ARGS_SIZE_VER0];
        raw[0..8].copy_from_slice(&0x1234u64.to_ne_bytes());
        raw[32..40].copy_from_slice(&17u64.to_ne_bytes());
        raw[56..64].copy_from_slice(&0xabcdu64.to_ne_bytes());
        let a = CloneArgs::decode(&raw);
        assert_eq!((a.flags, a.exit_signal, a.tls), (0x1234, 17, 0xabcd));
        assert_eq!((a.set_tid, a.set_tid_size, a.cgroup), (0, 0, 0));
    }

    #[test]
    fn clone_args_checks_are_copy_clone_args_from_users() {
        assert_eq!(args(0).check(64), Ok(()));
        let too_many = CloneArgs {
            set_tid: 8,
            set_tid_size: MAX_PID_NS_LEVEL + 1,
            ..args(0)
        };
        assert_eq!(too_many.check(88), EINVAL);
        let no_array = CloneArgs {
            set_tid_size: 1,
            ..args(0)
        };
        assert_eq!(no_array.check(88), EINVAL);
        let no_size = CloneArgs {
            set_tid: 8,
            ..args(0)
        };
        assert_eq!(no_size.check(88), EINVAL);
        let one = CloneArgs {
            set_tid: 8,
            set_tid_size: 1,
            ..args(0)
        };
        assert_eq!(one.check(80), Ok(()));
        let high_signal = CloneArgs {
            exit_signal: 0x100,
            ..args(0)
        };
        assert_eq!(high_signal.check(64), EINVAL);
        let cgroup = CloneArgs {
            cgroup: 3,
            ..args(CLONE_INTO_CGROUP)
        };
        assert_eq!(cgroup.check(88), Ok(()));
        // The struct must be long enough to have the field.
        assert_eq!(cgroup.check(80), EINVAL);
        let big = CloneArgs {
            cgroup: i32::MAX as u64 + 1,
            ..args(CLONE_INTO_CGROUP)
        };
        assert_eq!(big.check(88), EINVAL);
    }

    #[test]
    fn clone3_refuses_unknown_and_reused_flags() {
        assert!(clone3_request(&args(CLONE_VM), NO_IDS).is_ok());
        // Past the flags clone3 knows.
        assert_eq!(
            clone3_request(&args(1 << 40), NO_IDS),
            Err(SyscallError::InvalidArgument)
        );
        // CLONE_DETACHED and the CSIGNAL bits are kept for reuse, except
        // CLONE_NEWTIME.
        assert!(clone3_request(&args(CLONE_DETACHED), NO_IDS).is_err());
        assert!(clone3_request(&args(17), NO_IDS).is_err());
        assert!(clone3_request(&args(CLONE_NEWTIME), NO_IDS).is_ok());
        assert!(clone3_request(&args(CLONE_SIGHAND | CLONE_CLEAR_SIGHAND), NO_IDS).is_err());
        assert!(clone3_request(&args(CLONE_CLEAR_SIGHAND), NO_IDS).is_ok());
        // An exit signal for a thread, or for a CLONE_PARENT child.
        for flags in [CLONE_THREAD, CLONE_PARENT] {
            let a = CloneArgs {
                exit_signal: 17,
                ..args(flags)
            };
            assert!(clone3_request(&a, NO_IDS).is_err());
        }
    }

    #[test]
    fn clone3_stack_is_a_base_and_size_and_starts_at_its_top() {
        let stack = |stack, stack_size| CloneArgs {
            stack,
            stack_size,
            ..args(0)
        };
        assert_eq!(clone3_request(&stack(0, 0), NO_IDS).map(|r| r.stack), Ok(0));
        assert!(clone3_request(&stack(0, 4096), NO_IDS).is_err());
        assert!(clone3_request(&stack(0x10_0000, 0), NO_IDS).is_err());
        assert_eq!(
            clone3_request(&stack(0x10_0000, 0x4000), NO_IDS).map(|r| r.stack),
            Ok(0x10_4000)
        );
        // Not wholly user memory, or wrapping.
        let end = user_space_end() as u64;
        assert!(clone3_request(&stack(end - 0x1000, 0x2000), NO_IDS).is_err());
        assert!(clone3_request(&stack(u64::MAX - 0xfff, 0x2000), NO_IDS).is_err());
        assert!(clone3_request(&stack(end - 0x2000, 0x2000), NO_IDS).is_ok());
    }

    #[test]
    fn clone3_takes_its_pidfd_pointer_and_set_tid() {
        let a = CloneArgs {
            pidfd: 0x5000,
            parent_tid: 0x6000,
            set_tid: 0x7000,
            set_tid_size: 1,
            ..args(CLONE_PIDFD | CLONE_PARENT_SETTID)
        };
        let mut ids = [0; MAX_PID_NS_LEVEL as usize];
        ids[0] = 400;
        let r = clone3_request(&a, ids).unwrap();
        assert_eq!((r.pidfd, r.parent_tid), (0x5000, 0x6000));
        assert_eq!((r.set_tid[0], r.set_tid_levels), (400, 1));
    }

    #[test]
    fn copy_process_flag_rules_in_order() {
        let check = |flags| copy_process_checks(&request(flags), false);
        assert_eq!(check(CLONE_NEWNS | CLONE_FS), EINVAL);
        assert_eq!(check(CLONE_THREAD | CLONE_VM), EINVAL);
        assert_eq!(check(CLONE_SIGHAND), EINVAL);
        assert_eq!(
            check(CLONE_THREAD | CLONE_SIGHAND | CLONE_VM | CLONE_FILES),
            Ok(())
        );
        assert_eq!(check(CLONE_PIDFD | CLONE_DETACHED), EINVAL);
        assert_eq!(check(CLONE_PIDFD), Ok(()));
        assert_eq!(check(CLONE_DETACHED), Ok(()));
        // CLONE_AUTOREAP: a process, not CLONE_PARENT, no exit signal.
        assert_eq!(check(CLONE_AUTOREAP), Ok(()));
        assert_eq!(check(CLONE_AUTOREAP | CLONE_PARENT), EINVAL);
        assert_eq!(
            check(CLONE_AUTOREAP | CLONE_THREAD | CLONE_SIGHAND | CLONE_VM),
            EINVAL
        );
        let signalled = CloneRequest {
            exit_signal: 17,
            ..request(CLONE_AUTOREAP)
        };
        assert_eq!(copy_process_checks(&signalled, false), EINVAL);
        assert_eq!(
            check(CLONE_NNP | CLONE_THREAD | CLONE_SIGHAND | CLONE_VM),
            EINVAL
        );
        assert_eq!(check(CLONE_NNP), Ok(()));
    }

    #[test]
    fn pidfd_autokill_needs_a_reaped_pidfd_child_and_nnp_or_admin() {
        let check = |flags, admin| copy_process_checks(&request(flags), admin);
        let base = CLONE_PIDFD | CLONE_AUTOREAP | CLONE_PIDFD_AUTOKILL;
        assert_eq!(check(CLONE_AUTOREAP | CLONE_PIDFD_AUTOKILL, true), EINVAL);
        assert_eq!(check(CLONE_PIDFD | CLONE_PIDFD_AUTOKILL, true), EINVAL);
        assert_eq!(check(base, false), Err(SyscallError::OperationNotPermitted));
        assert_eq!(check(base, true), Ok(()));
        assert_eq!(check(base | CLONE_NNP, false), Ok(()));
    }

    #[test]
    fn kernel_clone_checks_before_copy_process() {
        let mut same = CloneRequest {
            pidfd: 0x5000,
            parent_tid: 0x5000,
            ..request(CLONE_PIDFD | CLONE_PARENT_SETTID)
        };
        assert_eq!(request_checks(&mut same, false), EINVAL);
        let mut apart = CloneRequest {
            pidfd: 0x5000,
            parent_tid: 0x6000,
            ..request(CLONE_PIDFD | CLONE_PARENT_SETTID)
        };
        assert_eq!(request_checks(&mut apart, false), Ok(()));
        let mut past = CloneRequest {
            exit_signal: crate::process::signals::NSIG as u64 + 1,
            ..request(0)
        };
        assert_eq!(request_checks(&mut past, false), EINVAL);
        // CLONE_EMPTY_MNTNS is CLONE_NEWNS too, so CLONE_FS goes with
        // neither.
        let mut empty = request(CLONE_EMPTY_MNTNS | CLONE_FS);
        assert_eq!(request_checks(&mut empty, false), EINVAL);
        assert!(empty.on(CLONE_NEWNS));
        // And so needs CAP_SYS_ADMIN before the child's root moves.
        let mut alone = request(CLONE_EMPTY_MNTNS);
        assert_eq!(request_checks(&mut alone, false), Ok(()));
        assert!(matches!(
            ns::for_clone(
                &ns::initial(),
                alone.flags,
                false,
                &crate::process::userns::initial(),
                &crate::process::pidns::initial(),
                0
            ),
            Err(ns::CloneNsError::Permission)
        ));
    }

    #[test]
    fn kernel_clone_refuses_what_it_does_not_support() {
        let check = |flags| request_checks(&mut request(flags), true);
        // Namespace flags pass here and meet copy_namespaces' rules later.
        assert_eq!(check(CLONE_NEWNS), Ok(()));
        assert_eq!(
            check(CLONE_THREAD | CLONE_SIGHAND | CLONE_VM | CLONE_FILES | ns::CLONE_NEWPID),
            EINVAL
        );
        // A thread never gets a user namespace (or the privilege over its
        // other namespaces that one confers) it would not run in.
        assert_eq!(
            check(CLONE_THREAD | CLONE_SIGHAND | CLONE_VM | ns::CLONE_NEWUSER),
            EINVAL
        );
        // A process may share the file table and signal actions, and be
        // its caller's sibling (G2b); the actions only with the memory.
        assert_eq!(check(CLONE_PARENT), Ok(()));
        assert_eq!(check(CLONE_VM | CLONE_SIGHAND | CLONE_FILES), Ok(()));
        assert_eq!(check(CLONE_FILES), Ok(()));
        assert_eq!(check(CLONE_SIGHAND | CLONE_FILES), EINVAL);
        assert_eq!(
            check(CLONE_THREAD | CLONE_SIGHAND | CLONE_VM | CLONE_PARENT),
            Ok(())
        );
        assert_eq!(check(0x2000 /* CLONE_PTRACE */), EINVAL);
        assert_eq!(check(CLONE_VM | CLONE_VFORK | CLONE_CLEAR_SIGHAND), Ok(()));
        assert_eq!(
            check(CLONE_THREAD | CLONE_SIGHAND | CLONE_VM | CLONE_FILES | CLONE_SETTLS),
            Ok(())
        );
        // CLONE_INTO_CGROUP passes the flag checks; kernel_clone refuses
        // the descriptor (EBADF).
        assert_eq!(check(CLONE_INTO_CGROUP), Ok(()));
    }

    #[test]
    fn legacy_clone_keeps_pidfd_and_signal_in_its_registers() {
        // clone's CLONE_PIDFD | CLONE_PARENT_SETTID store to one address.
        let flags = CLONE_PIDFD | CLONE_PARENT_SETTID | 17;
        let req = CloneRequest {
            flags: flags & !CSIGNAL,
            exit_signal: flags & CSIGNAL,
            parent_tid: 0x5000,
            pidfd: 0x5000,
            ..CloneRequest::default()
        };
        let mut checked = req;
        assert_eq!(request_checks(&mut checked, false), EINVAL);
        assert_eq!(req.exit_signal, 17);
        assert!(copy_process_checks(&request(CLONE_PIDFD | CLONE_DETACHED), false).is_err());
    }

    #[test]
    fn set_tid_is_in_range_and_privileged() {
        assert_eq!(set_tid_checks(0, true), Err(SyscallError::InvalidArgument));
        assert_eq!(set_tid_checks(-1, true), Err(SyscallError::InvalidArgument));
        let max = crate::process::PID_MAX as i32;
        assert_eq!(
            set_tid_checks(max, true),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(set_tid_checks(max - 1, true), Ok(max as u64 - 1));
        assert_eq!(
            set_tid_checks(400, false),
            Err(SyscallError::OperationNotPermitted)
        );
    }

    /// alloc_pid's set_tid walk: no more entries than levels, the first for
    /// the child's namespace, the last (at full length) the kernel id.
    #[test]
    fn set_tid_names_one_id_per_level() {
        use crate::process::pidns::{self, PidNs};
        let with = |ids: &[i32]| {
            let mut req = request(0);
            req.set_tid[..ids.len()].copy_from_slice(ids);
            req.set_tid_levels = ids.len() as u64;
            req
        };
        let init = pidns::initial();
        assert!(matches!(
            pid_numbers(&with(&[400, 401]), &init, |_| true),
            Err(SyscallError::InvalidArgument)
        ));
        let ns = PidNs::create(&init, &crate::process::userns::initial(), 0).unwrap();
        assert!(matches!(
            pid_numbers(&with(&[1, 2, 3]), &ns, |_| true),
            Err(SyscallError::InvalidArgument)
        ));
        // Only the namespace's own entry: no kernel id is chosen.
        let (r, claim) = pid_numbers(&with(&[1]), &ns, |_| true).unwrap();
        assert_eq!(r.innermost(), Some(1));
        assert!(claim.is_none());
        r.commit(3_900_001);
        // Both: a kernel id too, checked last.
        let (r, claim) = pid_numbers(&with(&[5, 3_900_002]), &ns, |_| true).unwrap();
        assert_eq!(r.innermost(), Some(5));
        assert_eq!(claim.as_ref().map(|c| c.id()), Some(3_900_002));
        drop(r);
        assert!(matches!(
            pid_numbers(&with(&[5, 0]), &ns, |_| true),
            Err(SyscallError::InvalidArgument)
        ));
        assert!(ns.find(5).is_none(), "a failed walk frees what it reserved");
        assert!(matches!(
            pid_numbers(&with(&[1]), &ns, |_| true),
            Err(SyscallError::FileExists)
        ));
        pidns::release(3_900_001);
    }
}

//! Process creation and setup
//!
//! Handles creating new processes from scratch and replacing process images
//! via the exec system call. Includes argument/environment stack setup for
//! newly executed programs.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{format, string::String, vec::Vec};

use super::{
    pcb::{Process, ProcessBuilder, ProcessState},
    table,
    thread::ThreadBuilder,
    ProcessId, ProcessPriority,
};
#[allow(unused_imports)]
use crate::{arch::context::ThreadContext, error::KernelError};

/// Default stack sizes
pub const DEFAULT_USER_STACK_SIZE: usize = 256 * 1024; // 256KB initial (grows via page faults)
pub const DEFAULT_KERNEL_STACK_SIZE: usize = 64 * 1024; // 64KB

/// Process creation options
#[cfg(feature = "alloc")]
pub struct ProcessCreateOptions {
    pub name: String,
    pub parent: Option<ProcessId>,
    pub priority: ProcessPriority,
    pub entry_point: usize,
    pub argv: Vec<String>,
    pub envp: Vec<String>,
    pub user_stack_size: usize,
    pub kernel_stack_size: usize,
}

#[cfg(feature = "alloc")]
impl Default for ProcessCreateOptions {
    fn default() -> Self {
        Self {
            name: String::from("unnamed"),
            parent: None,
            priority: ProcessPriority::Normal,
            entry_point: 0,
            argv: Vec::new(),
            envp: Vec::new(),
            user_stack_size: DEFAULT_USER_STACK_SIZE,
            kernel_stack_size: DEFAULT_KERNEL_STACK_SIZE,
        }
    }
}

/// Create a new process
#[cfg(feature = "alloc")]
pub fn create_process(name: String, entry_point: usize) -> Result<ProcessId, KernelError> {
    let options = ProcessCreateOptions {
        name,
        entry_point,
        ..Default::default()
    };

    create_process_with_options(options)
}

/// Create a new process with options
#[cfg(feature = "alloc")]
pub fn create_process_with_options(
    options: ProcessCreateOptions,
) -> Result<ProcessId, KernelError> {
    // Enforce process count limit before allocating resources.
    let current_count = table::PROCESS_TABLE.count();
    if current_count >= super::MAX_PROCESSES {
        return Err(KernelError::ResourceExhausted {
            resource: "process table",
        });
    }

    // Create the process
    let process = ProcessBuilder::new(options.name.clone())
        .parent(options.parent.unwrap_or(ProcessId(0)))
        .priority(options.priority)
        .build();

    let pid = process.pid;

    // Set up the process's address space
    {
        let mut memory_space = process.memory_space.lock();
        // init() already maps kernel space, so we don't need to call map_kernel_space()
        // again
        memory_space.init()?;
    }

    // Create the main thread
    // The new process starts in its creator's working directory: the
    // kernel shell's after a `cd`, or the calling thread's (N-115).
    let (start_cwd, start_root) = crate::fs::try_get_vfs()
        .map(|vfs| {
            let ctx = vfs.caller_context();
            (ctx.cwd, ctx.root)
        })
        .unwrap_or_else(|| (String::from("/"), String::from("/")));
    let main_thread =
        ThreadBuilder::new(pid, format!("{}-main", options.name), options.entry_point)
            .tid(super::ThreadId(pid.0))
            .user_stack_size(options.user_stack_size)
            .kernel_stack_size(options.kernel_stack_size)
            .fs(super::thread::ThreadFs::with_cwd(start_cwd, start_root))
            .build()?;

    let tid = main_thread.tid;

    // A program the kernel starts holds the object capabilities behind the
    // Linux privilege checks: its namespaces, the clocks and the system
    // (ADR 0024). Fork copies them and exec keeps them.
    crate::cap::authority::grant_initial(
        &process.capability_space.lock(),
        &main_thread.namespaces(),
        process.user_ns().ident,
        process.pid_ns().ident,
    )
    .map_err(|_| KernelError::ResourceExhausted {
        resource: "capability quota",
    })?;

    // Map the user stack pages into the process's VAS page tables.
    // ThreadBuilder::build() allocates physical frames for the user stack
    // but does not map them. We call vas.map_page() for each page, which
    // allocates new physical frames and creates the PTE entries.
    {
        let user_base = main_thread.user_stack.base;
        let user_size = main_thread.user_stack.size;
        let num_pages = user_size / 4096;

        let mut memory_space = process.memory_space.lock();

        let stack_flags = crate::mm::PageFlags::PRESENT
            | crate::mm::PageFlags::USER
            | crate::mm::PageFlags::WRITABLE
            | crate::mm::PageFlags::NO_EXECUTE;
        for i in 0..num_pages {
            let vaddr = user_base + i * 4096;
            memory_space.map_page(vaddr, stack_flags)?;
        }

        // Update VAS stack_top to match the main thread's actual allocated stack
        // This ensures setup_exec_stack() uses the correct stack range
        memory_space.set_stack_top(user_base + user_size);
        memory_space.set_stack_size(user_size);
    }

    // Add thread to process
    process.add_thread(main_thread)?;

    // Setup user stack with arguments and environment
    // Convert String vectors to &str slices for setup_exec_stack
    let argv_refs: Vec<&str> = options.argv.iter().map(|s| s.as_str()).collect();
    let envp_refs: Vec<&str> = options.envp.iter().map(|s| s.as_str()).collect();

    // Get the process before adding to table so we can set up the stack.
    // A program loaded later replaces it; for one that is not, the vector
    // has the entry point and no program headers.
    let program = crate::elf::dynamic::ProgramAux {
        entry: options.entry_point as u64,
        ..Default::default()
    };
    // A process the kernel starts runs as its credentials say, with no
    // exec to grant privilege: AT_SECURE only for differing IDs.
    let secure = process.credentials().ids_differ();
    let stack_top = setup_exec_stack(
        &process,
        &argv_refs,
        &envp_refs,
        &options.name,
        &program,
        secure,
    )?;

    // Update the thread context with the adjusted stack pointer
    if let Some(thread) = process.get_thread(tid) {
        let mut ctx = thread.context.lock();
        ctx.set_stack_pointer(stack_top);
    }

    // Add process to process table
    table::add_process(process)?;

    // Mark process as ready. Its launcher starts the main thread as a
    // dispatcher task (process::start_thread / run_and_wait).
    if let Some(process) = table::get_process(pid) {
        process.set_state(ProcessState::Ready);
    }

    // Memory hardening: stack canary + guard page for new process.
    // Only on x86_64 which has a proper LockedHeap allocator and trap
    // handler. AArch64 hangs on spin::Mutex in the RNG, and RISC-V has
    // no stvec trap handler so any fault during RNG init causes a reboot.
    #[cfg(target_arch = "x86_64")]
    {
        use crate::security::memory_protection::{GuardPage, StackCanary};

        // Create stack canary for the main thread
        let _canary = StackCanary::new();

        // Set up guard page below kernel stack to detect overflow
        let _guard = GuardPage::new(
            options.kernel_stack_size, // guard at bottom of stack region
            4096,                      // one 4KB guard page
        );
    }

    // Audit log: process creation
    crate::security::audit::log_process_create(pid.0, 0, 0);

    Ok(pid)
}

/// Parse a shebang (#!) line from the beginning of a file
///
/// If the data starts with `#!`, extracts the interpreter path and optional
/// argument from the first line (up to 256 bytes or first newline).
///
/// # Examples
/// - `#!/bin/sh\n`        -> Some(("/bin/sh", None))
/// - `#!/bin/sh -e\n`     -> Some(("/bin/sh", Some("-e")))
/// - `#!/usr/bin/env python3\n` -> Some(("/usr/bin/env", Some("python3")))
/// - `\x7fELF...`         -> None (not a shebang)
#[cfg(feature = "alloc")]
pub fn parse_shebang(data: &[u8]) -> Option<(String, Option<String>)> {
    // Must start with #!
    if data.len() < 2 || data[0] != b'#' || data[1] != b'!' {
        return None;
    }

    // Find end of first line, capped at 256 bytes
    let max_len = data.len().min(256);
    let line_end = data[2..max_len]
        .iter()
        .position(|&b| b == b'\n')
        .map(|pos| pos + 2)
        .unwrap_or(max_len);

    // Extract the shebang line content (after #!)
    let line = core::str::from_utf8(&data[2..line_end]).ok()?;
    let line = line.trim();

    if line.is_empty() {
        return None;
    }

    // Split into interpreter and optional argument
    // Only split on the first whitespace -- the rest is a single argument
    if let Some(space_pos) = line.find([' ', '\t']) {
        let interpreter = line[..space_pos].trim();
        let arg = line[space_pos + 1..].trim();
        if interpreter.is_empty() {
            return None;
        }
        let opt_arg = if arg.is_empty() {
            None
        } else {
            Some(String::from(arg))
        };
        Some((String::from(interpreter), opt_arg))
    } else {
        Some((String::from(line), None))
    }
}

/// What an exec runs (Linux's `linux_binprm` file and names): the node,
/// looked up once so that what is checked is what runs (N-101), and the
/// names it goes by.
#[cfg(feature = "alloc")]
pub struct ExecFile {
    /// The program file.
    pub node: alloc::sync::Arc<dyn crate::fs::VfsNode>,
    /// The name the program is given (`bprm->filename`): AT_EXECFN, and the
    /// path a script's interpreter is passed. As written, or `/dev/fd/N`
    /// (`/dev/fd/N/name`) for execveat relative to a descriptor.
    pub filename: String,
    /// The file's path from the caller's root: /proc/self/exe.
    pub exe_path: String,
    /// The command name: the base name of `filename`, or of the file itself
    /// when the name was made up from a descriptor (`comm_from_dentry`).
    pub comm: String,
    /// `filename` names a close-on-exec descriptor, which is gone once the
    /// exec happens, so a script cannot be run from it (ENOENT, Linux's
    /// BINPRM_FLAGS_PATH_INACCESSIBLE).
    pub path_inaccessible: bool,
    /// The flags of the mount it is on (`fs::mount::flags`): noexec
    /// refuses it, nosuid ignores its set-ID bits and capabilities (ADR
    /// 0022 H3c).
    pub mnt_flags: u32,
}

#[cfg(feature = "alloc")]
impl ExecFile {
    /// `path` as Linux's open_exec looks it up: in the caller's view, a
    /// relative path from the working directory (there is no PATH search;
    /// libc's execvp does that), symlinks followed.
    pub fn open(path: &str) -> Result<Self, KernelError> {
        let vfs = crate::fs::get_vfs().as_caller();
        let (node, canonical) = vfs.resolve_path_canonical(path)?;
        Ok(Self {
            node,
            exe_path: crate::fs::root_relative(&canonical, &vfs.get_root()),
            comm: String::from(basename(path)),
            filename: String::from(path),
            path_inaccessible: false,
            mnt_flags: mount_flags(&vfs, &canonical),
        })
    }

    /// EACCES for a file on a noexec mount, before the permission check
    /// (Linux's may_open, MAY_EXEC on a regular file).
    pub fn check_mount(&self) -> Result<(), KernelError> {
        refuse_noexec(self.mnt_flags)
    }
}

/// The flags of the mount serving canonical `path` in `vfs` (0 where no
/// mount serves it).
#[cfg(feature = "alloc")]
pub(crate) fn mount_flags(vfs: &crate::fs::CallerView<'_>, path: &str) -> u32 {
    vfs.mount_serving(path).map_or(0, |m| m.flags())
}

/// EACCES when mount flags `mnt` include noexec.
#[cfg(feature = "alloc")]
fn refuse_noexec(mnt: u32) -> Result<(), KernelError> {
    if mnt & crate::fs::mount::flags::NOEXEC != 0 {
        return Err(KernelError::PermissionDenied { operation: "exec" });
    }
    Ok(())
}

/// What exec learned from the program file about the credentials and
/// dumpability of the new image.
#[cfg(feature = "alloc")]
struct ExecIds {
    /// The owner a set-user-ID file offers.
    set_uid: Option<u32>,
    /// The group a set-group-ID file (with group execute) offers.
    set_gid: Option<u32>,
    /// The caller may read the program (and its loader).
    readable: bool,
    /// The user namespace the new image's memory belongs to: the
    /// process's, or for a program or loader it may not read the nearest
    /// one above that maps that file's owner and group (would_dump), so a
    /// namespace that could not read the file cannot trace the image.
    mm_userns: alloc::sync::Arc<super::userns::UserNs>,
    /// The process was traced, by a tracer without CAP_SYS_PTRACE, when
    /// the exec started.
    unprivileged_tracer_at_start: bool,
    /// The program's capabilities (`security.capability`), if it has any.
    file_caps: Option<super::linux_caps::FileCaps>,
}

/// Whether `process` is traced by a tracer whose credentials when it
/// attached lack CAP_SYS_PTRACE over the process's user namespace (Linux's
/// ptracer_capable). Read under `cred_guard`, which attach holds.
#[cfg(feature = "alloc")]
fn unprivileged_tracer(process: &super::Process) -> bool {
    use core::sync::atomic::Ordering;
    process.tracer.load(Ordering::Acquire) != 0
        && !process
            .tracer_cred
            .lock()
            .as_ref()
            .is_some_and(|t| t.capable_over(&process.user_ns()))
}

/// Install the credentials and capability sets a program runs with
/// (Linux's cred_guard_mutex section of exec), and say whether it runs
/// with AT_SECURE. `set_ids` are the owner and group a set-user-ID or
/// set-group-ID file offers, `file_caps` its capabilities;
/// `CapSets::after_exec` (Linux's cap_bprm_creds_from_file) decides what
/// is granted: not more than before if the thread asked for no new
/// privileges, a tracer without CAP_SYS_PTRACE is attached, or another
/// process shares this one's directories and umask (CLONE_FS: it could
/// redirect the privileged program; LSM_UNSAFE_SHARE). Decided and
/// installed under `cred_guard`, which ptrace attach takes too: a tracer
/// attached before is seen here, and one attaching after sees the new
/// credentials.
#[cfg(feature = "alloc")]
fn install_exec_credentials(
    process: &super::Process,
    thread: &super::Thread,
    ids: ExecIds,
) -> Result<bool, KernelError> {
    use core::sync::atomic::Ordering;

    let _guard = process.cred_guard.lock();
    // Traced without privilege as the exec started or now: either is
    // unsafe (Linux's ptracer_capable: the tracer's CAP_SYS_PTRACE when it
    // attached).
    let traced_unprivileged = ids.unprivileged_tracer_at_start || unprivileged_tracer(process);
    // exec runs only in a single-threaded process, so this thread is the
    // process's only user of its filesystem state.
    let fs_shared = thread.fs().users.load(Ordering::Acquire) > 1;
    let old = process.credentials();
    let no_new_privs = thread.no_new_privs.load(Ordering::Acquire);
    // Under no_new_privs the set-ID bits are ignored altogether (Linux's
    // bprm_fill_uid): the IDs do not change, so the program is not secure.
    let (set_uid, set_gid) = if no_new_privs {
        (None, None)
    } else {
        (ids.set_uid, ids.set_gid)
    };
    let (new_euid, new_egid) = (set_uid.unwrap_or(old.euid), set_gid.unwrap_or(old.egid));
    // CAP_SETUID in the process's own user namespace: set-ID bits counted
    // above only if it maps the file's owner and group.
    let may_setuid = thread.caps.lock().capable(super::linux_caps::CAP_SETUID);
    let ctx = super::linux_caps::ExecContext {
        file: ids.file_caps,
        no_new_privs,
        unsafe_exec: traced_unprivileged || fs_shared,
        may_setuid,
        egid_in_groups: old.in_group(new_egid),
        root_uid: process.user_ns().root_uid(),
    };
    // A copy: exec runs single-threaded, so nothing else changes this
    // thread's sets, and no thread's sets lock is held while the
    // credentials are (the order of `Process::change_uids`).
    let caps = *thread.caps.lock();
    // A capability-dumb program was refused before the point of no return,
    // by the same rules on the same file and sets: they cannot refuse here.
    let outcome = caps
        .after_exec(old.ruid, old.euid, old.rgid, new_euid, new_egid, ctx)
        .map_err(|_| KernelError::PermissionDenied { operation: "exec" })?;
    let creds = old.after_exec(Some(outcome.euid), Some(outcome.egid), true);
    process.update_credentials(|c| *c = creds);
    // Linux's begin_new_exec and commit_creds: a process whose effective
    // IDs differed from its real ones, whose effective or filesystem IDs
    // exec changed, whose permitted capabilities grew, or which runs a
    // program it could not read, may not be traced or have its memory
    // read by its user (PR_GET_DUMPABLE 0); otherwise exec makes it
    // dumpable again. What counts is the result: a set-user-ID exec that
    // was downgraded (shared filesystem state) leaves it dumpable.
    let grew = !outcome.caps.permitted.is_subset_of(caps.permitted);
    *thread.caps.lock() = outcome.caps;
    let dumpable = ids.readable && old.exec_keeps_dumpable(&creds) && !grew;
    process.dumpable.store(dumpable, Ordering::Release);
    // The new memory belongs to the namespace exec ran in (mm_alloc), or
    // one above it that maps a file the caller could not read (would_dump).
    process.set_mm_user_ns(ids.mm_userns.clone());
    Ok(outcome.secure)
}

/// The last component of `path` (Linux's kbasename).
#[cfg(feature = "alloc")]
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Read the file at `path` for exec (`read_exec_node`). A failed lookup
/// is its own error (ENOTDIR, ELOOP, EACCES), not always ENOENT. The node
/// comes back too: the loader maps the file's pages from its page cache
/// (ADR 0010).
#[cfg(feature = "alloc")]
pub(crate) fn read_executable(
    path: &str,
    caller: &super::creds::Caller,
) -> Result<(alloc::sync::Arc<dyn crate::fs::VfsNode>, ExecRead), KernelError> {
    let vfs = crate::fs::get_vfs().as_caller();
    let (node, canonical) = vfs.resolve_path_canonical(path)?;
    refuse_noexec(mount_flags(&vfs, &canonical))?;
    let read = read_exec_node(&node, caller)?;
    Ok((node, read))
}

/// What exec read of a file: its contents, and the denial of writes to it
/// that lasts while it is being executed (ETXTBSY, `fs::write_access`).
#[cfg(feature = "alloc")]
pub(crate) type ExecRead = (Vec<u8>, crate::fs::write_access::WriteDenial);

/// Whether `caller` may execute `node`: a regular file with permission to
/// execute it (`fs::permission`: CAP_DAC_OVERRIDE needs at least one
/// execute bit, as on Linux), or `PermissionDenied` (EACCES).
#[cfg(feature = "alloc")]
pub fn check_executable(
    node: &dyn crate::fs::VfsNode,
    caller: &super::creds::Caller,
) -> Result<(), KernelError> {
    if !may_execute(&node.metadata()?, caller) {
        return Err(KernelError::PermissionDenied { operation: "exec" });
    }
    Ok(())
}

/// `node`'s contents, once `check_executable` passed on the same node, so
/// the file cannot be swapped between the check and the read (N-101).
/// Writes to it are denied first, after the permission check as in
/// Linux's do_open_execat: a file open for writing is ETXTBSY, and none
/// can change it under the read or while the denial is held.
#[cfg(feature = "alloc")]
fn read_exec_node(
    node: &alloc::sync::Arc<dyn crate::fs::VfsNode>,
    caller: &super::creds::Caller,
) -> Result<ExecRead, KernelError> {
    check_executable(&**node, caller)?;
    let denial = crate::fs::write_access::WriteDenial::deny(node)?;
    let mut data = alloc::vec![0u8; node.metadata()?.size];
    let n = node.read(0, &mut data)?;
    data.truncate(n);
    Ok((data, denial))
}

/// Whether `caller` may execute a node with metadata `meta`: a regular
/// file only.
#[cfg(feature = "alloc")]
fn may_execute(meta: &crate::fs::Metadata, caller: &super::creds::Caller) -> bool {
    meta.node_type == crate::fs::NodeType::File
        && crate::fs::permission(caller, meta, crate::fs::MAY_EXEC)
}

/// Execute a new program in current process
///
/// Replaces the current process image with a new program.
/// This function does not return on success - the new program begins execution.
///
/// Supports shebang (`#!`) scripts: if the file starts with `#!`, the
/// interpreter specified on the shebang line is executed instead, with the
/// script's name prepended to the argument list.
///
/// `before_commit` runs just before the point of no return, once the
/// program (and its interpreter and set-ID bits) checked out: the system
/// call ends the process's other threads there (Linux's de_thread in
/// begin_new_exec), so an exec that fails -- a script whose interpreter is
/// missing -- leaves them running (PR #23 review).
#[cfg(feature = "alloc")]
pub fn exec_process(
    file: ExecFile,
    argv: &[&str],
    envp: &[&str],
    before_commit: &dyn Fn() -> Result<(), KernelError>,
) -> Result<(), KernelError> {
    exec_at_depth(file, argv, envp, before_commit, 0)
}

/// How many `#!` interpreters an exec follows (Linux's
/// BINPRM_MAX_RECURSION): a script whose interpreter is itself, or two
/// naming each other, recursed until the kernel stack overflowed (PR #22
/// review). ELOOP past it.
pub const MAX_INTERPRETER_DEPTH: usize = 4;

/// Whether an exec `depth` interpreters deep may follow one more.
pub fn may_follow_interpreter(depth: usize) -> bool {
    depth < MAX_INTERPRETER_DEPTH
}

#[cfg(feature = "alloc")]
fn exec_at_depth(
    file: ExecFile,
    argv: &[&str],
    envp: &[&str],
    before_commit: &dyn Fn() -> Result<(), KernelError>,
    depth: usize,
) -> Result<(), KernelError> {
    let process = super::current_process().ok_or(KernelError::ProcessNotFound { pid: 0 })?;
    let current_thread = super::current_thread().ok_or(KernelError::ThreadNotFound { tid: 0 })?;

    // Step 1: read the program, checking execute permission on the node
    // that is read (N-101). A script's denial lasts while its interpreter
    // is set up, the program's is the process's once the exec commits.
    file.check_mount()?;
    let (file_data, denial) = read_exec_node(&file.node, &super::creds::Caller::current())?;

    // Step 1b: a script runs its interpreter with [interpreter, opt_arg?,
    // the script's name, argv[1..]] (argv[0], the script as the caller
    // named it, is dropped).
    if let Some((interpreter, opt_arg)) = parse_shebang(&file_data) {
        // The interpreter opens the script by name, which a close-on-exec
        // descriptor's /dev/fd/N will not be after the exec (Linux's
        // binfmt_script).
        if file.path_inaccessible {
            return Err(KernelError::FsError(crate::error::FsError::NotFound));
        }
        let mut new_argv: Vec<&str> = Vec::new();
        new_argv.push(interpreter.as_str());
        if let Some(arg) = &opt_arg {
            new_argv.push(arg.as_str());
        }
        new_argv.push(file.filename.as_str());
        if argv.len() > 1 {
            new_argv.extend_from_slice(&argv[1..]);
        }

        // Exec the interpreter; the set-user-ID and set-group-ID bits of
        // the script itself are ignored, as on Linux, and the command name
        // is the script's (N-227).
        if !may_follow_interpreter(depth) {
            return Err(KernelError::FsError(crate::error::FsError::SymlinkLoop));
        }
        exec_at_depth(
            ExecFile::open(&interpreter)?,
            &new_argv,
            envp,
            before_commit,
            depth + 1,
        )?;
        *current_thread.comm.lock() = super::thread::comm_from(file.comm.as_bytes());
        return Ok(());
    }

    exec_image(
        process,
        current_thread,
        file,
        (file_data, denial),
        argv,
        envp,
        before_commit,
    )
}

/// The rest of exec once the file is a program rather than a script. It is
/// kept out of `exec_at_depth`, which recurses for `#!` interpreters, so
/// each level of that recursion holds only a small frame: the image work
/// takes several KiB of stack in a debug build, and with file capabilities
/// a self-interpreting script overflowed the 64 KiB kernel stack on AArch64.
#[cfg(feature = "alloc")]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn exec_image(
    process: alloc::sync::Arc<super::Process>,
    current_thread: alloc::sync::Arc<super::Thread>,
    file: ExecFile,
    (file_data, denial): ExecRead,
    argv: &[&str],
    envp: &[&str],
    before_commit: &dyn Fn() -> Result<(), KernelError>,
) -> Result<(), KernelError> {
    use crate::elf::ElfLoader;

    let ExecFile {
        node: file_node,
        filename,
        exe_path,
        comm,
        mnt_flags,
        ..
    } = file;

    // Validate and plan the image before touching the current address
    // space (ADR 0010). exec must leave the caller intact when it fails;
    // parsing after clear() meant a non-ELF file (e.g. the empty
    // /proc/self/exe) destroyed the caller's mappings and the failed exec
    // returned into nothing.
    let not_elf = |_| KernelError::InvalidArgument {
        name: "elf",
        value: "not a loadable ELF image",
    };
    let elf_binary = ElfLoader::new().parse(&file_data).map_err(not_elf)?;
    let program_plan =
        crate::elf::image::plan(&elf_binary, file_data.len(), crate::elf::image::PIE_BASE)
            .map_err(|e| match e {
                // Too large to load: ENOMEM, as Linux's mapping would fail.
                crate::elf::ElfError::MemoryAllocationFailed => KernelError::OutOfMemory {
                    requested: 0,
                    available: 0,
                },
                e => not_elf(e),
            })?;

    // The interpreter (PT_INTERP) is opened, permission-checked and checked
    // before the point of no return as well, as Linux does: a missing or
    // non-executable loader fails the exec with the old image intact. Its
    // writes are denied for the exec only (`_interp_denial`), as Linux's.
    type Interp = (
        alloc::sync::Arc<dyn crate::fs::VfsNode>,
        Vec<u8>,
        crate::elf::ElfBinary,
    );
    let mut _interp_denial = None;
    let interp: Option<Interp> = match &elf_binary.interpreter {
        Some(path) => {
            let (node, (data, interp_denial)) =
                read_executable(path, &super::creds::Caller::current())?;
            _interp_denial = Some(interp_denial);
            let bad = |_| KernelError::InvalidArgument {
                name: "interpreter",
                value: "not a loadable ELF image",
            };
            let parsed = ElfLoader::new().parse(&data).map_err(bad)?;
            crate::elf::image::check_interpreter(&parsed, data.len()).map_err(bad)?;
            Some((node, data, parsed))
        }
        None => None,
    };

    // A set-user-ID or set-group-ID program runs as its file's owner or
    // group (Linux's bprm_fill_uid): the IDs the file offers, read with the
    // file. Whether they are granted is decided when they are installed.
    // A program (or loader) the caller may run but not read must not be
    // readable through the process either (Linux's would_dump).
    let set_ids = {
        let caller = super::creds::Caller::current();
        let readable =
            |meta: &crate::fs::Metadata| crate::fs::permission(&caller, meta, crate::fs::MAY_READ);
        let meta = file_node.metadata()?;
        let perms = meta.permissions;
        let interp_meta = match &interp {
            Some((node, ..)) => Some(node.metadata()?),
            None => None,
        };
        let interp_readable = interp_meta.as_ref().is_none_or(readable);
        let mut mm_userns = process.user_ns();
        for m in core::iter::once(&meta).chain(interp_meta.as_ref()) {
            if !readable(m) {
                mm_userns = mm_userns.ancestor_mapping(m.uid, m.gid);
            }
        }
        // A nosuid mount offers neither (Linux's mnt_may_suid, in
        // bprm_fill_uid and get_file_caps).
        let suid = mnt_flags & crate::fs::mount::flags::NOSUID == 0;
        let file_caps = if suid {
            exec_file_caps(&*file_node, &process.user_ns())?
        } else {
            None
        };
        // Neither when the caller's user namespace does not map both the
        // owner and the group (bprm_fill_uid's vfsuid_has_mapping): root
        // of a namespace of one's own gets nothing from a program it
        // could not have made set-ID itself.
        let ns = process.user_ns();
        let mapped = ns.uid_up(meta.uid).is_some() && ns.gid_up(meta.gid).is_some();
        let suid_ids = suid && mapped;
        ExecIds {
            set_uid: (suid_ids && perms.set_uid).then_some(meta.uid),
            set_gid: (suid_ids && perms.set_gid && perms.group_exec).then_some(meta.gid),
            readable: readable(&meta) && interp_readable,
            mm_userns,
            unprivileged_tracer_at_start: false,
            file_caps,
        }
    };

    // A program whose effective bit asks for capabilities it would not get
    // (a capability-dumb program would run without them) fails here, with
    // the caller intact (Linux's bprm_caps_from_vfs_caps, EPERM). The
    // check does not depend on what install decides later.
    if let Some(file) = set_ids.file_caps {
        let old = process.credentials();
        let ctx = super::linux_caps::ExecContext {
            file: Some(file),
            root_uid: process.user_ns().root_uid(),
            ..Default::default()
        };
        current_thread
            .caps
            .lock()
            .after_exec(
                old.ruid,
                old.euid,
                old.rgid,
                set_ids.set_uid.unwrap_or(old.euid),
                set_ids.set_gid.unwrap_or(old.egid),
                ctx,
            )
            .map_err(|_| KernelError::FsError(crate::error::FsError::OperationNotPermitted))?;
    }

    // BTI guarding, from the images' property notes: a malformed one is
    // ENOEXEC here, with the caller intact.
    let guards = crate::elf::image::bti_guards(
        (&elf_binary, &file_data),
        interp
            .as_ref()
            .map(|(_, data, parsed)| (parsed, data.as_slice())),
        crate::arch::bti_supported(),
    )
    .map_err(not_elf)?;

    // The other threads go now, with everything checked (see
    // `exec_process`).
    before_commit()?;

    // Under the credential guard, as ptrace attach and detach: who traces
    // the process as the exec starts (a tracer that detaches before the
    // set-ID decision still denies it), and that an exec is under way, so
    // PEEK and POKE do not write into the image being built (PR #23
    // review: a tracer could poke the new set-ID image, then detach before
    // the decision and have the privilege granted to its modified image).
    let unprivileged_tracer_at_start = {
        let _guard = process.cred_guard.lock();
        process
            .exec_in_progress
            .store(true, core::sync::atomic::Ordering::Release);
        unprivileged_tracer(&process)
    };
    let set_ids = ExecIds {
        unprivileged_tracer_at_start,
        ..set_ids
    };
    // An exec that may grant privilege maps the bytes it checked, not the
    // page cache (`elf::image::load`).
    let private_image =
        set_ids.file_caps.is_some() || set_ids.set_uid.is_some() || set_ids.set_gid.is_some();

    // Point of no return: from clear() on, a failure cannot go back to the
    // old image. The process is then killed with SIGSEGV at the system-call
    // exit (Linux force_sigsegv), instead of returning into an emptied
    // address space (N-101).
    let committed = (|| -> Result<_, KernelError> {
        // Step 2: Clear current address space and load new program; the
        // old image's size stays in the peak resident set, as on Linux.
        process.note_rss();
        *process.exe_path.lock() = exe_path.clone();
        // The new program's writes stay denied while it runs (Linux's
        // set_mm_exe_file); the old program's are allowed again.
        *process.exe_denial.lock() = Some(denial);
        let loaded = {
            let mut memory_space = process.memory_space.lock();

            // Clear existing mappings before loading new program
            memory_space.clear();
            // Nothing of the old image survives: a new image starts with a
            // thread pointer of 0 (as on Linux) unless its own PT_TLS sets
            // one, and the signal trampoline (a page of the old image) is
            // mapped again on the next handler (PR #19 review).
            process
                .tls_fs_base
                .store(0, core::sync::atomic::Ordering::Release);
            process
                .sigtramp
                .store(0, core::sync::atomic::Ordering::Release);

            // Reinitialize the address space for the new program
            memory_space.init()?;

            // Re-map the main thread's user stack into the fresh VAS. `clear()`
            // removed all user mappings; without this, the new image would return
            // to an unmapped stack (the /bin/sh crash).
            if let Some(main_tid) = process.get_main_thread_id() {
                if let Some(main_thread) = process.get_thread(main_tid) {
                    let user_base = main_thread.user_stack.base;
                    let user_size = main_thread.user_stack.size;
                    let flags = crate::mm::PageFlags::PRESENT
                        | crate::mm::PageFlags::USER
                        | crate::mm::PageFlags::WRITABLE
                        | crate::mm::PageFlags::NO_EXECUTE;
                    let pages = user_size / 4096;
                    for i in 0..pages {
                        let vaddr = user_base + i * 4096;
                        memory_space.map_page(vaddr, flags)?;
                    }
                    memory_space.set_stack_top(user_base + user_size);
                    memory_space.set_stack_size(user_size);
                }
            }

            // The program, and for a dynamic one its interpreter, which
            // runs first.
            crate::elf::image::load(
                &memory_space,
                (&elf_binary, &*file_node, &file_data, &program_plan),
                interp
                    .as_ref()
                    .map(|(node, data, binary)| (binary, &**node, data.as_slice())),
                guards,
                private_image,
            )?
        };

        // Linux uname mode: kept if the process chose it, otherwise what
        // the new program asks for (ADR 0021).
        process
            .uname_linux
            .exec(crate::elf::image::wants_linux_uname(
                &elf_binary,
                &file_data,
            ));

        // Step 2c: a static program gets its TLS block from the kernel
        // (the native libc relies on it); a dynamic one's loader sets up
        // TLS itself.
        if interp.is_none() {
            setup_static_tls(&process, &elf_binary, &file_data);
        }

        // The program's credentials, before the stack: AT_SECURE (and so
        // the loader's refusal of LD_PRELOAD and LD_LIBRARY_PATH) follows
        // from them.
        let secure = install_exec_credentials(&process, &current_thread, set_ids)?;
        // personality: READ_IMPLIES_EXEC goes, and with a secure exec
        // PER_CLEAR_ON_SETID too (Linux's bprm->per_clear).
        let persona = current_thread
            .personality
            .load(core::sync::atomic::Ordering::Acquire);
        current_thread.personality.store(
            super::personality::after_exec(persona, secure),
            core::sync::atomic::Ordering::Release,
        );
        // A new address space as far as kcmp(KCMP_VM) can tell.
        process.vm_id.store(
            super::pcb::new_vm_id(),
            core::sync::atomic::Ordering::Release,
        );
        // After unshare(CLONE_NEWTIME) the new program runs in the time
        // namespace for children (Linux's exec_task_namespaces).
        if let Some(set) = current_thread.namespaces().after_exec() {
            current_thread.set_namespaces(alloc::sync::Arc::new(set));
        }

        // Step 3: Setup new stack with arguments, environment, and aux vector
        let stack_top = setup_exec_stack(&process, argv, envp, &filename, &loaded.aux, secure)?;
        Ok((loaded.start, stack_top))
    })();
    process
        .exec_in_progress
        .store(false, core::sync::atomic::Ordering::Release);
    let (final_entry, stack_top) = match committed {
        Ok(v) => v,
        Err(e) => {
            super::exit::set_kill_pending(&process, 11);
            return Err(e);
        }
    };

    // Step 3b: Populate the process's env_vars BTreeMap from envp.
    // This makes environment variables available to kernel-side lookups
    // without reading user memory.
    {
        let mut env_map = process.env_vars.lock();
        env_map.clear();
        for &env_str in envp {
            if let Some(eq_pos) = env_str.find('=') {
                let key = String::from(&env_str[..eq_pos]);
                let value = String::from(&env_str[eq_pos + 1..]);
                env_map.insert(key, value);
            }
        }
    }

    // Step 4: Reset thread context to new entry point
    {
        let mut ctx = current_thread.context.lock();

        // Set new instruction pointer to program entry (interpreter entry
        // for dynamically linked binaries, binary entry for static)
        ctx.set_instruction_pointer(final_entry as usize);

        // Set stack pointer to new stack top
        ctx.set_stack_pointer(stack_top);

        // Clear return value (argc is passed differently)
        ctx.set_return_value(0);
    }

    // The exec can no longer fail: the process counts as exec'd from here
    // (setpgid's EACCES). Before the close-on-exec step, because a parent
    // takes EOF on a close-on-exec pipe as the sign of a completed exec and
    // may run on another CPU the moment the descriptor closes.
    process
        .did_exec
        .store(true, core::sync::atomic::Ordering::Release);

    // Step 5: Close file descriptors marked close-on-exec, in a table of
    // the process's own: one it shares with another process (CLONE_FILES)
    // is copied first, so the other keeps its descriptors (Linux's
    // unshare_files in begin_new_exec).
    process.unshare_files();
    process.file_table.lock().close_on_exec();

    // Step 5b: Only capabilities marked PRESERVE_EXEC survive exec (N-30).
    // The filtered space used to be built in sys_exec and then dropped, so
    // every capability survived (N-94). Done here, after the last step that
    // can fail, so a failed exec leaves the caller's capabilities intact.
    {
        let filtered = crate::cap::CapabilitySpace::new();
        {
            let old = process.capability_space.lock();
            if crate::cap::inheritance::exec_inherit_capabilities(&old, &filtered).is_err() {
                println!("[WARN] exec: some PRESERVE_EXEC capabilities were not kept");
            }
        }
        let old = core::mem::replace(&mut *process.capability_space.lock(), filtered);
        drop(old);
    }

    // Step 6: Reset signal handlers to defaults, and the alternate signal
    // stack (it was in the old image), as Linux does. POSIX timers are
    // deleted and their pending signals dropped (exit_itimers and
    // flush_itimer_signals); the interval timers stay.
    // Shared actions (CLONE_SIGHAND) are copied first, as de_thread's
    // unshare_sighand.
    process.unshare_sighand();
    process.reset_signal_handlers();
    super::posix_timer::delete_all(&process);
    process.rt_queue.clear_timers(&process.pending_signals);
    current_thread
        .rt_queue
        .clear_timers(&current_thread.sigpending);
    *current_thread.altstack.lock() = super::signals::SigAltStack::default();
    // The robust list was in the old image too (N-225).
    current_thread
        .robust_list
        .store(0, core::sync::atomic::Ordering::Release);
    // The command name is the program's file name (N-227).
    *current_thread.comm.lock() = super::thread::comm_from(comm.as_bytes());

    // The image is replaced: a parent waiting in vfork may run again; the
    // memory it shared with this process is no longer this process's.
    process.release_vfork_parent();

    // The actual execution resumes when we return to user mode
    // The modified thread context will cause execution at the new entry point
    Ok(())
}

#[cfg(not(feature = "alloc"))]
pub fn exec_process(
    _path: &str,
    _argv: &[&str],
    _envp: &[&str],
    _before_commit: &dyn Fn() -> Result<(), KernelError>,
) -> Result<(), KernelError> {
    Err(KernelError::NotImplemented {
        feature: "exec (requires alloc)",
    })
}

/// Write a value to a user-space stack address via the physical memory window.
///
/// The process's page tables map `vaddr` to a physical frame. We look up the
/// mapping and write through the identity-mapped physical address.
///
/// # Safety
///
/// `vaddr` must be a valid mapped address in the process's VAS with write
/// permissions. The caller must ensure no concurrent access to this memory.
#[cfg(feature = "alloc")]
unsafe fn write_to_user_stack(
    memory_space: &crate::mm::VirtualAddressSpace,
    vaddr: usize,
    value: usize,
) {
    // Delegate to write_bytes_to_user_stack which handles page-crossing writes.
    // While pointer writes are typically 16-byte aligned (and thus page-safe),
    // this ensures correctness regardless of alignment.
    let bytes = value.to_ne_bytes();
    // SAFETY: caller guarantees vaddr is valid and mapped with write access.
    unsafe {
        write_bytes_to_user_stack(memory_space, vaddr, &bytes);
    }
}

/// Write a byte slice to a user-space stack address via the physical memory
/// window.  Handles writes that cross page boundaries by translating each
/// page separately and copying only the bytes within that page.
///
/// # Safety
///
/// `vaddr` through `vaddr+data.len()-1` must be valid mapped addresses in the
/// process's VAS with write permissions.
#[cfg(feature = "alloc")]
unsafe fn write_bytes_to_user_stack(
    memory_space: &crate::mm::VirtualAddressSpace,
    vaddr: usize,
    data: &[u8],
) {
    use crate::mm::VirtualAddress;

    let pt_root = memory_space.get_page_table();
    if pt_root == 0 {
        return;
    }

    // SAFETY: pt_root is the non-zero L4 physical address owned by
    // `memory_space`, which is what create_mapper_from_root requires.
    let mapper = unsafe { super::super::mm::vas::create_mapper_from_root_pub(pt_root) };

    // Write in page-sized chunks to handle data that crosses page boundaries.
    // Each virtual page may map to a non-contiguous physical frame, so we must
    // translate each page separately and copy only the bytes within that page.
    let mut offset = 0usize;
    while offset < data.len() {
        let cur_vaddr = vaddr + offset;
        let page_offset = cur_vaddr & 0xFFF;
        let bytes_in_page = core::cmp::min(0x1000 - page_offset, data.len() - offset);

        if let Ok((frame, _flags)) = mapper.translate_page(VirtualAddress(cur_vaddr as u64)) {
            let phys_addr = (frame.as_u64() << 12) + page_offset as u64;
            // SAFETY: phys_addr is converted to a kernel-accessible virtual
            // address via phys_to_virt_addr. We copy exactly bytes_in_page
            // bytes, which does not exceed the page boundary.
            unsafe {
                let virt = crate::mm::phys_to_virt_addr(phys_addr);
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(offset),
                    virt as *mut u8,
                    bytes_in_page,
                );
            }
        }

        offset += bytes_in_page;
    }
}

/// Give a static program its initial TLS block from its PT_TLS segment, if
/// it has one, and record its thread pointer for the return to user mode
/// (`Process::tls_fs_base`): on x86_64 (variant II) FS_BASE points at the
/// TCB, which follows the TLS image; on AArch64 and RISC-V (variant I) the
/// thread pointer (TPIDR_EL0, `tp`) is the block base and the image lies
/// above it, after a 16-byte TCB on AArch64 and at it on RISC-V. The native
/// libc relies on this; musl sets up TLS itself and replaces it. Nothing is
/// done (and it is logged) if the block cannot be set up.
#[cfg(feature = "alloc")]
pub(crate) fn setup_static_tls(process: &Process, binary: &crate::elf::ElfBinary, data: &[u8]) {
    #[cfg(target_arch = "x86_64")]
    {
        let Some(tls) = binary
            .segments
            .iter()
            .find(|s| s.segment_type == crate::elf::SegmentType::Tls)
        else {
            return;
        };
        // The image (data+bss) rounded up to p_align, then the 8-byte TCB
        // self-pointer at the p_align-aligned thread pointer.
        let Some((size, tcb_offset)) =
            crate::elf::tls_layout(tls.memory_size as usize, tls.alignment)
        else {
            crate::println!(
                "[LOADER] PT_TLS alignment {} unsupported; leaving TLS to libc",
                tls.alignment
            );
            return;
        };
        let memory_space = process.memory_space.lock();
        let base = match memory_space.mmap(size, crate::mm::vas::MappingType::Data) {
            Ok(base) => base.as_u64(),
            Err(e) => {
                crate::println!("[LOADER] TLS block of {} bytes not mapped: {:?}", size, e);
                return;
            }
        };
        let tcb = base + tcb_offset as u64;
        // The initial image from the file; the rest of the block is zero.
        let from = tls.file_offset as usize;
        if let Some(init) = from
            .checked_add(tls.file_size as usize)
            .and_then(|to| data.get(from..to))
        {
            let _ = crate::elf::write_to_user_pages(&memory_space, base, init);
        }
        // %fs:0 is the TCB's own address.
        let _ = crate::elf::write_to_user_pages(&memory_space, tcb, &tcb.to_le_bytes());
        drop(memory_space);
        process
            .tls_fs_base
            .store(tcb, core::sync::atomic::Ordering::Release);
    }
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    {
        let Some(tls) = binary
            .segments
            .iter()
            .find(|s| s.segment_type == crate::elf::SegmentType::Tls)
        else {
            return;
        };
        #[cfg(target_arch = "aarch64")]
        const TCB_SIZE: usize = 16;
        #[cfg(target_arch = "riscv64")]
        const TCB_SIZE: usize = 0;
        let Some((size, image_offset)) =
            crate::elf::tls_layout_above_tp(tls.memory_size as usize, tls.alignment, TCB_SIZE)
        else {
            crate::println!(
                "[LOADER] PT_TLS alignment {} unsupported; leaving TLS to libc",
                tls.alignment
            );
            return;
        };
        let memory_space = process.memory_space.lock();
        let base = match memory_space.mmap(size, crate::mm::vas::MappingType::Data) {
            Ok(base) => base.as_u64(),
            Err(e) => {
                crate::println!("[LOADER] TLS block of {} bytes not mapped: {:?}", size, e);
                return;
            }
        };
        // The initial image from the file; the TCB and the rest are zero.
        let from = tls.file_offset as usize;
        if let Some(init) = from
            .checked_add(tls.file_size as usize)
            .and_then(|to| data.get(from..to))
        {
            let _ =
                crate::elf::write_to_user_pages(&memory_space, base + image_offset as u64, init);
        }
        drop(memory_space);
        process
            .tls_fs_base
            .store(base, core::sync::atomic::Ordering::Release);
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    let _ = (process, binary, data);
}

/// Set up the initial user stack of a program: its arguments, environment
/// and auxiliary vector (ADR 0010), written through the physical memory
/// window. Returns the stack pointer, which points at argc.
///
/// ```text
/// [high addresses]
///   program file name (AT_EXECFN), envp strings, argv strings
///   platform string (AT_PLATFORM)
///   16 random bytes (AT_RANDOM)
///   padding (16-byte alignment)
///   auxv pairs, ending with AT_NULL
///   NULL, envp pointers, NULL, argv pointers
///   argc                     <- SP (returned)
/// [low addresses]
/// ```
#[cfg(feature = "alloc")]
pub(crate) fn setup_exec_stack(
    process: &Process,
    argv: &[&str],
    envp: &[&str],
    execfn: &str,
    program: &crate::elf::dynamic::ProgramAux,
    secure: bool,
) -> Result<usize, KernelError> {
    let creds = process.credentials();
    // As the process's own user namespace sees them (create_elf_tables).
    let ns = process.user_ns();
    let process_aux = crate::elf::dynamic::ProcessAux {
        uid: ns.uid_munged(creds.ruid),
        euid: ns.uid_munged(creds.euid),
        gid: ns.gid_munged(creds.rgid),
        egid: ns.gid_munged(creds.egid),
        // Linux's secureexec, as exec decided it (`ExecOutcome::secure`).
        secure,
    };
    let mut random = [0u8; 16];
    crate::crypto::random::get_random()
        .fill_bytes(&mut random)
        .map_err(|_| KernelError::InvalidState {
            expected: "a seeded random number generator",
            actual: "AT_RANDOM bytes unavailable",
        })?;

    let memory_space = process.memory_space.lock();

    // Get stack region
    let stack_base = memory_space.user_stack_base();
    let stack_size = memory_space.user_stack_size();
    let stack_top = stack_base + stack_size;

    let layout = exec_stack_layout(stack_top, argv, envp, execfn, program, &process_aux);

    if layout.sp < stack_base {
        return Err(KernelError::OutOfMemory {
            requested: stack_top - layout.sp,
            available: stack_size,
        });
    }

    for &(addr, bytes) in &layout.strings {
        // SAFETY: addr is within the stack mapping (sp >= stack_base was
        // checked above and every string lies above sp). We write the
        // string bytes followed by a null terminator.
        unsafe {
            write_bytes_to_user_stack(&memory_space, addr, bytes);
            write_bytes_to_user_stack(&memory_space, addr + bytes.len(), &[0]);
        }
    }
    // SAFETY: the 16 bytes exec_stack_layout reserved above sp.
    unsafe {
        write_bytes_to_user_stack(&memory_space, layout.random_at, &random);
    }
    random.fill(0);

    // argc, argv pointers + NULL, envp pointers + NULL, auxv pairs.
    for (i, &word) in layout.words.iter().enumerate() {
        // SAFETY: the words fill the block exec_stack_layout reserved
        // between sp and the strings, and sp >= stack_base was checked
        // above, so every write is within the stack region.
        unsafe {
            write_to_user_stack(
                &memory_space,
                layout.sp + i * core::mem::size_of::<usize>(),
                word,
            );
        }
    }

    Ok(layout.sp)
}

/// The initial user stack of a program: where each string and the random
/// bytes go, the stack pointer, and the words written upward from it.
#[cfg(feature = "alloc")]
pub(crate) struct ExecStackLayout<'a> {
    /// Each string's address; a NUL follows it on the stack.
    pub(crate) strings: Vec<(usize, &'a [u8])>,
    /// Where the 16 AT_RANDOM bytes go.
    pub(crate) random_at: usize,
    /// The initial stack pointer (16-byte aligned), pointing at argc.
    pub(crate) sp: usize,
    /// argc, argv pointers, NULL, envp pointers, NULL, then each auxv
    /// entry as (type, value).
    pub(crate) words: Vec<usize>,
}

/// Lay out the stack [`setup_exec_stack`] writes below `stack_top`. Pure:
/// nothing is written, and the bounds check is left to the caller.
#[cfg(feature = "alloc")]
pub(crate) fn exec_stack_layout<'a>(
    stack_top: usize,
    argv: &[&'a str],
    envp: &[&'a str],
    execfn: &'a str,
    program: &crate::elf::dynamic::ProgramAux,
    process: &crate::elf::dynamic::ProcessAux,
) -> ExecStackLayout<'a> {
    let mut strings = Vec::with_capacity(argv.len() + envp.len() + 2);
    let mut top = stack_top;
    let mut place = |s: &'a str, strings: &mut Vec<(usize, &'a [u8])>| {
        top -= s.len() + 1; // +1 for the NUL
        strings.push((top, s.as_bytes()));
        top
    };

    // As Linux: the file name at the top, then envp and argv strings, each
    // list ascending in order.
    let execfn_at = place(execfn, &mut strings);
    let mut envp_addrs: Vec<usize> = envp.iter().rev().map(|e| place(e, &mut strings)).collect();
    envp_addrs.reverse();
    let mut argv_addrs: Vec<usize> = argv.iter().rev().map(|a| place(a, &mut strings)).collect();
    argv_addrs.reverse();
    let platform_at = place(crate::elf::dynamic::PLATFORM, &mut strings);
    let random_at = (top - 16) & !0xF;

    let auxv = crate::elf::dynamic::aux_vector(
        program,
        process,
        &crate::elf::dynamic::StackAux {
            random: random_at as u64,
            execfn: execfn_at as u64,
            platform: platform_at as u64,
        },
    );

    let words_needed = 1 + argv.len() + 1 + envp.len() + 1 + auxv.len() * 2;
    let sp = (random_at - words_needed * core::mem::size_of::<usize>()) & !0xF;

    let mut words = Vec::with_capacity(words_needed);
    words.push(argv.len());
    words.extend_from_slice(&argv_addrs);
    words.push(0);
    words.extend_from_slice(&envp_addrs);
    words.push(0);
    for entry in &auxv {
        words.push(entry.type_id as usize);
        words.push(entry.value as usize);
    }

    ExecStackLayout {
        strings,
        random_at,
        sp,
        words,
    }
}

/// The program's own capabilities, read with the file (Linux's
/// get_file_caps): a malformed value fails the exec (EINVAL); a filesystem
/// without attributes gives none; any other failure to read them fails the
/// exec, since a set-user-ID-root program's file capabilities may be what
/// limits it (get_vfs_caps_from_disk returns the error).
#[cfg(feature = "alloc")]
fn exec_file_caps(
    node: &dyn crate::fs::VfsNode,
    ns: &super::userns::UserNs,
) -> Result<Option<super::linux_caps::FileCaps>, KernelError> {
    match node.xattrs() {
        Ok(attrs) => match attrs.get(crate::fs::xattr::XATTR_NAME_CAPS) {
            Some(value) => super::linux_caps::FileCaps::from_xattr(value, ns).map_err(|_| {
                KernelError::InvalidArgument {
                    name: "security.capability",
                    value: "not a file capability",
                }
            }),
            None => Ok(None),
        },
        Err(KernelError::FsError(crate::error::FsError::NotSupported)) => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elf::dynamic::{AuxType, ProcessAux, ProgramAux};

    /// An exec whose file capabilities cannot be read fails rather than
    /// running without them (PR #33 review round 7); no attributes is none.
    #[test]
    fn an_exec_whose_caps_cannot_be_read_fails() {
        use alloc::sync::Arc;

        use crate::fs::{
            tests::{make_vfs_with_root, StuckCaps},
            Permissions,
        };
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let inner = root.create("prog", Permissions::from_mode(0o4755)).unwrap();
        assert!(matches!(
            exec_file_caps(&*inner, &crate::process::userns::initial()),
            Ok(None)
        ));
        let stuck = Arc::new(StuckCaps(inner, true));
        assert!(exec_file_caps(&*stuck, &crate::process::userns::initial()).is_err());
    }

    /// An exec follows at most four `#!` interpreters, as Linux, and then
    /// fails with ELOOP: a script naming itself recursed until the kernel
    /// stack overflowed (PR #22 review).
    #[test]
    fn interpreter_chains_are_bounded() {
        assert!((0..MAX_INTERPRETER_DEPTH).all(may_follow_interpreter));
        assert!(!may_follow_interpreter(MAX_INTERPRETER_DEPTH));
        assert_eq!(MAX_INTERPRETER_DEPTH, 4);
    }

    const TOP: usize = 0x7FFF_F000;

    /// The string written at `addr` in `layout`.
    fn string_at<'a>(layout: &ExecStackLayout<'a>, addr: usize) -> &'a [u8] {
        layout
            .strings
            .iter()
            .find(|&&(a, _)| a == addr)
            .map(|&(_, s)| s)
            .expect("pointer to a laid-out string")
    }

    fn program() -> ProgramAux {
        ProgramAux {
            phdr: Some(0x40_0040),
            phent: 56,
            phnum: 9,
            entry: 0x40_1000,
            base: 0,
        }
    }

    /// The word after argc's pointer lists and each auxv value.
    fn aux(layout: &ExecStackLayout<'_>, argc: usize, envc: usize, t: AuxType) -> usize {
        let pairs = &layout.words[1 + argc + 1 + envc + 1..];
        pairs
            .chunks(2)
            .find(|p| p[0] == t as usize)
            .map(|p| p[1])
            .expect("aux entry present")
    }

    #[test]
    fn exec_stack_layout_matches_the_sysv_abi() {
        let layout = exec_stack_layout(
            TOP,
            &["/bin/sh", "-c"],
            &["HOME=/"],
            "/bin/sh",
            &program(),
            &ProcessAux::default(),
        );

        assert_eq!(layout.words[0], 2);
        assert_eq!(string_at(&layout, layout.words[1]), b"/bin/sh");
        assert_eq!(string_at(&layout, layout.words[2]), b"-c");
        assert_eq!(layout.words[3], 0);
        assert_eq!(string_at(&layout, layout.words[4]), b"HOME=/");
        assert_eq!(layout.words[5], 0);
        // The vector ends the block with one AT_NULL.
        assert_eq!(&layout.words[layout.words.len() - 2..], &[0, 0]);

        // The file name at the very top, then envp above argv.
        let execfn = aux(&layout, 2, 1, AuxType::AtExecfn);
        assert_eq!(execfn, TOP - "/bin/sh".len() - 1);
        assert_eq!(string_at(&layout, execfn), b"/bin/sh");
        assert_eq!(layout.words[4], execfn - "HOME=/".len() - 1);
        assert_eq!(layout.words[2], layout.words[4] - "-c".len() - 1);
        assert_eq!(layout.words[1], layout.words[2] - "/bin/sh".len() - 1);

        // The platform string below them, the random bytes below it.
        let platform = aux(&layout, 2, 1, AuxType::AtPlatform);
        assert_eq!(
            string_at(&layout, platform),
            crate::elf::dynamic::PLATFORM.as_bytes()
        );
        assert_eq!(
            platform,
            layout.words[1] - crate::elf::dynamic::PLATFORM.len() - 1
        );
        assert_eq!(aux(&layout, 2, 1, AuxType::AtRandom), layout.random_at);
        assert!(layout.random_at + 16 <= platform);
        assert_eq!(layout.random_at % 16, 0);

        // sp is 16-byte aligned and the block ends below the random bytes.
        assert_eq!(layout.sp % 16, 0);
        assert!(layout.sp + layout.words.len() * 8 <= layout.random_at);
        assert_eq!(aux(&layout, 2, 1, AuxType::AtEntry), 0x40_1000);
        assert_eq!(aux(&layout, 2, 1, AuxType::AtPhnum), 9);
    }

    /// The stack pointer stays 16-byte aligned whatever the string
    /// lengths, and nothing overlaps.
    #[test]
    fn exec_stack_layout_alignment_for_any_string_length() {
        let args = ["a", "bb", "ccc", "dddd", "eeeee", "ffffff", "ggggggg"];
        for n in 0..args.len() {
            for top in [TOP, TOP - 3, TOP - 8] {
                let layout = exec_stack_layout(
                    top,
                    &args[..n],
                    &args[n..],
                    "x",
                    &program(),
                    &ProcessAux::default(),
                );
                assert_eq!(layout.sp % 16, 0);
                let lowest = layout.strings.iter().map(|&(a, _)| a).min().unwrap();
                assert!(layout.random_at + 16 <= lowest);
                assert!(layout.sp + layout.words.len() * 8 <= layout.random_at);
                let total: usize = args.iter().map(|s| s.len() + 1).sum::<usize>()
                    + 2
                    + crate::elf::dynamic::PLATFORM.len()
                    + 1;
                assert_eq!(lowest, top - total);
            }
        }
    }

    fn meta(node_type: crate::fs::NodeType, mode: u32, uid: u32, gid: u32) -> crate::fs::Metadata {
        crate::fs::Metadata {
            node_type,
            size: 0,
            permissions: crate::fs::Permissions::from_mode(mode),
            uid,
            gid,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 1,
        }
    }

    /// N-101: exec needs a regular file with an execute bit the caller
    /// holds; CAP_DAC_OVERRIDE (root) needs any execute bit, and never a
    /// directory.
    #[test]
    fn exec_permission_follows_linux_rules() {
        use crate::{
            fs::NodeType,
            process::creds::{Caller, Credentials},
        };
        let file = NodeType::File;
        let root = Caller::kernel();
        let user = |uid: u32, gid: u32| Caller {
            creds: Credentials::new(uid, gid),
            caps: crate::process::linux_caps::LinuxCaps::EMPTY,
            userns: crate::process::userns::initial(),
        };
        // Root: any execute bit, but at least one.
        assert!(!may_execute(&meta(file, 0o644, 0, 0), &root));
        assert!(may_execute(&meta(file, 0o001, 5, 5), &root));
        assert!(!may_execute(&meta(NodeType::Directory, 0o755, 0, 0), &root));
        // Owner, group and other classes.
        assert!(may_execute(
            &meta(file, 0o700, 1000, 1000),
            &user(1000, 1000)
        ));
        assert!(!may_execute(&meta(file, 0o700, 0, 0), &user(1000, 1000)));
        assert!(may_execute(&meta(file, 0o710, 0, 100), &user(1000, 100)));
        assert!(may_execute(&meta(file, 0o701, 0, 0), &user(1000, 1000)));
        // The owner class decides for the owner, even if others may run it.
        assert!(!may_execute(&meta(file, 0o611, 1000, 0), &user(1000, 1000)));
        // Uid 0 without CAP_DAC_OVERRIDE is held to the mode bits.
        assert!(!may_execute(&meta(file, 0o100, 5, 5), &user(0, 0)));
        assert!(may_execute(&meta(file, 0o100, 5, 5), &root));
    }

    /// execveat checks the node it looked up before copying argv, and an
    /// exec that fails the check is EACCES, not ENOENT as every exec error
    /// used to be (group 3 step D3c).
    #[test]
    fn check_executable_refuses_directories_and_plain_files() {
        use crate::{
            fs::{tests::make_vfs_with_root, Permissions},
            process::creds::Caller,
        };
        let vfs = make_vfs_with_root();
        let root = vfs.root_fs().unwrap().root();
        let dir = root.mkdir("d", Permissions::from_mode(0o755)).unwrap();
        let plain = root.create("plain", Permissions::from_mode(0o644)).unwrap();
        let prog = root.create("prog", Permissions::from_mode(0o755)).unwrap();
        let denied =
            |r: Result<(), KernelError>| matches!(r, Err(KernelError::PermissionDenied { .. }));
        assert!(denied(check_executable(&*dir, &Caller::kernel())));
        assert!(denied(check_executable(&*plain, &Caller::kernel())));
        assert!(check_executable(&*prog, &Caller::kernel()).is_ok());
    }
}

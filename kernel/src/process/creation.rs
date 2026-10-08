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
    lifecycle::create_scheduler_task,
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
    let stack_top = setup_exec_stack(&process, &argv_refs, &envp_refs, &options.name, &program)?;

    // Update the thread context with the adjusted stack pointer
    if let Some(thread) = process.get_thread(tid) {
        let mut ctx = thread.context.lock();
        ctx.set_stack_pointer(stack_top);
    }

    // Add process to process table
    table::add_process(process)?;

    // Mark process as ready
    if let Some(process) = table::get_process(pid) {
        process.set_state(ProcessState::Ready);

        // Add main thread to scheduler
        if let Some(thread) = process.get_thread(tid) {
            create_scheduler_task(&process, &thread)?;
        }
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

/// Search for an executable by name in PATH directories
///
/// If `name` contains a `/`, it is treated as an explicit path and returned
/// as-is (if it exists in the VFS). Otherwise, the function first checks the
/// current process's `env_vars` for a `PATH` entry (colon-separated list of
/// directories). If no `PATH` environment variable is set, it falls back to
/// the default search directories: `/bin`, `/usr/bin`, `/usr/local/bin`.
#[cfg(feature = "alloc")]
pub fn search_path(name: &str) -> Option<String> {
    // In the caller's view: under its root, from its working directory.
    let file_exists = |path: &str| {
        crate::fs::try_get_vfs().is_some_and(|vfs| vfs.as_caller().resolve_path(path).is_ok())
    };

    // If name already contains a slash, treat it as a path
    if name.contains('/') {
        if file_exists(name) {
            return Some(String::from(name));
        }
        return None;
    }

    // Try to read PATH from the current process's environment variables.
    let path_env: Option<String> = super::current_process().and_then(|proc| {
        let env = proc.env_vars.lock();
        env.get("PATH").cloned()
    });

    if let Some(ref path_val) = path_env {
        // Search each colon-separated directory in PATH.
        for dir in path_val.split(':') {
            if dir.is_empty() {
                continue;
            }
            let full_path = format!("{}/{}", dir, name);
            if file_exists(&full_path) {
                return Some(full_path);
            }
        }
    } else {
        // Fallback: standard search directories when no PATH env is set.
        const DEFAULT_SEARCH_DIRS: &[&str] = &["/bin", "/usr/bin", "/usr/local/bin"];

        for dir in DEFAULT_SEARCH_DIRS {
            let full_path = format!("{}/{}", dir, name);
            if file_exists(&full_path) {
                return Some(full_path);
            }
        }
    }

    None
}

/// Install the credentials a program runs with (Linux's
/// cred_guard_mutex section of exec): `set_ids` are the owner and group a
/// set-user-ID or set-group-ID file offers, granted unless the thread asked
/// for no new privileges, a tracer without privilege is attached, or
/// another process shares this one's directories and umask (CLONE_FS: it
/// could redirect the privileged program; LSM_UNSAFE_SHARE). Decided and
/// installed under `cred_guard`, which ptrace attach takes too: a tracer
/// attached before is seen here, and one attaching after sees the new
/// credentials.
#[cfg(feature = "alloc")]
fn install_exec_credentials(
    process: &super::Process,
    thread: &super::Thread,
    (set_uid, set_gid): (Option<u32>, Option<u32>),
) {
    use core::sync::atomic::Ordering;

    let _guard = process.cred_guard.lock();
    let traced_unprivileged = match process.tracer.load(Ordering::Acquire) {
        0 => false,
        tracer => super::table::get_process(super::ProcessId(tracer)).is_none_or(|t| t.euid() != 0),
    };
    // exec runs only in a single-threaded process, so this thread is the
    // process's only user of its filesystem state.
    let fs_shared = thread.fs().users.load(Ordering::Acquire) > 1;
    let may_gain =
        !thread.no_new_privs.load(Ordering::Acquire) && !traced_unprivileged && !fs_shared;
    let creds = process.credentials().after_exec(set_uid, set_gid, may_gain);
    process.update_credentials(|c| *c = creds);
}

/// The last component of `path` (Linux's kbasename).
#[cfg(feature = "alloc")]
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Read the file at `path` for exec: it must be a regular file the caller
/// (`uid`, `gid`) may execute -- root needs at least one execute bit, as on
/// Linux -- or the result is `PermissionDenied` (EACCES). The check and the
/// read use the same resolved node, so the file cannot be swapped between
/// them (N-101). The node comes back too: the loader maps the file's pages
/// from its page cache (ADR 0010).
#[cfg(feature = "alloc")]
pub(crate) fn read_executable(
    path: &str,
    creds: &super::creds::Credentials,
) -> Result<(alloc::sync::Arc<dyn crate::fs::VfsNode>, Vec<u8>), KernelError> {
    let node = crate::fs::get_vfs()
        .as_caller()
        .resolve_path(path)
        .map_err(|_| KernelError::FsError(crate::error::FsError::NotFound))?;
    let meta = node.metadata()?;
    if !may_execute(&meta, creds.euid, creds.gid_for(meta.gid)) {
        return Err(KernelError::PermissionDenied { operation: "exec" });
    }
    let mut data = alloc::vec![0u8; meta.size];
    let n = node.read(0, &mut data)?;
    data.truncate(n);
    Ok((node, data))
}

/// Whether `uid`/`gid` may execute a node with metadata `meta`.
#[cfg(feature = "alloc")]
fn may_execute(meta: &crate::fs::Metadata, uid: u32, gid: u32) -> bool {
    let perms = &meta.permissions;
    meta.node_type == crate::fs::NodeType::File
        && if uid == 0 {
            perms.owner_exec || perms.group_exec || perms.other_exec
        } else {
            perms.can_run(uid, gid, meta.uid, meta.gid)
        }
}

/// Execute a new program in current process
///
/// Replaces the current process image with a new program.
/// This function does not return on success - the new program begins execution.
///
/// Supports shebang (`#!`) scripts: if the file starts with `#!`, the
/// interpreter specified on the shebang line is executed instead, with the
/// script path prepended to the argument list. Also supports PATH search --
/// if the path does not start with `/`, standard directories are searched.
#[cfg(feature = "alloc")]
pub fn exec_process(path: &str, argv: &[&str], envp: &[&str]) -> Result<(), KernelError> {
    use crate::elf::ElfLoader;

    let process = super::current_process().ok_or(KernelError::ProcessNotFound { pid: 0 })?;
    let current_thread = super::current_thread().ok_or(KernelError::ThreadNotFound { tid: 0 })?;

    // Resolve path via PATH search if it doesn't start with '/'
    let resolved_path = if !path.starts_with('/') {
        search_path(path).ok_or(KernelError::FsError(crate::error::FsError::NotFound))?
    } else {
        String::from(path)
    };

    // Step 1: Load new program from filesystem, checking execute
    // permission on the node that is read (N-101).
    let (file_node, file_data) = read_executable(&resolved_path, &process.credentials())?;

    // Step 1b: Check for shebang (#!) and delegate to interpreter if found
    if let Some((interpreter, opt_arg)) = parse_shebang(&file_data) {
        // Build new argv: [interpreter, opt_arg?, script_path, original_argv[1..]]
        let mut new_argv: Vec<&str> = Vec::new();
        let interp_ref: &str = &interpreter;
        new_argv.push(interp_ref);

        // Borrow opt_arg for the lifetime of this block
        let opt_arg_string;
        if let Some(ref arg) = opt_arg {
            opt_arg_string = arg.clone();
            new_argv.push(&opt_arg_string);
        }

        let resolved_ref: &str = &resolved_path;
        new_argv.push(resolved_ref);

        // Append original argv[1..] (skip argv[0] which was the script name)
        if argv.len() > 1 {
            new_argv.extend_from_slice(&argv[1..]);
        }

        // Recursively exec the interpreter; the set-user-ID and
        // set-group-ID bits of the script itself are ignored, as on Linux,
        // and the command name is the script's (N-227).
        exec_process(&interpreter, &new_argv, envp)?;
        *current_thread.comm.lock() = super::thread::comm_from(basename(&resolved_path).as_bytes());
        return Ok(());
    }

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
            .map_err(not_elf)?;

    // The interpreter (PT_INTERP) is opened, permission-checked and checked
    // before the point of no return as well, as Linux does: a missing or
    // non-executable loader fails the exec with the old image intact.
    type Interp = (
        alloc::sync::Arc<dyn crate::fs::VfsNode>,
        Vec<u8>,
        crate::elf::ElfBinary,
    );
    let interp: Option<Interp> = match &elf_binary.interpreter {
        Some(path) => {
            let (node, data) = read_executable(path, &process.credentials())?;
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
    let set_ids = {
        let meta = file_node.metadata()?;
        let perms = meta.permissions;
        (
            perms.set_uid.then_some(meta.uid),
            (perms.set_gid && perms.group_exec).then_some(meta.gid),
        )
    };

    // Point of no return: from clear() on, a failure cannot go back to the
    // old image. The process is then killed with SIGSEGV at the system-call
    // exit (Linux force_sigsegv), instead of returning into an emptied
    // address space (N-101).
    let committed = (|| -> Result<_, KernelError> {
        // Step 2: Clear current address space and load new program; the
        // old image's size stays in the peak resident set, as on Linux.
        process.note_rss();
        *process.exe_path.lock() = resolved_path.clone();
        let loaded = {
            let mut memory_space = process.memory_space.lock();

            // Clear existing mappings before loading new program
            memory_space.clear();

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
            )?
        };

        // Step 2c: a static program gets its TLS block from the kernel
        // (the native libc relies on it); a dynamic one's loader sets up
        // TLS itself.
        if interp.is_none() {
            setup_static_tls(&process, &elf_binary, &file_data);
        }

        // The program's credentials, before the stack: AT_SECURE (and so
        // the loader's refusal of LD_PRELOAD and LD_LIBRARY_PATH) follows
        // from them.
        install_exec_credentials(&process, &current_thread, set_ids);

        // Step 3: Setup new stack with arguments, environment, and aux vector
        let stack_top = setup_exec_stack(&process, argv, envp, &resolved_path, &loaded.aux)?;
        Ok((loaded.start, stack_top))
    })();
    let (final_entry, stack_top) = match committed {
        Ok(v) => v,
        Err(e) => {
            let _ = process.kill_pending.compare_exchange(
                0,
                11,
                core::sync::atomic::Ordering::AcqRel,
                core::sync::atomic::Ordering::Acquire,
            );
            return Err(e);
        }
    };

    // Step 3b: Populate the process's env_vars BTreeMap from envp.
    // This makes environment variables available to kernel-side lookups
    // (e.g. PATH resolution in search_path()) without reading user memory.
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

    // Step 4b: Sync scheduler Task context with the updated thread context.
    // The scheduler has its own TaskContext (set at task creation) which must
    // match the thread's new entry point/stack, otherwise the scheduler will
    // resume at the old (pre-exec) address.
    {
        let sched = crate::sched::scheduler::current_scheduler().lock();
        if let Some(task_ptr) = sched.current() {
            // SAFETY: We are the currently running task and hold the scheduler
            // lock, so no other CPU will modify this Task concurrently.
            let task = unsafe { &mut *task_ptr.as_ptr() };
            task.context = crate::sched::task::TaskContext::new(final_entry as usize, stack_top);
        }
    }

    // Step 5: Close file descriptors marked close-on-exec
    {
        let file_table = process.file_table.lock();

        file_table.close_on_exec();
    }

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
    // stack (it was in the old image), as Linux does.
    process.reset_signal_handlers();
    *current_thread.altstack.lock() = super::signals::SigAltStack::default();
    // The robust list was in the old image too (N-225).
    current_thread
        .robust_list
        .store(0, core::sync::atomic::Ordering::Release);
    // The command name is the program's file name (N-227).
    *current_thread.comm.lock() = super::thread::comm_from(basename(&resolved_path).as_bytes());

    // The image is replaced: a parent waiting in vfork may run again; the
    // memory it shared with this process is no longer this process's.
    process.release_vfork_parent();
    process
        .did_exec
        .store(true, core::sync::atomic::Ordering::Release);

    // The actual execution resumes when we return to user mode
    // The modified thread context will cause execution at the new entry point
    Ok(())
}

#[cfg(not(feature = "alloc"))]
pub fn exec_process(_path: &str, _argv: &[&str], _envp: &[&str]) -> Result<(), KernelError> {
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
/// it has one (x86_64, variant II: FS_BASE points at the TCB, which follows
/// the TLS image), and record FS_BASE for the return to user mode. The
/// native libc relies on this; musl sets up TLS itself and replaces it.
/// Nothing is done (and it is logged) if the block cannot be set up.
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
    #[cfg(not(target_arch = "x86_64"))]
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
) -> Result<usize, KernelError> {
    let creds = process.credentials();
    let process_aux = crate::elf::dynamic::ProcessAux {
        uid: creds.ruid,
        euid: creds.euid,
        gid: creds.rgid,
        egid: creds.egid,
        // Linux's secureexec: the effective IDs differ from the real ones.
        secure: creds.euid != creds.ruid || creds.egid != creds.rgid,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elf::dynamic::{AuxType, ProcessAux, ProgramAux};

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
    /// holds; root needs any execute bit, and never a directory.
    #[test]
    fn exec_permission_follows_linux_rules() {
        use crate::fs::NodeType;
        let file = NodeType::File;
        // Root: any execute bit, but at least one.
        assert!(!may_execute(&meta(file, 0o644, 0, 0), 0, 0));
        assert!(may_execute(&meta(file, 0o001, 5, 5), 0, 0));
        assert!(!may_execute(&meta(NodeType::Directory, 0o755, 0, 0), 0, 0));
        // Owner, group and other classes.
        assert!(may_execute(&meta(file, 0o700, 1000, 1000), 1000, 1000));
        assert!(!may_execute(&meta(file, 0o700, 0, 0), 1000, 1000));
        assert!(may_execute(&meta(file, 0o710, 0, 100), 1000, 100));
        assert!(may_execute(&meta(file, 0o701, 0, 0), 1000, 1000));
        // The owner class decides for the owner, even if others may run it.
        assert!(!may_execute(&meta(file, 0o611, 1000, 0), 1000, 1000));
    }
}

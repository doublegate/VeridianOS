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
pub const DEFAULT_USER_STACK_SIZE: usize = 64 * 1024; // 64KB
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
    let main_thread =
        ThreadBuilder::new(pid, format!("{}-main", options.name), options.entry_point)
            .user_stack_size(options.user_stack_size)
            .kernel_stack_size(options.kernel_stack_size)
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
    }

    // Add thread to process
    process.add_thread(main_thread)?;

    // Setup user stack with arguments and environment
    // Convert String vectors to &str slices for setup_exec_stack
    let argv_refs: Vec<&str> = options.argv.iter().map(|s| s.as_str()).collect();
    let envp_refs: Vec<&str> = options.envp.iter().map(|s| s.as_str()).collect();

    // Get the process before adding to table so we can set up the stack
    let stack_top = setup_exec_stack(&process, &argv_refs, &envp_refs, None)?;

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
            create_scheduler_task(process, thread)?;
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
    use crate::fs;

    // If name already contains a slash, treat it as a path
    if name.contains('/') {
        if fs::file_exists(name) {
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
            if fs::file_exists(&full_path) {
                return Some(full_path);
            }
        }
    } else {
        // Fallback: standard search directories when no PATH env is set.
        const DEFAULT_SEARCH_DIRS: &[&str] = &["/bin", "/usr/bin", "/usr/local/bin"];

        for dir in DEFAULT_SEARCH_DIRS {
            let full_path = format!("{}/{}", dir, name);
            if fs::file_exists(&full_path) {
                return Some(full_path);
            }
        }
    }

    None
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
    use crate::{elf::ElfLoader, fs};

    let process = super::current_process().ok_or(KernelError::ProcessNotFound { pid: 0 })?;
    let current_thread = super::current_thread().ok_or(KernelError::ThreadNotFound { tid: 0 })?;

    // Resolve path via PATH search if it doesn't start with '/'
    let resolved_path = if !path.starts_with('/') {
        search_path(path).ok_or(KernelError::FsError(crate::error::FsError::NotFound))?
    } else {
        String::from(path)
    };

    // Step 1: Load new program from filesystem
    let file_data = fs::read_file(&resolved_path)
        .map_err(|_| KernelError::FsError(crate::error::FsError::NotFound))?;

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

        // Recursively exec the interpreter
        return exec_process(&interpreter, &new_argv, envp);
    }

    // Step 2: Clear current address space and load new program
    let entry_point = {
        let mut memory_space = process.memory_space.lock();

        // Clear existing mappings before loading new program
        memory_space.clear();

        // Reinitialize the address space for the new program
        memory_space.init()?;

        // Load ELF segments into address space and get entry point
        ElfLoader::load(&file_data, &mut memory_space)?
    };

    // Step 2b: Check for dynamic linking
    let (final_entry, aux_vector) = {
        let loader = ElfLoader::new();
        let elf_binary = loader
            .parse(&file_data)
            .map_err(|_| KernelError::InvalidArgument {
                name: "elf",
                value: "failed to parse ELF for dynamic linking check",
            })?;

        if elf_binary.dynamic && elf_binary.interpreter.is_some() {
            // Dynamically linked -- load interpreter and build aux vector
            let dyn_info = crate::elf::dynamic::prepare_dynamic_linking(
                &file_data,
                &elf_binary,
                elf_binary.load_base,
            )?
            .ok_or(KernelError::InvalidArgument {
                name: "dynamic",
                value: "binary has interpreter but prepare_dynamic_linking returned None",
            })?;

            // Load interpreter LOAD segments into the process address space.
            // The interpreter is a separate ELF loaded at its own base address
            // (distinct from the main binary) to avoid overlap.
            let interp_data = fs::read_file(&dyn_info.interp_path)
                .map_err(|_| KernelError::FsError(crate::error::FsError::NotFound))?;
            {
                let mut memory_space = process.memory_space.lock();
                let _interp_entry = ElfLoader::load(&interp_data, &mut memory_space)?;
            }

            // Entry point is the interpreter, not the main binary
            (dyn_info.interp_entry, Some(dyn_info.aux_vector))
        } else {
            // Statically linked -- use binary entry directly, no aux vector
            (entry_point, None)
        }
    };

    // Step 3: Setup new stack with arguments, environment, and aux vector
    let stack_top = setup_exec_stack(process, argv, envp, aux_vector.as_deref())?;

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

    // Step 6: Reset signal handlers to defaults
    process.reset_signal_handlers();

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
    use crate::mm::VirtualAddress;

    let pt_root = memory_space.get_page_table();
    if pt_root == 0 {
        return;
    }

    let mapper = unsafe { super::super::mm::vas::create_mapper_from_root_pub(pt_root) };
    if let Ok((frame, _flags)) = mapper.translate_page(VirtualAddress(vaddr as u64)) {
        let page_offset = vaddr & 0xFFF;
        let phys_addr = (frame.as_u64() << 12) + page_offset as u64;
        // SAFETY: phys_addr is converted to a kernel-accessible virtual
        // address via phys_to_virt_addr (required on x86_64 where physical
        // memory is mapped at a dynamic offset, not identity-mapped).
        unsafe {
            let virt = crate::mm::phys_to_virt_addr(phys_addr);
            core::ptr::write(virt as *mut usize, value);
        }
    }
}

/// Write a byte slice to a user-space stack address via the physical memory
/// window.
///
/// # Safety
///
/// Same requirements as `write_to_user_stack`. The range
/// `[vaddr, vaddr+data.len())` must be within a single mapped page.
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

    let mapper = unsafe { super::super::mm::vas::create_mapper_from_root_pub(pt_root) };
    if let Ok((frame, _flags)) = mapper.translate_page(VirtualAddress(vaddr as u64)) {
        let page_offset = vaddr & 0xFFF;
        let phys_addr = (frame.as_u64() << 12) + page_offset as u64;
        // SAFETY: phys_addr is converted to a kernel-accessible virtual
        // address via phys_to_virt_addr. The destination has at least
        // data.len() bytes available within the page.
        unsafe {
            let virt = crate::mm::phys_to_virt_addr(phys_addr);
            core::ptr::copy_nonoverlapping(data.as_ptr(), virt as *mut u8, data.len());
        }
    }
}

/// Setup stack for exec with arguments, environment, and optional auxiliary
/// vector.
///
/// Writes the full argc/argv/envp/auxv layout to the user stack via the
/// physical memory window. The layout (growing downward from stack_top) is:
///
/// ```text
/// [high addresses]
///   envp strings (null-terminated)
///   argv strings (null-terminated)
///   padding (16-byte alignment)
///   AT_NULL (0, 0)           <- auxv terminator (if present)
///   auxv[N-1] (type, value)
///   ...
///   auxv[0] (type, value)
///   NULL                     <- envp[N]
///   envp[N-1] pointer
///   ...
///   envp[0] pointer
///   NULL                     <- argv[argc]
///   argv[argc-1] pointer
///   ...
///   argv[0] pointer
///   argc (usize)             <- SP (returned)
/// [low addresses]
/// ```
#[cfg(feature = "alloc")]
fn setup_exec_stack(
    process: &Process,
    argv: &[&str],
    envp: &[&str],
    aux_vector: Option<&[crate::elf::dynamic::AuxVecEntry]>,
) -> Result<usize, KernelError> {
    let memory_space = process.memory_space.lock();

    // Get stack region
    let stack_base = memory_space.user_stack_base();
    let stack_size = memory_space.user_stack_size();
    let stack_top = stack_base + stack_size;

    // ---- Phase 1: Write strings from the top of the stack downward ----
    let mut string_sp = stack_top;

    // Write envp strings and record their user-space addresses
    let mut envp_addrs: Vec<usize> = Vec::with_capacity(envp.len());
    for &env in envp.iter().rev() {
        let bytes = env.as_bytes();
        string_sp -= bytes.len() + 1; // +1 for null terminator
                                      // SAFETY: string_sp is within the stack mapping. We write the string
                                      // bytes followed by a null terminator.
        unsafe {
            write_bytes_to_user_stack(&memory_space, string_sp, bytes);
            write_bytes_to_user_stack(&memory_space, string_sp + bytes.len(), &[0]);
        }
        envp_addrs.push(string_sp);
    }
    envp_addrs.reverse();

    // Write argv strings and record their user-space addresses
    let mut argv_addrs: Vec<usize> = Vec::with_capacity(argv.len());
    for &arg in argv.iter().rev() {
        let bytes = arg.as_bytes();
        string_sp -= bytes.len() + 1;
        // SAFETY: string_sp is within the stack mapping.
        unsafe {
            write_bytes_to_user_stack(&memory_space, string_sp, bytes);
            write_bytes_to_user_stack(&memory_space, string_sp + bytes.len(), &[0]);
        }
        argv_addrs.push(string_sp);
    }
    argv_addrs.reverse();

    // ---- Phase 2: Align and write pointer arrays ----
    // Align to 16 bytes
    let mut sp = string_sp & !0xF;

    // Ensure space for: argc + argv ptrs + NULL + envp ptrs + NULL + auxv entries
    // Each auxv entry is 2 usizes (type, value)
    let auxv_slots = aux_vector.map(|v| v.len() * 2).unwrap_or(0);
    let ptrs_needed = 1 + argv.len() + 1 + envp.len() + 1 + auxv_slots;
    sp -= ptrs_needed * core::mem::size_of::<usize>();
    // Re-align to 16 bytes (ABI requirement)
    sp &= !0xF;

    // DIAGNOSTIC: Check if sp is still within stack bounds
    if sp < stack_base {
        crate::kprintln!(
            "[STACK_SETUP] OVERFLOW! sp={:#x} < stack_base={:#x}, need {} bytes",
            sp,
            stack_base,
            stack_top - sp
        );
        return Err(KernelError::OutOfMemory {
            requested: stack_top - sp,
            available: stack_size,
        });
    }

    let mut write_pos = sp;

    // Write argc
    // SAFETY: write_pos is within the stack region.
    unsafe {
        write_to_user_stack(&memory_space, write_pos, argv.len());
    }
    write_pos += core::mem::size_of::<usize>();

    // Write argv pointers
    for &addr in &argv_addrs {
        // SAFETY: write_pos is within the stack region.
        unsafe {
            write_to_user_stack(&memory_space, write_pos, addr);
        }
        write_pos += core::mem::size_of::<usize>();
    }
    // NULL terminator for argv
    // SAFETY: write_pos is within the stack region.
    unsafe {
        write_to_user_stack(&memory_space, write_pos, 0);
    }
    write_pos += core::mem::size_of::<usize>();

    // Write envp pointers
    for &addr in &envp_addrs {
        // SAFETY: write_pos is within the stack region.
        unsafe {
            write_to_user_stack(&memory_space, write_pos, addr);
        }
        write_pos += core::mem::size_of::<usize>();
    }
    // NULL terminator for envp
    // SAFETY: write_pos is within the stack region.
    unsafe {
        write_to_user_stack(&memory_space, write_pos, 0);
    }
    write_pos += core::mem::size_of::<usize>();

    // Write auxiliary vector (if present, for dynamically linked binaries)
    if let Some(auxv) = aux_vector {
        for entry in auxv {
            // Each aux entry is two usize values: type, value
            // SAFETY: write_pos is within the stack region, reserved in
            // ptrs_needed calculation above.
            unsafe {
                write_to_user_stack(&memory_space, write_pos, entry.type_id as usize);
            }
            write_pos += core::mem::size_of::<usize>();
            // SAFETY: write_pos is within the stack region.
            unsafe {
                write_to_user_stack(&memory_space, write_pos, entry.value as usize);
            }
            write_pos += core::mem::size_of::<usize>();
        }
    }

    Ok(sp)
}

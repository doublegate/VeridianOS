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
use crate::{arch::context::ThreadContext, println};

/// Default stack sizes
pub const DEFAULT_USER_STACK_SIZE: usize = 8 * 1024 * 1024; // 8MB
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
pub fn create_process(name: String, entry_point: usize) -> Result<ProcessId, &'static str> {
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
) -> Result<ProcessId, &'static str> {
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

    // Add thread to process
    process.add_thread(main_thread)?;

    // Add process to process table
    table::add_process(process)?;

    // Mark process as ready
    if let Some(process) = table::get_process(pid) {
        process.set_state(ProcessState::Ready);

        // Add main thread to scheduler
        if let Some(thread) = process.get_thread(tid) {
            // Create a scheduler task for this thread
            create_scheduler_task(process, thread)?;
        }
    }

    println!(
        "[PROCESS] Created process {} ({}) with main thread {}",
        pid.0, options.name, tid.0
    );

    Ok(pid)
}

/// Execute a new program in current process
///
/// Replaces the current process image with a new program.
/// This function does not return on success - the new program begins execution.
#[cfg(feature = "alloc")]
pub fn exec_process(path: &str, argv: &[&str], envp: &[&str]) -> Result<(), &'static str> {
    use crate::{elf::ElfLoader, fs};

    let process = super::current_process().ok_or("No current process")?;
    let current_thread = super::current_thread().ok_or("No current thread")?;

    println!(
        "[PROCESS] exec() called for process {} with path: {}",
        process.pid.0, path
    );

    // Step 1: Load new program from filesystem
    let file_data = fs::read_file(path).map_err(|_| "Failed to read executable file")?;

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

    // Step 3: Setup new stack with arguments and environment
    let stack_top = setup_exec_stack(process, argv, envp)?;

    // Step 4: Reset thread context to new entry point
    {
        let mut ctx = current_thread.context.lock();

        // Set new instruction pointer to program entry
        ctx.set_instruction_pointer(entry_point as usize);

        // Set stack pointer to new stack top
        ctx.set_stack_pointer(stack_top);

        // Clear return value (argc is passed differently)
        ctx.set_return_value(0);
    }

    // Step 5: Close file descriptors marked close-on-exec
    {
        let file_table = process.file_table.lock();
        file_table.close_on_exec();
    }

    // Step 6: Reset signal handlers to defaults
    process.reset_signal_handlers();

    // Step 7: Update process name to reflect new executable
    #[cfg(feature = "alloc")]
    {
        // Extract filename from path
        let _name = path.rsplit('/').next().unwrap_or(path);
        // Note: Can't directly modify process.name since it's behind shared ref
        // In a full impl, we'd need interior mutability here
        println!(
            "[PROCESS] Process {} now executing: {}",
            process.pid.0, _name
        );
    }

    println!(
        "[PROCESS] exec() completed for process {}, entry: {:#x}",
        process.pid.0, entry_point
    );

    // The actual execution resumes when we return to user mode
    // The modified thread context will cause execution at the new entry point
    Ok(())
}

#[cfg(not(feature = "alloc"))]
pub fn exec_process(_path: &str, _argv: &[&str], _envp: &[&str]) -> Result<(), &'static str> {
    Err("exec requires alloc feature")
}

/// Setup stack for exec with arguments and environment
#[cfg(feature = "alloc")]
fn setup_exec_stack(
    process: &Process,
    argv: &[&str],
    envp: &[&str],
) -> Result<usize, &'static str> {
    let memory_space = process.memory_space.lock();

    // Get stack region (typically at end of user address space)
    let stack_base = memory_space.user_stack_base();
    let stack_size = memory_space.user_stack_size();
    let stack_top = stack_base + stack_size;

    // Layout: [env strings] [arg strings] [env pointers] [arg pointers] [argc]
    // Stack grows downward, so we start from top

    let mut sp = stack_top;

    // Align stack to 16 bytes
    sp &= !0xF;

    // Reserve space for strings and pointers
    // Calculate total string size
    let argv_total: usize = argv.iter().map(|s| s.len() + 1).sum();
    let envp_total: usize = envp.iter().map(|s| s.len() + 1).sum();

    // Push null terminator for envp array
    sp -= core::mem::size_of::<usize>();

    // Push envp pointers (will be filled in)
    let envp_ptrs_start = sp - (envp.len() * core::mem::size_of::<usize>());
    sp = envp_ptrs_start;

    // Push null terminator for argv array
    sp -= core::mem::size_of::<usize>();

    // Push argv pointers (will be filled in)
    let argv_ptrs_start = sp - (argv.len() * core::mem::size_of::<usize>());
    sp = argv_ptrs_start;

    // Push argc
    sp -= core::mem::size_of::<usize>();

    // Reserve space for strings
    sp -= argv_total + envp_total;

    // Align final sp to 16 bytes
    sp &= !0xF;

    // Note: In a full implementation, we would actually copy the strings
    // and pointers to the stack. For now, we just set up the layout.

    // The actual argument passing will be handled by the C runtime (crt0)
    // which expects argc at sp, argv at sp+8, envp at sp+16 (for 64-bit)

    // Store argc at stack pointer
    let _argc = argv.len();
    // In real implementation: unsafe { *(sp as *mut usize) = argc; }

    println!(
        "[PROCESS] Stack setup: base={:#x}, top={:#x}, sp={:#x}, argc={}",
        stack_base, stack_top, sp, _argc
    );

    Ok(sp)
}

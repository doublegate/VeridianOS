//! User program loader
//!
//! This module handles loading user programs from ELF binaries into process memory.

use crate::elf::ElfLoader;
use crate::process::{Process, ProcessId};
use alloc::string::String;
use alloc::vec::Vec;

/// Load a user program from ELF binary data
pub fn load_user_program(
    process: &mut Process,
    elf_data: &[u8],
) -> Result<u64, &'static str> {
    // Get process VAS
    let vas = process.memory_space_mut()
        .ok_or("No memory space")?;
    
    // Load ELF into process memory
    let entry_point = ElfLoader::load(elf_data, vas)?;
    
    Ok(entry_point)
}

/// Create a new process from an ELF binary
pub fn create_process_from_elf(
    name: String,
    elf_data: &[u8],
    _parent_pid: ProcessId,
) -> Result<ProcessId, &'static str> {
    use crate::process::lifecycle::create_process;
    
    // Create the process (entry_point is temporary, will be replaced)
    let pid = create_process(name, 0)?;
    
    // Get the process
    let process = crate::process::table::get_process_mut(pid)
        .ok_or("Process not found")?;
    
    // Load the program
    let entry_point = load_user_program(process, elf_data)?;
    
    // Set the entry point for the main thread
    if let Some(thread) = process.get_main_thread_mut() {
        thread.set_entry_point(entry_point as usize);
    }
    
    Ok(pid)
}

/// Execute a program by replacing current process image
pub fn exec_program(
    process: &mut Process,
    elf_data: &[u8],
    args: Vec<String>,
) -> Result<(), &'static str> {
    // Clear current memory space
    if let Some(vas) = process.memory_space_mut() {
        vas.clear_user_space()?;
    }
    
    // Load new program
    let entry_point = load_user_program(process, elf_data)?;
    
    // Update process state
    process.set_name(args.get(0).cloned().unwrap_or_else(|| String::from("unknown")));
    
    // Reset main thread to new entry point
    if let Some(thread) = process.get_main_thread_mut() {
        thread.set_entry_point(entry_point as usize);
        thread.reset_context();
    }
    
    Ok(())
}
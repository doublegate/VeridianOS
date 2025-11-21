//! User-space program loader
//!
//! This module handles loading user-space programs from the filesystem
//! and creating processes to execute them.

#![allow(clippy::slow_vector_initialization, clippy::explicit_auto_deref)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{string::String, vec, vec::Vec};

#[allow(unused_imports)]
use crate::{
    elf::{ElfError, ElfLoader},
    fs::get_vfs,
    println,
    process::{lifecycle, ProcessId},
};

/// Load and execute the init process
#[cfg(feature = "alloc")]
pub fn load_init_process() -> Result<ProcessId, &'static str> {
    println!("[LOADER] Loading init process...");

    // Try to load init from various locations
    let init_paths = [
        "/sbin/init",
        "/bin/init",
        "/usr/sbin/init",
        "/usr/bin/init",
        "/bin/sh", // Fallback to shell if no init found
    ];

    for path in &init_paths {
        match load_user_program(path, &[], &[]) {
            Ok(pid) => {
                println!("[LOADER] Successfully loaded init from {}", path);
                return Ok(pid);
            }
            Err(_e) => {
                println!("[LOADER] Failed to load {}: {}", path, _e);
            }
        }
    }

    // If no init binary found, create a minimal init process
    println!("[LOADER] No init binary found, creating minimal init process");
    create_minimal_init()
}

/// Load a user program from the filesystem
#[cfg(feature = "alloc")]
pub fn load_user_program(
    path: &str,
    argv: &[&str],
    envp: &[&str],
) -> Result<ProcessId, &'static str> {
    println!("[LOADER] Loading program: {}", path);

    // Open the file
    let file_node = get_vfs()
        .read()
        .open(path, crate::fs::file::OpenFlags::read_only())
        .map_err(|_| "Failed to open program file")?;

    // Get file size
    let metadata = file_node
        .metadata()
        .map_err(|_| "Failed to get file metadata")?;
    let file_size = metadata.size;

    // Read the entire file into memory
    let mut buffer = Vec::with_capacity(file_size);
    buffer.resize(file_size, 0);

    let bytes_read = file_node
        .read(0, &mut buffer)
        .map_err(|_| "Failed to read program file")?;

    if bytes_read != file_size {
        return Err("Failed to read entire program file");
    }

    // Create an ELF loader and parse the binary
    let loader = ElfLoader::new();
    let binary = loader.parse(&buffer).map_err(|e| match e {
        ElfError::InvalidMagic => "Invalid ELF magic number",
        ElfError::InvalidClass => "Invalid ELF class",
        ElfError::InvalidData => "Invalid ELF data encoding",
        ElfError::InvalidType => "Invalid ELF type",
        ElfError::UnsupportedMachine => "Unsupported machine architecture",
        ElfError::InvalidProgramHeader => "Invalid program header",
        ElfError::InvalidSymbol => "Invalid symbol",
        ElfError::MemoryAllocationFailed => "Memory allocation failed",
        ElfError::FileReadFailed => "File read failed",
        ElfError::RelocationFailed => "Relocation failed",
    })?;

    // Get the entry point
    let entry_point = binary.entry_point as usize;

    // Extract program name from path
    let name: String = path.rsplit('/').next().unwrap_or("unknown").into();

    // Convert arguments to owned strings
    let argv_vec: Vec<String> = argv.iter().map(|s| String::from(*s)).collect();
    let envp_vec: Vec<String> = envp.iter().map(|s| String::from(*s)).collect();

    // Create process with ELF entry point
    let options = lifecycle::ProcessCreateOptions {
        name: name.clone(),
        parent: None,
        priority: crate::process::ProcessPriority::Normal,
        entry_point,
        argv: argv_vec,
        envp: envp_vec,
        user_stack_size: lifecycle::DEFAULT_USER_STACK_SIZE,
        kernel_stack_size: lifecycle::DEFAULT_KERNEL_STACK_SIZE,
    };

    let pid = lifecycle::create_process_with_options(options)?;

    // Load the ELF segments into the process's address space
    if let Some(process) = crate::process::get_process(pid) {
        let mut memory_space = process.memory_space.lock();

        // Use the ELF loader to load the binary into the process's address space
        let entry = ElfLoader::load(&buffer, &mut *memory_space)?;

        // Verify the entry point matches
        if entry != binary.entry_point {
            return Err("Entry point mismatch after loading");
        }

        // Handle dynamic linking if needed
        if binary.dynamic {
            println!("[LOADER] Program requires dynamic linking");
            if let Some(interpreter) = &binary.interpreter {
                println!("[LOADER] Loading interpreter: {}", interpreter);

                // Load the dynamic linker/interpreter
                #[allow(unused_variables)]
                let interp_entry = load_dynamic_linker(process, interpreter, &binary)?;

                // When dynamic linking is used, execution starts at the interpreter
                // The interpreter will then load the main program
                println!("[LOADER] Interpreter loaded, entry: 0x{:x}", interp_entry);
            }
        }

        println!(
            "[LOADER] Successfully loaded program: {} (PID {})",
            name, pid.0
        );
    }

    Ok(pid)
}

/// Load the dynamic linker/interpreter for dynamically linked binaries
#[cfg(feature = "alloc")]
fn load_dynamic_linker(
    process: &crate::process::Process,
    interpreter_path: &str,
    _main_binary: &crate::elf::ElfBinary,
) -> Result<u64, &'static str> {
    use crate::mm::PageFlags;

    // Read the interpreter from filesystem
    let file_node = get_vfs()
        .read()
        .open(interpreter_path, crate::fs::file::OpenFlags::read_only())
        .map_err(|_| "Failed to open interpreter")?;

    let metadata = file_node
        .metadata()
        .map_err(|_| "Failed to get interpreter metadata")?;
    let file_size = metadata.size;

    let mut buffer = Vec::with_capacity(file_size);
    buffer.resize(file_size, 0);

    file_node
        .read(0, &mut buffer)
        .map_err(|_| "Failed to read interpreter")?;

    // Parse the interpreter ELF
    let loader = ElfLoader::new();
    let interp_binary = loader
        .parse(&buffer)
        .map_err(|_| "Failed to parse interpreter ELF")?;

    // Load interpreter at a high address to avoid collision with main binary
    // Standard Linux ld.so loads at 0x7f00_0000_0000 region
    let interp_base = 0x7F00_0000_0000_u64;

    let mut memory_space = process.memory_space.lock();

    // Map and load each segment of the interpreter
    for segment in &interp_binary.segments {
        if segment.segment_type != crate::elf::SegmentType::Load {
            continue;
        }

        // Calculate adjusted virtual address
        let adjusted_vaddr = interp_base + segment.virtual_addr;
        let page_start = adjusted_vaddr & !0xFFF;
        let page_end = (adjusted_vaddr + segment.memory_size + 0xFFF) & !0xFFF;
        let num_pages = ((page_end - page_start) / 0x1000) as usize;

        // Determine page flags
        let mut flags = PageFlags::USER | PageFlags::PRESENT;
        if (segment.flags & 0x2) != 0 {
            // PF_W
            flags |= PageFlags::WRITABLE;
        }
        if (segment.flags & 0x1) == 0 {
            // PF_X not set
            flags |= PageFlags::NO_EXECUTE;
        }

        // Map pages for this segment
        for i in 0..num_pages {
            let addr = page_start + (i as u64 * 0x1000);
            memory_space
                .map_page(addr as usize, flags)
                .map_err(|_| "Failed to map interpreter page")?;
        }

        // Copy segment data
        if segment.file_size > 0 {
            unsafe {
                let dest = adjusted_vaddr as *mut u8;
                let src = buffer.as_ptr().add(segment.file_offset as usize);
                core::ptr::copy_nonoverlapping(src, dest, segment.file_size as usize);
            }
        }

        // Zero BSS
        if segment.memory_size > segment.file_size {
            unsafe {
                let bss_start = (adjusted_vaddr + segment.file_size) as *mut u8;
                let bss_size = (segment.memory_size - segment.file_size) as usize;
                core::ptr::write_bytes(bss_start, 0, bss_size);
            }
        }
    }

    // Calculate interpreter entry point (adjusted for base address)
    let interp_entry = interp_base + interp_binary.entry_point;

    // Set up auxiliary vector (auxv) for the interpreter
    // This provides information about the main program to the dynamic linker
    setup_auxiliary_vector(process, _main_binary, interp_base)?;

    println!(
        "[LOADER] Dynamic linker loaded at 0x{:x}, entry: 0x{:x}",
        interp_base, interp_entry
    );

    Ok(interp_entry)
}

/// Set up the auxiliary vector for dynamic linking
#[cfg(feature = "alloc")]
#[allow(unused_variables)]
fn setup_auxiliary_vector(
    _process: &crate::process::Process,
    main_binary: &crate::elf::ElfBinary,
    interp_base: u64,
) -> Result<(), &'static str> {
    // Auxiliary vector types (from Linux elf.h)
    const AT_NULL: u64 = 0; // End of vector
    const AT_PHDR: u64 = 3; // Program headers for program
    const AT_PHENT: u64 = 4; // Size of program header entry
    const AT_PHNUM: u64 = 5; // Number of program headers
    const AT_PAGESZ: u64 = 6; // System page size
    const AT_BASE: u64 = 7; // Base address of interpreter
    const AT_ENTRY: u64 = 9; // Entry point of program
    const AT_UID: u64 = 11; // Real user ID
    const AT_EUID: u64 = 12; // Effective user ID
    const AT_GID: u64 = 13; // Real group ID
    const AT_EGID: u64 = 14; // Effective group ID

    // Build auxiliary vector entries
    let auxv: Vec<(u64, u64)> = vec![
        (AT_PAGESZ, 0x1000),                           // Page size
        (AT_BASE, interp_base),                        // Interpreter base
        (AT_ENTRY, main_binary.entry_point),           // Main program entry
        (AT_PHNUM, main_binary.segments.len() as u64), // Number of program headers
        (AT_PHENT, 56),                                // Size of program header (Elf64_Phdr)
        (AT_PHDR, main_binary.load_base),              // Program headers address
        (AT_UID, 0),                                   // Root user
        (AT_EUID, 0),
        (AT_GID, 0),
        (AT_EGID, 0),
        (AT_NULL, 0), // End of auxv
    ];

    // The auxiliary vector would typically be pushed onto the stack
    // after the environment pointers. For now, we just prepare the data.
    // The actual stack setup happens in the setup_args function.

    println!(
        "[LOADER] Auxiliary vector prepared with {} entries",
        auxv.len()
    );

    Ok(())
}

/// Create a minimal init process when no init binary is available
#[cfg(feature = "alloc")]
fn create_minimal_init() -> Result<ProcessId, &'static str> {
    println!("[LOADER] Creating minimal init process");

    // Create a simple init that just waits for children
    // This would normally be assembly code or a minimal binary
    // For now, we'll create a placeholder process

    // Entry point for minimal init (this would be actual code in memory)
    // For now, use a dummy address that will just loop
    let entry_point = 0x200000; // User-space address

    let options = lifecycle::ProcessCreateOptions {
        name: String::from("init"),
        parent: None,
        priority: crate::process::ProcessPriority::System,
        entry_point,
        argv: vec![String::from("init")],
        envp: Vec::new(),
        user_stack_size: 64 * 1024, // Smaller stack for minimal init
        kernel_stack_size: 16 * 1024,
    };

    let pid = lifecycle::create_process_with_options(options)?;

    // Set up minimal code at the entry point
    // This would normally be done by loading an actual binary
    if let Some(process) = crate::process::get_process(pid) {
        let mut memory_space = process.memory_space.lock();

        // Map a page for the minimal init code
        let page_flags = crate::mm::PageFlags::PRESENT | crate::mm::PageFlags::USER;
        // Note: Not setting NO_EXECUTE since we need to execute this code

        memory_space.map_page(entry_point, page_flags)?;

        // Write a simple infinite loop (architecture-specific)
        #[cfg(target_arch = "x86_64")]
        {
            // x86_64: jmp $ (eb fe)
            unsafe {
                let code_ptr = entry_point as *mut u8;
                *code_ptr = 0xeb;
                *code_ptr.add(1) = 0xfe;
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            // AArch64: b . (14000000)
            unsafe {
                let code_ptr = entry_point as *mut u32;
                *code_ptr = 0x14000000;
            }
        }

        #[cfg(target_arch = "riscv64")]
        {
            // RISC-V: j . (0000006f)
            unsafe {
                let code_ptr = entry_point as *mut u32;
                *code_ptr = 0x0000006f;
            }
        }

        println!("[LOADER] Minimal init process created (PID {})", pid.0);
    }

    Ok(pid)
}

/// Load the shell program
#[cfg(feature = "alloc")]
pub fn load_shell() -> Result<ProcessId, &'static str> {
    println!("[LOADER] Loading shell...");

    // Try to load a shell
    let shell_paths = [
        "/bin/vsh",  // VeridianOS shell
        "/bin/sh",   // Standard shell
        "/bin/bash", // Bash
        "/bin/ash",  // Ash (busybox)
    ];

    for path in &shell_paths {
        match load_user_program(
            path,
            &[path],
            &["PATH=/bin:/usr/bin", "HOME=/", "TERM=veridian"],
        ) {
            Ok(pid) => {
                println!("[LOADER] Successfully loaded shell from {}", path);
                return Ok(pid);
            }
            Err(_) => continue,
        }
    }

    // If no shell found, create a minimal shell
    println!("[LOADER] No shell binary found, creating minimal shell");
    create_minimal_shell()
}

/// Create a minimal shell when no shell binary is available
#[cfg(feature = "alloc")]
fn create_minimal_shell() -> Result<ProcessId, &'static str> {
    // Similar to minimal init, but configured as a shell
    let entry_point = 0x300000;

    let options = lifecycle::ProcessCreateOptions {
        name: String::from("vsh"),
        parent: Some(ProcessId(1)), // Child of init
        priority: crate::process::ProcessPriority::Normal,
        entry_point,
        argv: vec![String::from("vsh")],
        envp: vec![String::from("PATH=/bin"), String::from("HOME=/")],
        user_stack_size: lifecycle::DEFAULT_USER_STACK_SIZE,
        kernel_stack_size: lifecycle::DEFAULT_KERNEL_STACK_SIZE,
    };

    lifecycle::create_process_with_options(options)
}

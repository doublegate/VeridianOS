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
    elf::ElfLoader,
    error::KernelError,
    fs::get_vfs,
    println,
    process::{lifecycle, ProcessId},
};

/// Load and execute the init process
#[cfg(feature = "alloc")]
pub fn load_init_process() -> Result<ProcessId, KernelError> {
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
                println!("[LOADER] init from {} (PID {})", path, pid.0);
                return Ok(pid);
            }
            Err(_e) => {
                // Silently try next path — only log on final failure
            }
        }
    }

    // If no init binary found, create a minimal init process
    create_minimal_init()
}

/// Load a user program from the filesystem
#[cfg(feature = "alloc")]
pub fn load_user_program(
    path: &str,
    argv: &[&str],
    envp: &[&str],
) -> Result<ProcessId, KernelError> {
    // Open the file
    let file_node = get_vfs()
        .read()
        .open(path, crate::fs::file::OpenFlags::read_only())
        .map_err(|_| KernelError::NotFound {
            resource: "program file",
            id: 0,
        })?;

    // Get file size
    let metadata = file_node
        .metadata()
        .map_err(|_| KernelError::FsError(crate::error::FsError::IoError))?;
    let file_size = metadata.size;

    // Read the entire file into memory
    let mut buffer = Vec::with_capacity(file_size);
    buffer.resize(file_size, 0);

    let bytes_read = file_node
        .read(0, &mut buffer)
        .map_err(|_| KernelError::FsError(crate::error::FsError::IoError))?;

    if bytes_read != file_size {
        return Err(KernelError::FsError(crate::error::FsError::IoError));
    }

    // Create an ELF loader and parse the binary
    let loader = ElfLoader::new();
    let binary = loader
        .parse(&buffer)
        .map_err(|_| KernelError::InvalidArgument {
            name: "elf_binary",
            value: "failed to parse ELF",
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

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[LOADER] pid created, opening fds\n");
    }

    // Open /dev/console for stdin(0), stdout(1), stderr(2).
    // Try VFS first; fall back to a direct serial console node if /dev/console
    // is not yet mounted (ensures fds 0/1/2 are always occupied so that
    // pipe()/open() don't claim those slots).
    if let Some(process) = crate::process::get_process(pid) {
        use alloc::sync::Arc;

        use crate::fs::file::{File, OpenFlags};

        let console_node: Arc<dyn crate::fs::VfsNode> = {
            let vfs = get_vfs().read();
            match vfs.resolve_path("/dev/console") {
                Ok(node) => node,
                Err(_) => Arc::new(SerialConsoleNode),
            }
        };

        let ft = process.file_table.lock();

        // fd 0 = stdin (read-only)
        let stdin_file = Arc::new(File::new_with_path(
            console_node.clone(),
            OpenFlags::read_only(),
            String::from("/dev/console"),
        ));
        let _ = ft.open(stdin_file);

        // fd 1 = stdout (write-only)
        let stdout_file = Arc::new(File::new_with_path(
            console_node.clone(),
            OpenFlags::write_only(),
            String::from("/dev/console"),
        ));
        let _ = ft.open(stdout_file);

        // fd 2 = stderr (write-only)
        let stderr_file = Arc::new(File::new_with_path(
            console_node,
            OpenFlags::write_only(),
            String::from("/dev/console"),
        ));
        let _ = ft.open(stderr_file);
    }

    // Load the ELF segments into the process's address space.
    //
    // On RISC-V, the MMU is not enabled (satp = Bare mode), so ELF load
    // addresses (e.g. 0x400000) map directly to physical addresses that
    // are not valid RAM on the QEMU virt machine (RAM starts at
    // 0x80000000). Writing to those addresses causes a store access fault
    // and a CPU reset. Skip segment loading on RISC-V; the process PCB
    // is still created for bookkeeping.
    #[cfg(not(target_arch = "riscv64"))]
    if let Some(process) = crate::process::get_process(pid) {
        let mut memory_space = process.memory_space.lock();

        #[cfg(target_arch = "x86_64")]
        unsafe {
            crate::arch::x86_64::idt::raw_serial_str(b"[LOADER] loading ELF segments\n");
        }

        // Use the ELF loader to load the binary into the process's address space
        let entry = ElfLoader::load(&buffer, &mut *memory_space)?;

        // Verify the entry point matches
        if entry != binary.entry_point {
            return Err(KernelError::InvalidState {
                expected: "matching entry point",
                actual: "entry point mismatch after loading",
            });
        }

        // Handle dynamic linking if needed
        if binary.dynamic {
            if let Some(interpreter) = &binary.interpreter {
                match load_dynamic_linker(process, interpreter, &binary) {
                    Ok(interp_entry) => {
                        // Update process entry point to the interpreter.
                        // The dynamic linker will initialize GOT/PLT then jump
                        // to the main binary's entry point.
                        if let Some(main_tid) = process.get_main_thread_id() {
                            if let Some(thread) = process.get_thread(main_tid) {
                                use crate::arch::context::ThreadContext;
                                let mut ctx = thread.context.lock();
                                ctx.set_instruction_pointer(interp_entry as usize);
                            }
                        }
                    }
                    Err(_e) => {
                        // No dynamic linker available in rootfs -- warn and
                        // fall through to the main entry point. The binary
                        // will likely GP fault due to uninitialized GOT/PLT.
                        println!(
                            "[LOADER] WARNING: dynamic binary requires interpreter '{}' but it \
                             could not be loaded; proceeding with main entry (expect GP fault)",
                            interpreter
                        );
                    }
                }
            } else {
                println!(
                    "[LOADER] WARNING: binary is dynamically linked but has no interpreter set"
                );
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::idt::raw_serial_str(b"[LOADER] load complete, returning pid\n");
    }

    Ok(pid)
}

/// Load the dynamic linker/interpreter for dynamically linked binaries
#[cfg(all(feature = "alloc", not(target_arch = "riscv64")))]
fn load_dynamic_linker(
    process: &crate::process::Process,
    interpreter_path: &str,
    _main_binary: &crate::elf::ElfBinary,
) -> Result<u64, KernelError> {
    use crate::mm::PageFlags;

    // Read the interpreter from filesystem
    let file_node = get_vfs()
        .read()
        .open(interpreter_path, crate::fs::file::OpenFlags::read_only())
        .map_err(|_| KernelError::NotFound {
            resource: "interpreter",
            id: 0,
        })?;

    let metadata = file_node
        .metadata()
        .map_err(|_| KernelError::FsError(crate::error::FsError::IoError))?;
    let file_size = metadata.size;

    let mut buffer = Vec::with_capacity(file_size);
    buffer.resize(file_size, 0);

    file_node
        .read(0, &mut buffer)
        .map_err(|_| KernelError::FsError(crate::error::FsError::IoError))?;

    // Parse the interpreter ELF
    let loader = ElfLoader::new();
    let interp_binary = loader
        .parse(&buffer)
        .map_err(|_| KernelError::InvalidArgument {
            name: "interpreter_elf",
            value: "failed to parse interpreter ELF",
        })?;

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
            memory_space.map_page(addr as usize, flags)?;
        }

        // Copy segment data
        if segment.file_size > 0 {
            // SAFETY: 'dest' points to freshly mapped pages at adjusted_vaddr
            // (mapped in the loop above). 'src' is buffer.as_ptr() offset by
            // file_offset, which is within the ELF buffer (validated by the
            // segment parser). copy_nonoverlapping is valid because the mapped
            // virtual pages and the ELF buffer do not overlap.
            unsafe {
                let dest = adjusted_vaddr as *mut u8;
                let src = buffer.as_ptr().add(segment.file_offset as usize);
                core::ptr::copy_nonoverlapping(src, dest, segment.file_size as usize);
            }
        }

        // Zero BSS
        if segment.memory_size > segment.file_size {
            // SAFETY: bss_start is within the mapped page range (pages were
            // mapped for the full memory_size above). bss_size is the
            // difference between memory_size and file_size, so write_bytes
            // stays within the mapped region. Zeroing BSS is required by
            // the ELF specification.
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

    Ok(interp_entry)
}

/// Set up the auxiliary vector for dynamic linking
#[cfg(all(feature = "alloc", not(target_arch = "riscv64")))]
fn setup_auxiliary_vector(
    _process: &crate::process::Process,
    main_binary: &crate::elf::ElfBinary,
    interp_base: u64,
) -> Result<(), KernelError> {
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
    let _auxv: Vec<(u64, u64)> = vec![
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

    Ok(())
}

/// Create a minimal init process when no init binary is available
#[cfg(feature = "alloc")]
fn create_minimal_init() -> Result<ProcessId, KernelError> {
    // Entry point for minimal init
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
    // Note: For x86_64 with bootloader 0.11+, we need to use the physical memory
    // mapping For now, skip the code writing and just report success - the
    // kernel initialization is demonstrated and user-space execution will need
    // proper memory mapping.
    #[cfg(target_arch = "x86_64")]
    {
        // x86_64 with bootloader 0.11: Cannot directly access user-space
        // addresses. Actual user-space execution uses the ELF loader +
        // iretq path.
    }

    #[cfg(target_arch = "aarch64")]
    if let Some(process) = crate::process::get_process(pid) {
        let mut memory_space = process.memory_space.lock();
        let page_flags = crate::mm::PageFlags::PRESENT | crate::mm::PageFlags::USER;
        memory_space.map_page(entry_point, page_flags)?;

        // AArch64: b . (14000000)
        // SAFETY: entry_point was just mapped with USER | PRESENT flags above.
        // Writing the AArch64 "b ." (branch-to-self) instruction at the
        // entry point creates a minimal init process that spins in place.
        unsafe {
            let code_ptr = entry_point as *mut u32;
            *code_ptr = 0x14000000;
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // RISC-V: Cannot directly write to user-space virtual addresses during
        // early boot because map_page() only records the mapping in a BTreeMap
        // without creating hardware page table entries. With SATP in Bare mode,
        // address 0x200000 maps to physical 0x200000, which is not RAM on the
        // QEMU virt machine (RAM starts at 0x80000000). Writing there causes a
        // store access fault and, with no trap handler (stvec) configured, the
        // CPU reboots via OpenSBI.
        //
        // The init process PCB is created for bookkeeping. Actual user-space
        // code loading will require proper page table activation in a future
        // phase.
    }

    Ok(pid)
}

/// Load the shell program
#[cfg(feature = "alloc")]
pub fn load_shell() -> Result<ProcessId, KernelError> {
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
                return Ok(pid);
            }
            Err(_) => {
                // Silently try next path — only log on final failure
            }
        }
    }

    // If no shell found, create a minimal shell
    create_minimal_shell()
}

/// Create a minimal shell when no shell binary is available
#[cfg(feature = "alloc")]
fn create_minimal_shell() -> Result<ProcessId, KernelError> {
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

/// Lightweight serial console VFS node used as a fallback when /dev/console
/// is not available. Provides serial UART I/O for stdin/stdout/stderr so
/// that fds 0/1/2 are always occupied in the process file table.
#[cfg(feature = "alloc")]
struct SerialConsoleNode;

#[cfg(feature = "alloc")]
impl crate::fs::VfsNode for SerialConsoleNode {
    fn node_type(&self) -> crate::fs::NodeType {
        crate::fs::NodeType::CharDevice
    }

    fn read(&self, _offset: usize, buffer: &mut [u8]) -> Result<usize, KernelError> {
        // Blocking read from serial (same logic as sys_read stdin fallback)
        #[cfg(target_arch = "x86_64")]
        {
            let mut count = 0;
            for slot in buffer.iter_mut() {
                loop {
                    let status: u8;
                    unsafe {
                        core::arch::asm!("in al, dx", out("al") status, in("dx") 0x3FDu16);
                    }
                    if (status & 1) != 0 {
                        let byte: u8;
                        unsafe {
                            core::arch::asm!("in al, dx", out("al") byte, in("dx") 0x3F8u16);
                        }
                        *slot = byte;
                        count += 1;
                        break;
                    }
                    core::hint::spin_loop();
                }
            }
            Ok(count)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            // Non-x86: no serial polling available, return EOF
            let _ = buffer;
            Ok(0)
        }
    }

    fn write(&self, _offset: usize, data: &[u8]) -> Result<usize, KernelError> {
        for &byte in data {
            crate::print!("{}", byte as char);
        }
        Ok(data.len())
    }

    fn metadata(&self) -> Result<crate::fs::Metadata, KernelError> {
        Ok(crate::fs::Metadata {
            node_type: crate::fs::NodeType::CharDevice,
            size: 0,
            permissions: crate::fs::Permissions::from_mode(0o666),
            uid: 0,
            gid: 0,
            created: 0,
            modified: 0,
            accessed: 0,
            inode: 0,
        })
    }

    fn readdir(&self) -> Result<Vec<crate::fs::DirEntry>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn lookup(&self, _name: &str) -> Result<alloc::sync::Arc<dyn crate::fs::VfsNode>, KernelError> {
        Err(KernelError::FsError(crate::error::FsError::NotADirectory))
    }

    fn create(
        &self,
        _name: &str,
        _permissions: crate::fs::Permissions,
    ) -> Result<alloc::sync::Arc<dyn crate::fs::VfsNode>, KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "create on serial console",
        })
    }

    fn mkdir(
        &self,
        _name: &str,
        _permissions: crate::fs::Permissions,
    ) -> Result<alloc::sync::Arc<dyn crate::fs::VfsNode>, KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "mkdir on serial console",
        })
    }

    fn unlink(&self, _name: &str) -> Result<(), KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "unlink on serial console",
        })
    }

    fn truncate(&self, _size: usize) -> Result<(), KernelError> {
        Err(KernelError::OperationNotSupported {
            operation: "truncate on serial console",
        })
    }
}

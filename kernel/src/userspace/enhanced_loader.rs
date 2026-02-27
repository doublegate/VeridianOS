//! Enhanced ELF Loader for User Space Programs
//!
//! Loads ELF binaries from filesystem and executes them in user space.

#![allow(clippy::needless_borrow, clippy::op_ref)]

use alloc::{string::String, vec, vec::Vec};

use crate::{
    elf::{Elf64Header, Elf64ProgramHeader},
    error::KernelError,
    mm::{PageFlags, VirtualAddress},
    process::{Process, ProcessId},
};

/// Program arguments
pub struct ProgramArgs {
    /// Argument strings
    pub args: Vec<String>,

    /// Environment variables
    pub env: Vec<String>,
}

impl ProgramArgs {
    /// Create new program arguments
    pub fn new() -> Self {
        Self {
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    /// Add argument
    pub fn add_arg(&mut self, arg: String) {
        self.args.push(arg);
    }

    /// Add environment variable
    pub fn add_env(&mut self, env: String) {
        self.env.push(env);
    }
}

impl Default for ProgramArgs {
    fn default() -> Self {
        Self::new()
    }
}

/// ELF loader for user space programs
#[allow(dead_code)] // Alternative ELF loader -- fields used in load methods below
pub struct EnhancedElfLoader {
    /// ELF file data
    data: Vec<u8>,

    /// Parsed ELF header
    header: Option<Elf64Header>,

    /// Program headers
    program_headers: Vec<Elf64ProgramHeader>,

    /// Parsed entry point
    parsed_entry_point: u64,

    /// Requires dynamic linking
    requires_dynamic: bool,

    /// Interpreter path if dynamic
    interpreter_path: Option<String>,
}

impl EnhancedElfLoader {
    /// Create a new ELF loader from file data
    pub fn new(data: Vec<u8>) -> Result<Self, KernelError> {
        let mut loader = Self {
            data,
            header: None,
            program_headers: Vec::new(),
            parsed_entry_point: 0,
            requires_dynamic: false,
            interpreter_path: None,
        };

        loader.parse_header()?;
        loader.parse_program_headers()?;

        Ok(loader)
    }

    /// Parse ELF header
    fn parse_header(&mut self) -> Result<(), KernelError> {
        if self.data.len() < core::mem::size_of::<Elf64Header>() {
            return Err(KernelError::InvalidArgument {
                name: "elf_size",
                value: "too_small",
            });
        }

        // Check magic number (ELF magic: 0x7f, 'E', 'L', 'F')
        const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
        if &self.data[0..4] != &ELF_MAGIC {
            return Err(KernelError::InvalidArgument {
                name: "elf_magic",
                value: "invalid",
            });
        }

        // Check class (must be 64-bit)
        if self.data[4] != 2 {
            return Err(KernelError::InvalidArgument {
                name: "elf_class",
                value: "not_64bit",
            });
        }

        // Check data encoding (must be little endian)
        if self.data[5] != 1 {
            return Err(KernelError::InvalidArgument {
                name: "elf_encoding",
                value: "not_little_endian",
            });
        }

        // Parse header using unsafe pointer cast
        // SAFETY: We verified self.data.len() >= size_of::<Elf64Header>()
        // above, and checked the ELF magic, class, and encoding fields.
        // Elf64Header is #[repr(C)] with Copy, so reading via pointer
        // cast is valid. Alignment is not an issue because we copy
        // immediately via dereference.
        let header = unsafe { *(self.data.as_ptr() as *const Elf64Header) };

        // Validate type (must be executable or shared object)
        if header.elf_type != 2 && header.elf_type != 3 {
            return Err(KernelError::InvalidArgument {
                name: "elf_type",
                value: "not_executable",
            });
        }

        // Validate machine type for current architecture
        let valid_machine = match () {
            #[cfg(target_arch = "x86_64")]
            () => header.machine == 62,
            #[cfg(target_arch = "aarch64")]
            () => header.machine == 183,
            #[cfg(target_arch = "riscv64")]
            () => header.machine == 243,
        };

        if !valid_machine {
            return Err(KernelError::InvalidArgument {
                name: "elf_machine",
                value: "unsupported_architecture",
            });
        }

        // Store the entry point
        self.parsed_entry_point = header.entry;
        self.header = Some(header);

        println!(
            "[ELF-LOADER] ELF header parsed, entry: 0x{:x}",
            header.entry
        );

        Ok(())
    }

    /// Parse program headers
    fn parse_program_headers(&mut self) -> Result<(), KernelError> {
        let header = self.header.ok_or(KernelError::InvalidArgument {
            name: "elf_header",
            value: "not_parsed",
        })?;

        let ph_offset = header.phoff as usize;
        let ph_size = header.phentsize as usize;
        let ph_count = header.phnum as usize;

        // Validate program header table bounds
        if ph_offset + (ph_size * ph_count) > self.data.len() {
            return Err(KernelError::InvalidArgument {
                name: "program_headers",
                value: "out_of_bounds",
            });
        }

        // Parse each program header
        for i in 0..ph_count {
            let offset = ph_offset + (i * ph_size);

            // SAFETY: We verified ph_offset + (ph_size * ph_count) <=
            // self.data.len() above, so `offset` is within bounds.
            // Elf64ProgramHeader is #[repr(C)] with Copy. The pointer
            // is derived from a valid slice.
            let ph = unsafe { *(self.data[offset..].as_ptr() as *const Elf64ProgramHeader) };
            self.program_headers.push(ph);

            // Check for dynamic linking requirement (PT_INTERP = 3)
            if ph.p_type == 3 {
                self.requires_dynamic = true;

                // Extract interpreter path
                let interp_offset = ph.p_offset as usize;
                let interp_size = ph.p_filesz as usize;
                if interp_offset + interp_size <= self.data.len() {
                    let interp_data = &self.data[interp_offset..interp_offset + interp_size - 1]; // -1 for null terminator
                    if let Ok(interp_str) = core::str::from_utf8(interp_data) {
                        self.interpreter_path = Some(String::from(interp_str));
                    }
                }
            }

            // Check for dynamic section (PT_DYNAMIC = 2)
            if ph.p_type == 2 {
                self.requires_dynamic = true;
            }
        }

        println!(
            "[ELF-LOADER] Parsed {} program headers, dynamic: {}",
            ph_count, self.requires_dynamic
        );

        Ok(())
    }

    /// Load program into memory
    pub fn load(&self, process: &Process) -> Result<VirtualAddress, KernelError> {
        println!("[ELF-LOADER] Loading ELF program into process");

        let mut memory_space = process.memory_space.lock();

        // Process each LOAD segment (PT_LOAD = 1)
        for ph in &self.program_headers {
            if ph.p_type != 1 {
                continue; // Skip non-LOAD segments
            }

            // Calculate page-aligned addresses
            let page_mask = 0xFFF_u64;
            let page_start = ph.p_vaddr & !page_mask;
            let page_end = (ph.p_vaddr + ph.p_memsz + page_mask) & !page_mask;
            let num_pages = ((page_end - page_start) / 0x1000) as usize;

            // Determine page flags based on segment flags
            // PF_X = 0x1, PF_W = 0x2, PF_R = 0x4
            let mut flags = PageFlags::USER | PageFlags::PRESENT;
            if (ph.p_flags & 0x2) != 0 {
                flags |= PageFlags::WRITABLE;
            }
            if (ph.p_flags & 0x1) == 0 {
                flags |= PageFlags::NO_EXECUTE;
            }

            // Map pages for this segment
            for i in 0..num_pages {
                let addr = page_start + (i as u64 * 0x1000);
                memory_space.map_page(addr as usize, flags).map_err(|_| {
                    KernelError::OutOfMemory {
                        requested: 0x1000,
                        available: 0,
                    }
                })?;
            }

            // Copy segment data from file
            if ph.p_filesz > 0 {
                let src_offset = ph.p_offset as usize;
                let copy_size = ph.p_filesz as usize;

                if src_offset + copy_size <= self.data.len() {
                    // SAFETY: src_offset + copy_size is within self.data
                    // bounds (checked above). dest (p_vaddr) was just mapped
                    // into the process address space by map_page above. The
                    // source (ELF file data) and destination (process memory)
                    // do not overlap since they are in different address
                    // ranges.
                    unsafe {
                        let dest = ph.p_vaddr as *mut u8;
                        let src = self.data.as_ptr().add(src_offset);
                        core::ptr::copy_nonoverlapping(src, dest, copy_size);
                    }
                }
            }

            // Zero BSS portion (memory size > file size)
            if ph.p_memsz > ph.p_filesz {
                // SAFETY: The BSS region starts at p_vaddr + p_filesz and
                // extends to p_vaddr + p_memsz. This entire range was
                // mapped by map_page above (num_pages covers p_memsz).
                // write_bytes zeroes the uninitialized data section.
                unsafe {
                    let bss_start = (ph.p_vaddr + ph.p_filesz) as *mut u8;
                    let bss_size = (ph.p_memsz - ph.p_filesz) as usize;
                    core::ptr::write_bytes(bss_start, 0, bss_size);
                }
            }

            println!(
                "[ELF-LOADER] Loaded segment at 0x{:x}, size: {} bytes",
                ph.p_vaddr, ph.p_memsz
            );
        }

        let entry_point = VirtualAddress(self.parsed_entry_point);

        println!(
            "[ELF-LOADER] Program loaded, entry point: 0x{:x}",
            entry_point.as_u64()
        );

        Ok(entry_point)
    }

    /// Set up program arguments on stack
    /// Format on stack (growing downward):
    /// [padding for alignment]
    /// [env string N]
    /// ...
    /// [env string 0]
    /// [arg string argc-1]
    /// ...
    /// [arg string 0]
    /// [NULL] (envp terminator)
    /// [envp[N]]
    /// ...
    /// [envp[0]]
    /// [NULL] (argv terminator)
    /// [argv[argc-1]]
    /// ...
    /// [argv[0]]
    /// [argc]
    pub fn setup_args(&self, process: &Process, args: &ProgramArgs) -> Result<(), KernelError> {
        println!(
            "[ELF-LOADER] Setting up {} arguments and {} env vars",
            args.args.len(),
            args.env.len()
        );

        let mut memory_space = process.memory_space.lock();

        // User stack typically starts at a high address and grows down
        // Use the process's configured stack pointer
        let stack_top = 0x7FFF_FFFF_F000_u64; // Top of user stack (below kernel)
        let stack_size = 0x10000_u64; // 64KB stack

        // Map stack pages
        let flags =
            PageFlags::USER | PageFlags::WRITABLE | PageFlags::NO_EXECUTE | PageFlags::PRESENT;
        let num_stack_pages = (stack_size / 0x1000) as usize;
        for i in 0..num_stack_pages {
            let addr = stack_top - stack_size + (i as u64 * 0x1000);
            memory_space
                .map_page(addr as usize, flags)
                .map_err(|_| KernelError::OutOfMemory {
                    requested: 0x1000,
                    available: 0,
                })?;
        }

        // Start from top of stack and work down
        let mut sp = stack_top;

        // First, push all the string data (env vars then args)
        let mut env_ptrs: Vec<u64> = Vec::with_capacity(args.env.len());
        let mut arg_ptrs: Vec<u64> = Vec::with_capacity(args.args.len());

        // Push environment strings
        for env_str in args.env.iter().rev() {
            let bytes = env_str.as_bytes();
            sp -= (bytes.len() + 1) as u64; // +1 for null terminator
                                            // SAFETY: sp points into the user stack region that was mapped
                                            // above. We write bytes.len() + 1 bytes (string + null terminator).
                                            // sp was decremented by that exact amount, staying within the
                                            // mapped stack region (stack_top - stack_size to stack_top).
            unsafe {
                let ptr = sp as *mut u8;
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
                *ptr.add(bytes.len()) = 0; // Null terminator
            }
            env_ptrs.push(sp);
        }
        env_ptrs.reverse();

        // Push argument strings
        for arg_str in args.args.iter().rev() {
            let bytes = arg_str.as_bytes();
            sp -= (bytes.len() + 1) as u64;
            // SAFETY: Same as environment string push - sp is within
            // the mapped stack region and decremented by the exact
            // number of bytes written.
            unsafe {
                let ptr = sp as *mut u8;
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
                *ptr.add(bytes.len()) = 0;
            }
            arg_ptrs.push(sp);
        }
        arg_ptrs.reverse();

        // Align stack to 16 bytes
        sp &= !0xF;

        // Push NULL terminator for envp
        sp -= 8;
        // SAFETY: sp is 16-byte aligned (aligned above) and within the
        // mapped stack. Writing a u64 zero as the envp NULL terminator.
        unsafe {
            *(sp as *mut u64) = 0;
        }

        // Push envp pointers
        for env_ptr in env_ptrs.iter().rev() {
            sp -= 8;
            // SAFETY: sp is within the mapped stack and 8-byte aligned
            // (all writes are 8-byte u64). env_ptr contains a valid
            // address pointing to a string previously written to the stack.
            unsafe {
                *(sp as *mut u64) = *env_ptr;
            }
        }

        // Push NULL terminator for argv
        sp -= 8;
        // SAFETY: sp is within the mapped stack and 8-byte aligned.
        // Writing a u64 zero as the argv NULL terminator.
        unsafe {
            *(sp as *mut u64) = 0;
        }

        // Push argv pointers
        for arg_ptr in arg_ptrs.iter().rev() {
            sp -= 8;
            // SAFETY: sp is within the mapped stack and 8-byte aligned.
            // arg_ptr contains a valid address pointing to an argument
            // string previously written to the stack.
            unsafe {
                *(sp as *mut u64) = *arg_ptr;
            }
        }

        // Push argc
        sp -= 8;
        // SAFETY: sp is within the mapped stack and 8-byte aligned.
        // Writing argc as the topmost stack entry for the C ABI.
        unsafe {
            *(sp as *mut u64) = args.args.len() as u64;
        }

        println!(
            "[ELF-LOADER] Stack setup complete, sp: 0x{:x}, argc: {}",
            sp,
            args.args.len()
        );

        Ok(())
    }

    /// Get entry point address
    pub fn entry_point(&self) -> VirtualAddress {
        VirtualAddress(self.parsed_entry_point)
    }

    /// Check if binary requires dynamic linking
    pub fn requires_dynamic_linking(&self) -> bool {
        self.requires_dynamic
    }

    /// Get interpreter path if dynamic
    pub fn interpreter(&self) -> Option<&str> {
        self.interpreter_path.as_deref()
    }

    /// Get segment information for a given type
    pub fn segments(&self) -> &[Elf64ProgramHeader] {
        &self.program_headers
    }

    /// Get the raw ELF file data for segment copying.
    pub fn raw_data(&self) -> &[u8] {
        &self.data
    }

    /// Get total memory size needed
    pub fn memory_size(&self) -> usize {
        let mut min_addr = u64::MAX;
        let mut max_addr = 0u64;

        for ph in &self.program_headers {
            if ph.p_type == 1 {
                // PT_LOAD
                if ph.p_vaddr < min_addr {
                    min_addr = ph.p_vaddr;
                }
                let end = ph.p_vaddr + ph.p_memsz;
                if end > max_addr {
                    max_addr = end;
                }
            }
        }

        if min_addr == u64::MAX {
            0
        } else {
            (max_addr - min_addr) as usize
        }
    }
}

/// Load and execute an ELF program
pub fn load_and_execute(path: &str, args: ProgramArgs) -> Result<ProcessId, KernelError> {
    println!("[ELF-LOADER] Loading program: {}", path);

    // Read file from filesystem
    let vfs = crate::fs::get_vfs().read();
    let file_node = vfs
        .open(path, crate::fs::file::OpenFlags::read_only())
        .map_err(|_| KernelError::NotFound {
            resource: "file",
            id: 0,
        })?;

    // Get file size
    let metadata = file_node
        .metadata()
        .map_err(|_| KernelError::InvalidArgument {
            name: "metadata",
            value: "failed",
        })?;
    let file_size = metadata.size;

    // Read the entire file
    let mut elf_data = vec![0u8; file_size];

    let bytes_read =
        file_node
            .read(0, &mut elf_data)
            .map_err(|_| KernelError::InvalidArgument {
                name: "read",
                value: "failed",
            })?;

    if bytes_read != file_size {
        return Err(KernelError::InvalidArgument {
            name: "read",
            value: "incomplete",
        });
    }

    // Release the VFS lock before creating process
    drop(vfs);

    // Create ELF loader and parse
    let loader = EnhancedElfLoader::new(elf_data)?;

    // Extract program name from path
    let name: String = path.rsplit('/').next().unwrap_or("unknown").into();

    // Create new process
    println!("[ELF-LOADER] Creating new process for execution: {}", name);

    let create_options = crate::process::lifecycle::ProcessCreateOptions {
        name: name.clone(),
        parent: None,
        priority: crate::process::ProcessPriority::Normal,
        entry_point: loader.entry_point().as_usize(),
        argv: args.args.clone(),
        envp: args.env.clone(),
        user_stack_size: crate::process::lifecycle::DEFAULT_USER_STACK_SIZE,
        kernel_stack_size: crate::process::lifecycle::DEFAULT_KERNEL_STACK_SIZE,
    };

    let pid =
        crate::process::lifecycle::create_process_with_options(create_options).map_err(|_| {
            KernelError::ResourceExhausted {
                resource: "process",
            }
        })?;

    // Get the process and load program
    if let Some(process) = crate::process::get_process(pid) {
        // Load program segments into process memory
        let _entry = loader.load(&process)?;

        // Setup arguments on stack
        loader.setup_args(&process, &args)?;

        // Handle dynamic linking if needed
        if loader.requires_dynamic_linking() {
            if let Some(interp_path) = loader.interpreter() {
                println!("[ELF-LOADER] Loading interpreter: {}", interp_path);
                load_interpreter(&process, interp_path)?;
            }
        }

        println!(
            "[ELF-LOADER] Program loaded and ready to execute (PID {}, entry 0x{:x})",
            pid.0, _entry.0
        );
    }

    Ok(pid)
}

/// Load the dynamic linker/interpreter
fn load_interpreter(process: &Process, interp_path: &str) -> Result<VirtualAddress, KernelError> {
    println!("[ELF-LOADER] Loading interpreter from {}", interp_path);

    // Read interpreter from filesystem
    let vfs = crate::fs::get_vfs().read();
    let file_node = vfs
        .open(interp_path, crate::fs::file::OpenFlags::read_only())
        .map_err(|_| KernelError::NotFound {
            resource: "interpreter",
            id: 0,
        })?;

    let metadata = file_node
        .metadata()
        .map_err(|_| KernelError::InvalidArgument {
            name: "metadata",
            value: "failed",
        })?;
    let file_size = metadata.size;

    let mut interp_data = vec![0u8; file_size];

    file_node
        .read(0, &mut interp_data)
        .map_err(|_| KernelError::InvalidArgument {
            name: "read",
            value: "failed",
        })?;

    drop(vfs);

    // Parse interpreter ELF
    let interp_loader = EnhancedElfLoader::new(interp_data)?;

    // Load interpreter at a different base address to avoid collision
    // Typically interpreters are position-independent, so we load at a high address
    let interp_base = 0x7F00_0000_0000_u64;

    let mut memory_space = process.memory_space.lock();

    // Load each segment with offset
    for ph in interp_loader.segments() {
        if ph.p_type != 1 {
            continue; // Skip non-LOAD
        }

        let page_mask = 0xFFF_u64;
        let seg_vaddr = interp_base + ph.p_vaddr;
        let page_start = seg_vaddr & !page_mask;
        let page_end = (seg_vaddr + ph.p_memsz + page_mask) & !page_mask;
        let num_pages = ((page_end - page_start) / 0x1000) as usize;

        let mut flags = PageFlags::USER | PageFlags::PRESENT;
        if (ph.p_flags & 0x2) != 0 {
            flags |= PageFlags::WRITABLE;
        }
        if (ph.p_flags & 0x1) == 0 {
            flags |= PageFlags::NO_EXECUTE;
        }

        for i in 0..num_pages {
            let addr = page_start + (i as u64 * 0x1000);
            memory_space
                .map_page(addr as usize, flags)
                .map_err(|_| KernelError::OutOfMemory {
                    requested: 0x1000,
                    available: 0,
                })?;
        }

        // Zero BSS portion (p_memsz > p_filesz)
        if ph.p_memsz > ph.p_filesz {
            let bss_start = seg_vaddr + ph.p_filesz;
            let bss_len = (ph.p_memsz - ph.p_filesz) as usize;
            // SAFETY: bss_start is within the mapped region (seg_vaddr..seg_vaddr+p_memsz).
            unsafe {
                core::ptr::write_bytes(bss_start as *mut u8, 0, bss_len);
            }
        }

        // Copy real segment data from the interpreter ELF image.
        if ph.p_filesz > 0 {
            let src_offset = ph.p_offset as usize;
            let copy_len = ph.p_filesz as usize;
            let elf_data = interp_loader.raw_data();
            if src_offset + copy_len <= elf_data.len() {
                // SAFETY: seg_vaddr was mapped above. src is within the
                // loader's parsed ELF buffer. copy_len is bounded by p_filesz.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        elf_data.as_ptr().add(src_offset),
                        seg_vaddr as *mut u8,
                        copy_len,
                    );
                }
            }
        }
    }

    let interp_entry = VirtualAddress(interp_base + interp_loader.parsed_entry_point);
    println!(
        "[ELF-LOADER] Interpreter loaded at base 0x{:x}, entry 0x{:x}",
        interp_base,
        interp_entry.as_u64()
    );

    Ok(interp_entry)
}

/// Execute a built-in test program
pub fn execute_hello_world() -> Result<(), KernelError> {
    println!("[ELF-LOADER] Executing hello_world program");

    // Create arguments
    let mut args = ProgramArgs::new();
    args.add_arg(String::from("hello_world"));
    args.add_env(String::from("PATH=/bin:/usr/bin"));
    args.add_env(String::from("HOME=/"));

    // Load and execute
    let _pid = load_and_execute("/bin/hello_world", args)?;

    println!("[ELF-LOADER] hello_world execution started");

    Ok(())
}

/// Execute a program by path with default environment
pub fn execute_program(path: &str) -> Result<ProcessId, KernelError> {
    let mut args = ProgramArgs::new();
    args.add_arg(String::from(path));
    args.add_env(String::from("PATH=/bin:/usr/bin:/sbin:/usr/sbin"));
    args.add_env(String::from("HOME=/root"));
    args.add_env(String::from("TERM=vt100"));
    args.add_env(String::from("USER=root"));
    args.add_env(String::from("SHELL=/bin/vsh"));

    load_and_execute(path, args)
}

/// Initialize the enhanced ELF loader
pub fn init() -> Result<(), KernelError> {
    println!("[ELF-LOADER] Enhanced ELF loader initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_program_args() {
        let mut args = ProgramArgs::new();
        args.add_arg(String::from("test"));
        args.add_env(String::from("PATH=/bin"));

        assert_eq!(args.args.len(), 1);
        assert_eq!(args.env.len(), 1);
    }

    #[test]
    fn test_elf_magic_check() {
        // Valid ELF magic
        let data = vec![0x7f, b'E', b'L', b'F'];
        assert_eq!(&data[0..4], &[0x7f, b'E', b'L', b'F']);
    }

    #[test]
    fn test_program_args_default() {
        let args = ProgramArgs::default();
        assert_eq!(args.args.len(), 0);
        assert_eq!(args.env.len(), 0);
    }
}

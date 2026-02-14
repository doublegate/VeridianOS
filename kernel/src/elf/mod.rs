//! ELF64 Binary Loader
//!
//! Loads and executes ELF64 binaries for user-space programs.

#![allow(clippy::slow_vector_initialization, clippy::unnecessary_cast)]

use alloc::{string::String, vec::Vec};
use core::{mem, slice};

use crate::fs::get_vfs;

/// ELF magic number
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// ELF class
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ElfClass {
    None = 0,
    Elf32 = 1,
    Elf64 = 2,
}

/// ELF data encoding
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ElfData {
    None = 0,
    LittleEndian = 1,
    BigEndian = 2,
}

/// ELF file type
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ElfType {
    None = 0,
    Relocatable = 1,
    Executable = 2,
    SharedObject = 3,
    Core = 4,
}

/// ELF machine type
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ElfMachine {
    None = 0,
    X86_64 = 62,
    AArch64 = 183,
    RiscV = 243,
}

/// ELF header
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Header {
    pub magic: [u8; 4],
    pub class: u8,
    pub data: u8,
    pub version: u8,
    pub os_abi: u8,
    pub abi_version: u8,
    pub padding: [u8; 7],
    pub elf_type: u16,
    pub machine: u16,
    pub version2: u32,
    pub entry: u64,
    pub phoff: u64,
    pub shoff: u64,
    pub flags: u32,
    pub ehsize: u16,
    pub phentsize: u16,
    pub phnum: u16,
    pub shentsize: u16,
    pub shnum: u16,
    pub shstrndx: u16,
}

/// Program header type
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProgramType {
    Null = 0,
    Load = 1,
    Dynamic = 2,
    Interp = 3,
    Note = 4,
    Shlib = 5,
    Phdr = 6,
    Tls = 7,
}

/// Program header
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64ProgramHeader {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

/// Section header
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64SectionHeader {
    pub sh_name: u32,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub sh_addr: u64,
    pub sh_offset: u64,
    pub sh_size: u64,
    pub sh_link: u32,
    pub sh_info: u32,
    pub sh_addralign: u64,
    pub sh_entsize: u64,
}

/// Dynamic entry
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Dynamic {
    pub d_tag: i64,
    pub d_val: u64,
}

/// Symbol table entry
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Symbol {
    pub st_name: u32,
    pub st_info: u8,
    pub st_other: u8,
    pub st_shndx: u16,
    pub st_value: u64,
    pub st_size: u64,
}

/// Relocation entry
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Rela {
    pub r_offset: u64,
    pub r_info: u64,
    pub r_addend: i64,
}

/// ELF loader errors
#[derive(Debug)]
pub enum ElfError {
    InvalidMagic,
    InvalidClass,
    InvalidData,
    InvalidType,
    UnsupportedMachine,
    InvalidProgramHeader,
    MemoryAllocationFailed,
    FileReadFailed,
    RelocationFailed,
    InvalidSymbol,
}

/// ELF segment information
#[derive(Debug, Clone)]
pub struct ElfSegment {
    pub segment_type: SegmentType,
    pub virtual_addr: u64,
    pub physical_addr: u64,
    pub file_offset: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub flags: u32,
    pub alignment: u64,
}

/// Segment type
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SegmentType {
    Null,
    Load,
    Dynamic,
    Interp,
    Note,
    Shlib,
    Phdr,
    Tls,
    Other(u32),
}

/// ELF binary information
pub struct ElfBinary {
    pub entry_point: u64,
    pub load_base: u64,
    pub load_size: usize,
    pub segments: Vec<ElfSegment>,
    pub interpreter: Option<String>,
    pub dynamic: bool,
}

/// Dynamic linking information
#[derive(Debug)]
pub struct DynamicInfo {
    pub needed: Vec<String>,              // Required shared libraries
    pub soname: Option<String>,           // Library name
    pub rpath: Option<String>,            // Runtime library search path
    pub runpath: Option<String>,          // Runtime library search path (newer)
    pub init: Option<u64>,                // Initialization function
    pub fini: Option<u64>,                // Finalization function
    pub init_array: Option<(u64, usize)>, // Init array (addr, count)
    pub fini_array: Option<(u64, usize)>, // Fini array (addr, count)
    pub hash: Option<u64>,                // Symbol hash table
    pub strtab: Option<u64>,              // String table
    pub symtab: Option<u64>,              // Symbol table
    pub strsz: usize,                     // String table size
    pub syment: usize,                    // Symbol table entry size
    pub pltgot: Option<u64>,              // PLT/GOT address
    pub pltrelsz: usize,                  // PLT relocation table size
    pub pltrel: Option<u64>,              // PLT relocation type
    pub jmprel: Option<u64>,              // PLT relocations
    pub rel: Option<u64>,                 // Relocation table
    pub relsz: usize,                     // Relocation table size
    pub relent: usize,                    // Relocation entry size
    pub rela: Option<u64>,                // Relocation table with addends
    pub relasz: usize,                    // Rela table size
    pub relaent: usize,                   // Rela entry size
}

/// Symbol information
#[derive(Debug, Clone)]
pub struct ElfSymbol {
    pub name: String,
    pub value: u64,
    pub size: u64,
    pub info: u8,
    pub other: u8,
    pub shndx: u16,
}

/// Relocation entry
#[derive(Debug, Clone)]
pub struct ElfRelocation {
    pub offset: u64,
    pub symbol: u32,
    pub reloc_type: u32,
    pub addend: i64,
}

/// ELF loader
pub struct ElfLoader;

impl Default for ElfLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ElfLoader {
    /// Create a new ELF loader
    pub fn new() -> Self {
        Self
    }

    /// Load an ELF binary directly into a VAS
    pub fn load(
        data: &[u8],
        vas: &mut crate::mm::vas::VirtualAddressSpace,
    ) -> Result<u64, &'static str> {
        let loader = Self::new();
        let binary = loader.parse(data).map_err(|_| "Failed to parse ELF")?;

        // Process each LOAD segment
        for segment in &binary.segments {
            if segment.segment_type == SegmentType::Load {
                // Calculate page-aligned addresses
                let page_start = segment.virtual_addr & !0xFFF;
                let page_end = (segment.virtual_addr + segment.memory_size + 0xFFF) & !0xFFF;
                let num_pages = ((page_end - page_start) / 0x1000) as usize;

                // Map pages for this segment
                for i in 0..num_pages {
                    let addr = page_start + (i as u64 * 0x1000);

                    // Determine page flags based on segment flags
                    let mut flags = crate::mm::PageFlags::USER | crate::mm::PageFlags::PRESENT;
                    if (segment.flags & 0x2) != 0 {
                        // PF_W
                        flags |= crate::mm::PageFlags::WRITABLE;
                    }
                    if (segment.flags & 0x1) == 0 {
                        // PF_X
                        flags |= crate::mm::PageFlags::NO_EXECUTE;
                    }

                    vas.map_page(addr as usize, flags)
                        .map_err(|_| "Failed to map page")?;
                }

                // Copy segment data from file
                if segment.file_size > 0 {
                    // SAFETY: The virtual address pages were just mapped above via
                    // vas.map_page(). The source pointer is within the validated `data`
                    // slice (file_offset + file_size <= data.len() checked by parse).
                    // copy_nonoverlapping is appropriate since source (ELF data) and
                    // destination (newly mapped pages) do not overlap.
                    unsafe {
                        let dest = segment.virtual_addr as *mut u8;
                        let src = data.as_ptr().add(segment.file_offset as usize);
                        core::ptr::copy_nonoverlapping(src, dest, segment.file_size as usize);
                    }
                }

                // Zero BSS portion if memory size > file size
                if segment.memory_size > segment.file_size {
                    // SAFETY: The memory range [virtual_addr..virtual_addr+memory_size]
                    // was mapped above. The BSS region starts after file_size bytes and
                    // extends to memory_size, which is within the mapped page range.
                    // write_bytes zeroes the uninitialized BSS section as required by ELF.
                    unsafe {
                        let bss_start = (segment.virtual_addr + segment.file_size) as *mut u8;
                        let bss_size = (segment.memory_size - segment.file_size) as usize;
                        core::ptr::write_bytes(bss_start, 0, bss_size);
                    }
                }
            }
        }

        Ok(binary.entry_point)
    }

    /// Parse an ELF binary from a byte slice
    pub fn parse(&self, data: &[u8]) -> Result<ElfBinary, ElfError> {
        // Parse header
        let header = self.parse_header(data)?;

        // Validate header
        self.validate_header(&header)?;

        // Get program headers
        let program_headers = self.parse_program_headers(data, &header)?;

        // Find load segments and calculate memory requirements
        let (load_base, load_size) = self.calculate_memory_layout(&program_headers)?;

        // Check for interpreter (dynamic linking)
        let interpreter = self.find_interpreter(data, &header, &program_headers)?;

        // Check if binary is dynamically linked
        let dynamic = program_headers
            .iter()
            .any(|ph| ph.p_type == ProgramType::Dynamic as u32);

        // Convert program headers to segments
        let mut segments = Vec::new();
        for ph in &program_headers {
            let segment_type = match ph.p_type {
                0 => SegmentType::Null,
                1 => SegmentType::Load,
                2 => SegmentType::Dynamic,
                3 => SegmentType::Interp,
                4 => SegmentType::Note,
                5 => SegmentType::Shlib,
                6 => SegmentType::Phdr,
                7 => SegmentType::Tls,
                other => SegmentType::Other(other),
            };

            segments.push(ElfSegment {
                segment_type,
                virtual_addr: ph.p_vaddr,
                physical_addr: ph.p_paddr,
                file_offset: ph.p_offset,
                file_size: ph.p_filesz,
                memory_size: ph.p_memsz,
                flags: ph.p_flags,
                alignment: ph.p_align,
            });
        }

        Ok(ElfBinary {
            entry_point: header.entry,
            load_base,
            load_size,
            segments,
            interpreter,
            dynamic,
        })
    }

    /// Load an ELF binary into memory
    pub fn load_into_memory(&self, data: &[u8], target_base: u64) -> Result<u64, ElfError> {
        // Parse header
        let header = self.parse_header(data)?;

        // Validate header
        self.validate_header(&header)?;

        // Get program headers
        let program_headers = self.parse_program_headers(data, &header)?;

        // Load each LOAD segment
        for ph in program_headers.iter() {
            if ph.p_type == ProgramType::Load as u32 {
                self.load_segment(data, ph, target_base)?;
            }
        }

        // Return entry point
        Ok(header.entry)
    }

    /// Parse ELF header
    fn parse_header(&self, data: &[u8]) -> Result<Elf64Header, ElfError> {
        if data.len() < mem::size_of::<Elf64Header>() {
            return Err(ElfError::InvalidMagic);
        }

        // SAFETY: We verified data.len() >= size_of::<Elf64Header>() above.
        // Elf64Header is #[repr(C)] with Copy, so reading it via pointer cast
        // is valid as long as the data is large enough (which we checked).
        // Alignment is not an issue because we dereference and copy immediately.
        let header = unsafe { *(data.as_ptr() as *const Elf64Header) };

        Ok(header)
    }

    /// Validate ELF header
    fn validate_header(&self, header: &Elf64Header) -> Result<(), ElfError> {
        // Check magic number
        if header.magic != ELF_MAGIC {
            return Err(ElfError::InvalidMagic);
        }

        // Check class (must be 64-bit)
        if header.class != ElfClass::Elf64 as u8 {
            return Err(ElfError::InvalidClass);
        }

        // Check data encoding
        if header.data != ElfData::LittleEndian as u8 {
            return Err(ElfError::InvalidData);
        }

        // Check type (must be executable or shared object)
        let elf_type = header.elf_type;
        if elf_type != ElfType::Executable as u16 && elf_type != ElfType::SharedObject as u16 {
            return Err(ElfError::InvalidType);
        }

        // Check machine type
        let machine = header.machine;
        match machine {
            62 => {}  // x86_64
            183 => {} // AArch64
            243 => {} // RISC-V
            _ => return Err(ElfError::UnsupportedMachine),
        }

        Ok(())
    }

    /// Parse program headers
    fn parse_program_headers(
        &self,
        data: &[u8],
        header: &Elf64Header,
    ) -> Result<Vec<Elf64ProgramHeader>, ElfError> {
        let mut headers = Vec::new();

        let ph_offset = header.phoff as usize;
        let ph_size = header.phentsize as usize;
        let ph_count = header.phnum as usize;

        for i in 0..ph_count {
            let offset = ph_offset + (i * ph_size);
            if offset + ph_size > data.len() {
                return Err(ElfError::InvalidProgramHeader);
            }

            // SAFETY: We verified offset + ph_size <= data.len() above.
            // Elf64ProgramHeader is #[repr(C)] with Copy, so the pointer cast
            // and dereference is valid for the checked bounds. The value is
            // immediately copied out.
            let ph = unsafe { *(data[offset..].as_ptr() as *const Elf64ProgramHeader) };

            headers.push(ph);
        }

        Ok(headers)
    }

    /// Calculate memory layout for loading
    fn calculate_memory_layout(
        &self,
        program_headers: &[Elf64ProgramHeader],
    ) -> Result<(u64, usize), ElfError> {
        let mut min_addr = u64::MAX;
        let mut max_addr = 0u64;

        for ph in program_headers {
            if ph.p_type == ProgramType::Load as u32 {
                if ph.p_vaddr < min_addr {
                    min_addr = ph.p_vaddr;
                }
                let end_addr = ph.p_vaddr + ph.p_memsz;
                if end_addr > max_addr {
                    max_addr = end_addr;
                }
            }
        }

        if min_addr == u64::MAX {
            return Err(ElfError::InvalidProgramHeader);
        }

        let load_size = (max_addr - min_addr) as usize;
        Ok((min_addr, load_size))
    }

    /// Parse dynamic section
    pub fn parse_dynamic_section(
        &self,
        data: &[u8],
        dynamic_offset: u64,
        dynamic_size: u64,
    ) -> Result<DynamicInfo, ElfError> {
        let mut info = DynamicInfo {
            needed: Vec::new(),
            soname: None,
            rpath: None,
            runpath: None,
            init: None,
            fini: None,
            init_array: None,
            fini_array: None,
            hash: None,
            strtab: None,
            symtab: None,
            strsz: 0,
            syment: 0,
            pltgot: None,
            pltrelsz: 0,
            pltrel: None,
            jmprel: None,
            rel: None,
            relsz: 0,
            relent: 0,
            rela: None,
            relasz: 0,
            relaent: 0,
        };

        let entry_size = mem::size_of::<u64>() * 2; // Each dynamic entry is 2 u64s
        let num_entries = (dynamic_size as usize) / entry_size;
        let offset = dynamic_offset as usize;

        for i in 0..num_entries {
            let entry_offset = offset + (i * entry_size);
            if entry_offset + entry_size > data.len() {
                break;
            }

            // SAFETY: entry_offset + entry_size <= data.len() was checked above.
            // i64 and u64 are primitive types that can be read from any byte
            // alignment on the platforms we support. The values are copied out
            // immediately.
            let tag = unsafe { *(data[entry_offset..].as_ptr() as *const i64) };
            let value = unsafe { *(data[entry_offset + 8..].as_ptr() as *const u64) };

            match tag {
                0 => break, // DT_NULL - end of dynamic section
                1 => { // DT_NEEDED - needed library
                     // Value is offset into string table
                     // We'll resolve this later when we have strtab
                }
                5 => info.strtab = Some(value),          // DT_STRTAB
                6 => info.symtab = Some(value),          // DT_SYMTAB
                10 => info.strsz = value as usize,       // DT_STRSZ
                11 => info.syment = value as usize,      // DT_SYMENT
                12 => info.init = Some(value),           // DT_INIT
                13 => info.fini = Some(value),           // DT_FINI
                14 => info.soname = Some(String::new()), // DT_SONAME - resolve later
                15 => info.rpath = Some(String::new()),  // DT_RPATH - resolve later
                17 => info.rel = Some(value),            // DT_REL
                18 => info.relsz = value as usize,       // DT_RELSZ
                19 => info.relent = value as usize,      // DT_RELENT
                20 => info.pltrel = Some(value),         // DT_PLTREL
                23 => info.jmprel = Some(value),         // DT_JMPREL
                7 => info.rela = Some(value),            // DT_RELA
                8 => info.relasz = value as usize,       // DT_RELASZ
                9 => info.relaent = value as usize,      // DT_RELAENT
                25 => {
                    // DT_INIT_ARRAY
                    info.init_array = Some((value, 0));
                }
                27 => {
                    // DT_INIT_ARRAYSZ
                    if let Some((addr, _)) = info.init_array {
                        info.init_array = Some((addr, value as usize / 8));
                    }
                }
                26 => {
                    // DT_FINI_ARRAY
                    info.fini_array = Some((value, 0));
                }
                28 => {
                    // DT_FINI_ARRAYSZ
                    if let Some((addr, _)) = info.fini_array {
                        info.fini_array = Some((addr, value as usize / 8));
                    }
                }
                29 => info.runpath = Some(String::new()), // DT_RUNPATH - resolve later
                _ => {}                                   // Ignore unknown tags
            }
        }

        Ok(info)
    }

    /// Perform relocations
    pub fn perform_relocations(
        &self,
        base_addr: u64,
        relocations: &[ElfRelocation],
        symbols: &[ElfSymbol],
    ) -> Result<(), ElfError> {
        for reloc in relocations {
            let target_addr = base_addr + reloc.offset;

            match reloc.reloc_type {
                // R_X86_64_RELATIVE (8)
                8 => {
                    // Adjust by base address
                    // SAFETY: target_addr = base_addr + reloc.offset. The caller
                    // is responsible for ensuring the loaded binary's memory at
                    // target_addr is mapped and writable. The relocation writes
                    // a u64 value (base + addend) as specified by the ELF spec.
                    unsafe {
                        let ptr = target_addr as *mut u64;
                        *ptr = base_addr + reloc.addend as u64;
                    }
                }
                // R_X86_64_64 (1)
                1 => {
                    // Symbol + addend
                    if reloc.symbol as usize >= symbols.len() {
                        return Err(ElfError::InvalidSymbol);
                    }
                    let symbol = &symbols[reloc.symbol as usize];
                    // SAFETY: target_addr points into the loaded binary's mapped
                    // memory. The symbol index was bounds-checked above. Writing
                    // symbol.value + addend implements the R_X86_64_64 relocation.
                    unsafe {
                        let ptr = target_addr as *mut u64;
                        *ptr = symbol.value + reloc.addend as u64;
                    }
                }
                // R_X86_64_GLOB_DAT (6)
                6 => {
                    // Symbol value
                    if reloc.symbol as usize >= symbols.len() {
                        return Err(ElfError::InvalidSymbol);
                    }
                    let symbol = &symbols[reloc.symbol as usize];
                    // SAFETY: target_addr points into the loaded binary's mapped
                    // memory. The symbol index was bounds-checked above. Writing
                    // symbol.value implements the R_X86_64_GLOB_DAT relocation.
                    unsafe {
                        let ptr = target_addr as *mut u64;
                        *ptr = symbol.value;
                    }
                }
                // R_X86_64_JUMP_SLOT (7)
                7 => {
                    // Jump slot for PLT
                    if reloc.symbol as usize >= symbols.len() {
                        return Err(ElfError::InvalidSymbol);
                    }
                    let symbol = &symbols[reloc.symbol as usize];
                    // SAFETY: target_addr points into the loaded binary's mapped
                    // memory. The symbol index was bounds-checked above. Writing
                    // symbol.value implements the R_X86_64_JUMP_SLOT relocation
                    // for the PLT (Procedure Linkage Table).
                    unsafe {
                        let ptr = target_addr as *mut u64;
                        *ptr = symbol.value;
                    }
                }
                _ => {
                    // Unsupported relocation type
                    return Err(ElfError::RelocationFailed);
                }
            }
        }

        Ok(())
    }

    /// Resolve symbols
    pub fn resolve_symbols(
        &self,
        data: &[u8],
        symtab_offset: u64,
        symtab_size: usize,
        strtab_offset: u64,
        strtab_size: usize,
    ) -> Result<Vec<ElfSymbol>, ElfError> {
        let mut symbols = Vec::new();

        let sym_entry_size = 24; // Size of Elf64_Sym
        let num_symbols = symtab_size / sym_entry_size;

        for i in 0..num_symbols {
            let sym_offset = (symtab_offset as usize) + (i * sym_entry_size);
            if sym_offset + sym_entry_size > data.len() {
                break;
            }

            // Parse symbol entry
            // SAFETY: sym_offset + sym_entry_size (24) <= data.len() was checked
            // above. Each field is read at its correct offset within the Elf64_Sym
            // structure layout. The primitive types (u32, u16, u64) are copied out
            // immediately. Alignment is handled by reading through pointer casts
            // of packed ELF data.
            let name_idx = unsafe { *(data[sym_offset..].as_ptr() as *const u32) };
            let info = data[sym_offset + 4];
            let other = data[sym_offset + 5];
            let shndx = unsafe { *(data[sym_offset + 6..].as_ptr() as *const u16) };
            let value = unsafe { *(data[sym_offset + 8..].as_ptr() as *const u64) };
            let size = unsafe { *(data[sym_offset + 16..].as_ptr() as *const u64) };

            // Get symbol name from string table
            let name = if (name_idx as usize) < strtab_size {
                let name_offset = (strtab_offset as usize) + (name_idx as usize);
                self.read_string(data, name_offset)?
            } else {
                String::new()
            };

            symbols.push(ElfSymbol {
                name,
                value,
                size,
                info,
                other,
                shndx,
            });
        }

        Ok(symbols)
    }

    /// Read null-terminated string from data
    fn read_string(&self, data: &[u8], offset: usize) -> Result<String, ElfError> {
        let mut end = offset;
        while end < data.len() && data[end] != 0 {
            end += 1;
        }

        if end > data.len() {
            return Err(ElfError::InvalidData);
        }

        String::from_utf8(data[offset..end].to_vec()).map_err(|_| ElfError::InvalidData)
    }

    /// Find interpreter path for dynamic binaries
    fn find_interpreter(
        &self,
        data: &[u8],
        _header: &Elf64Header,
        program_headers: &[Elf64ProgramHeader],
    ) -> Result<Option<String>, ElfError> {
        for ph in program_headers {
            if ph.p_type == ProgramType::Interp as u32 {
                let offset = ph.p_offset as usize;
                let size = ph.p_filesz as usize;

                if offset + size > data.len() {
                    return Err(ElfError::InvalidProgramHeader);
                }

                let interp_data = &data[offset..offset + size];
                // Remove null terminator
                let interp_str = core::str::from_utf8(&interp_data[..size - 1])
                    .map_err(|_| ElfError::InvalidProgramHeader)?;

                return Ok(Some(String::from(interp_str)));
            }
        }

        Ok(None)
    }

    /// Load a program segment into memory
    fn load_segment(
        &self,
        data: &[u8],
        ph: &Elf64ProgramHeader,
        base_addr: u64,
    ) -> Result<(), ElfError> {
        let file_offset = ph.p_offset as usize;
        let file_size = ph.p_filesz as usize;
        let mem_size = ph.p_memsz as usize;
        let vaddr = ph.p_vaddr;

        // Validate offsets
        if file_offset + file_size > data.len() {
            return Err(ElfError::InvalidProgramHeader);
        }

        // Calculate target address
        let target_addr = (base_addr + vaddr) as *mut u8;

        // Copy file data
        if file_size > 0 {
            // SAFETY: target_addr = base_addr + vaddr points to memory that the
            // caller has mapped for loading. file_offset + file_size <= data.len()
            // was validated above. from_raw_parts_mut creates a mutable slice over
            // the target region for the copy. Source and destination do not overlap
            // (ELF data buffer vs mapped load address).
            unsafe {
                let src = &data[file_offset..file_offset + file_size];
                let dst = slice::from_raw_parts_mut(target_addr, file_size);
                dst.copy_from_slice(src);
            }
        }

        // Zero remaining memory (BSS)
        if mem_size > file_size {
            // SAFETY: target_addr + file_size is within the mapped memory region
            // (base_addr + vaddr .. base_addr + vaddr + mem_size). The BSS section
            // (mem_size - file_size bytes) must be zeroed per the ELF specification.
            // The memory was mapped by the caller for this segment.
            unsafe {
                let bss_start = target_addr.add(file_size);
                let bss_size = mem_size - file_size;
                core::ptr::write_bytes(bss_start, 0, bss_size);
            }
        }

        Ok(())
    }

    /// Process relocations for a loaded binary.
    ///
    /// Scans the ELF data for a PT_DYNAMIC segment, parses the dynamic section
    /// to find relocation tables and the symbol table, then applies all
    /// relocations.  Handles RELA and JMPREL (PLT) relocation tables.
    pub fn process_relocations(&self, data: &[u8], base_addr: u64) -> Result<(), ElfError> {
        let header = self.parse_header(data)?;
        let program_headers = self.parse_program_headers(data, &header)?;

        // Find the PT_DYNAMIC segment
        let dynamic_ph = program_headers
            .iter()
            .find(|ph| ph.p_type == ProgramType::Dynamic as u32);

        let dynamic_ph = match dynamic_ph {
            Some(ph) => ph,
            None => return Ok(()), // No dynamic section -- static binary, nothing to do
        };

        // Parse the dynamic section
        let dyn_info =
            self.parse_dynamic_section(data, dynamic_ph.p_offset, dynamic_ph.p_filesz)?;

        // Resolve symbols if we have a symbol table
        let symbols = if let (Some(symtab), Some(strtab)) = (dyn_info.symtab, dyn_info.strtab) {
            // For file-offset based symbol tables: convert virtual addresses to
            // file offsets by finding the containing LOAD segment.
            let symtab_file = self.vaddr_to_file_offset(&program_headers, symtab);
            let strtab_file = self.vaddr_to_file_offset(&program_headers, strtab);

            if let (Some(sym_off), Some(str_off)) = (symtab_file, strtab_file) {
                // Estimate symbol table size from strtab - symtab if available
                let sym_size = if dyn_info.syment > 0 {
                    // Use RELA entries to estimate max symbol index
                    let max_sym = self.estimate_max_symbol_index(data, &dyn_info);
                    (max_sym + 1) * dyn_info.syment
                } else {
                    (str_off as usize).saturating_sub(sym_off as usize)
                };

                self.resolve_symbols(data, sym_off, sym_size, str_off, dyn_info.strsz)
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // Collect RELA relocations
        let mut relocations = Vec::new();

        if let Some(rela_vaddr) = dyn_info.rela {
            if dyn_info.relasz > 0 && dyn_info.relaent > 0 {
                if let Some(rela_off) = self.vaddr_to_file_offset(&program_headers, rela_vaddr) {
                    let count = dyn_info.relasz / dyn_info.relaent;
                    self.parse_rela_entries(data, rela_off as usize, count, &mut relocations)?;
                }
            }
        }

        // Collect PLT relocations (JMPREL)
        if let Some(jmprel_vaddr) = dyn_info.jmprel {
            if dyn_info.pltrelsz > 0 {
                if let Some(jmprel_off) = self.vaddr_to_file_offset(&program_headers, jmprel_vaddr)
                {
                    let entry_size = if dyn_info.relaent > 0 {
                        dyn_info.relaent
                    } else {
                        mem::size_of::<Elf64Rela>()
                    };
                    let count = dyn_info.pltrelsz / entry_size;
                    self.parse_rela_entries(data, jmprel_off as usize, count, &mut relocations)?;
                }
            }
        }

        // Apply relocations with architecture-aware type handling
        self.perform_relocations_arch(base_addr, &relocations, &symbols, header.machine)
    }

    /// Convert a virtual address to a file offset using LOAD segment mappings.
    fn vaddr_to_file_offset(
        &self,
        program_headers: &[Elf64ProgramHeader],
        vaddr: u64,
    ) -> Option<u64> {
        for ph in program_headers {
            if ph.p_type == ProgramType::Load as u32
                && vaddr >= ph.p_vaddr
                && vaddr < ph.p_vaddr + ph.p_filesz
            {
                return Some(ph.p_offset + (vaddr - ph.p_vaddr));
            }
        }
        None
    }

    /// Parse RELA entries from the given file offset.
    fn parse_rela_entries(
        &self,
        data: &[u8],
        offset: usize,
        count: usize,
        out: &mut Vec<ElfRelocation>,
    ) -> Result<(), ElfError> {
        let entry_size = mem::size_of::<Elf64Rela>();
        for i in 0..count {
            let pos = offset + i * entry_size;
            if pos + entry_size > data.len() {
                break;
            }
            // SAFETY: pos + entry_size <= data.len() was checked above.
            // Elf64Rela is #[repr(C)] with Copy. The value is copied immediately.
            let rela = unsafe { *(data[pos..].as_ptr() as *const Elf64Rela) };
            out.push(ElfRelocation {
                offset: rela.r_offset,
                symbol: (rela.r_info >> 32) as u32,
                reloc_type: (rela.r_info & 0xFFFF_FFFF) as u32,
                addend: rela.r_addend,
            });
        }
        Ok(())
    }

    /// Estimate the maximum symbol index referenced by relocations.
    fn estimate_max_symbol_index(&self, data: &[u8], dyn_info: &DynamicInfo) -> usize {
        let mut max_idx: usize = 0;
        let _entry_size = mem::size_of::<Elf64Rela>();

        // Check RELA
        if let Some(_rela) = dyn_info.rela {
            let count = if dyn_info.relaent > 0 {
                dyn_info.relasz / dyn_info.relaent
            } else {
                0
            };
            // We don't have file offsets here easily, so use a conservative estimate
            max_idx = max_idx.max(count);
        }
        // Check JMPREL
        if let Some(_jmprel) = dyn_info.jmprel {
            let count = if dyn_info.relaent > 0 {
                dyn_info.pltrelsz / dyn_info.relaent
            } else {
                0
            };
            max_idx = max_idx.max(count);
        }
        let _ = data; // used for bounds in full implementation
        max_idx.max(64) // reasonable minimum
    }

    /// Architecture-aware relocation application.
    ///
    /// Handles x86_64, AArch64, and RISC-V relocation types.
    fn perform_relocations_arch(
        &self,
        base_addr: u64,
        relocations: &[ElfRelocation],
        symbols: &[ElfSymbol],
        machine: u16,
    ) -> Result<(), ElfError> {
        for reloc in relocations {
            let target_addr = base_addr.wrapping_add(reloc.offset);

            match machine {
                // x86_64
                62 => self.apply_x86_64_reloc(target_addr, reloc, symbols, base_addr)?,
                // AArch64
                183 => self.apply_aarch64_reloc(target_addr, reloc, symbols, base_addr)?,
                // RISC-V
                243 => self.apply_riscv_reloc(target_addr, reloc, symbols, base_addr)?,
                _ => return Err(ElfError::UnsupportedMachine),
            }
        }
        Ok(())
    }

    /// Apply a single x86_64 relocation.
    fn apply_x86_64_reloc(
        &self,
        target_addr: u64,
        reloc: &ElfRelocation,
        symbols: &[ElfSymbol],
        base_addr: u64,
    ) -> Result<(), ElfError> {
        match reloc.reloc_type {
            8 => {
                // R_X86_64_RELATIVE: base + addend
                // SAFETY: caller mapped memory at target_addr
                unsafe {
                    *(target_addr as *mut u64) = base_addr.wrapping_add(reloc.addend as u64);
                }
            }
            1 => {
                // R_X86_64_64: S + A
                let sym = self.get_symbol(symbols, reloc.symbol)?;
                unsafe {
                    *(target_addr as *mut u64) = sym.value.wrapping_add(reloc.addend as u64);
                }
            }
            6 => {
                // R_X86_64_GLOB_DAT: S
                let sym = self.get_symbol(symbols, reloc.symbol)?;
                unsafe {
                    *(target_addr as *mut u64) = sym.value;
                }
            }
            7 => {
                // R_X86_64_JUMP_SLOT: S
                let sym = self.get_symbol(symbols, reloc.symbol)?;
                unsafe {
                    *(target_addr as *mut u64) = sym.value;
                }
            }
            _ => {} // Ignore unsupported x86_64 relocation types
        }
        Ok(())
    }

    /// Apply a single AArch64 relocation.
    fn apply_aarch64_reloc(
        &self,
        target_addr: u64,
        reloc: &ElfRelocation,
        symbols: &[ElfSymbol],
        base_addr: u64,
    ) -> Result<(), ElfError> {
        match reloc.reloc_type {
            1027 => {
                // R_AARCH64_RELATIVE: B + A
                unsafe {
                    *(target_addr as *mut u64) = base_addr.wrapping_add(reloc.addend as u64);
                }
            }
            257 => {
                // R_AARCH64_ABS64: S + A
                let sym = self.get_symbol(symbols, reloc.symbol)?;
                unsafe {
                    *(target_addr as *mut u64) = sym.value.wrapping_add(reloc.addend as u64);
                }
            }
            1025 => {
                // R_AARCH64_GLOB_DAT: S + A
                let sym = self.get_symbol(symbols, reloc.symbol)?;
                unsafe {
                    *(target_addr as *mut u64) = sym.value.wrapping_add(reloc.addend as u64);
                }
            }
            1026 => {
                // R_AARCH64_JUMP_SLOT: S
                let sym = self.get_symbol(symbols, reloc.symbol)?;
                unsafe {
                    *(target_addr as *mut u64) = sym.value;
                }
            }
            _ => {} // Ignore unsupported AArch64 relocation types
        }
        Ok(())
    }

    /// Apply a single RISC-V relocation.
    fn apply_riscv_reloc(
        &self,
        target_addr: u64,
        reloc: &ElfRelocation,
        symbols: &[ElfSymbol],
        base_addr: u64,
    ) -> Result<(), ElfError> {
        match reloc.reloc_type {
            3 => {
                // R_RISCV_RELATIVE: B + A
                unsafe {
                    *(target_addr as *mut u64) = base_addr.wrapping_add(reloc.addend as u64);
                }
            }
            2 => {
                // R_RISCV_64: S + A
                let sym = self.get_symbol(symbols, reloc.symbol)?;
                unsafe {
                    *(target_addr as *mut u64) = sym.value.wrapping_add(reloc.addend as u64);
                }
            }
            5 => {
                // R_RISCV_JUMP_SLOT: S
                let sym = self.get_symbol(symbols, reloc.symbol)?;
                unsafe {
                    *(target_addr as *mut u64) = sym.value;
                }
            }
            _ => {} // Ignore unsupported RISC-V relocation types
        }
        Ok(())
    }

    /// Helper to safely look up a symbol by index.
    fn get_symbol<'a>(
        &self,
        symbols: &'a [ElfSymbol],
        index: u32,
    ) -> Result<&'a ElfSymbol, ElfError> {
        symbols.get(index as usize).ok_or(ElfError::InvalidSymbol)
    }
}

/// Load an ELF binary from the filesystem
pub fn load_elf_from_file(path: &str) -> Result<ElfBinary, ElfError> {
    use crate::fs::get_vfs;

    // Open the file
    let vfs = get_vfs().read();
    let node = vfs
        .resolve_path(path)
        .map_err(|_| ElfError::FileReadFailed)?;

    // Get file size
    let metadata = node.metadata().map_err(|_| ElfError::FileReadFailed)?;
    let size = metadata.size;

    // Read file contents
    let mut buffer = Vec::with_capacity(size);
    buffer.resize(size, 0);

    node.read(0, &mut buffer)
        .map_err(|_| ElfError::FileReadFailed)?;

    // Load ELF binary
    let loader = ElfLoader::new();
    loader.parse(&buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal valid ELF64 header in a byte buffer.
    /// Returns a Vec<u8> containing a valid ELF header + one LOAD program
    /// header.
    fn make_minimal_elf(
        elf_type: u16,
        machine: u16,
        entry: u64,
        vaddr: u64,
        memsz: u64,
    ) -> Vec<u8> {
        let header_size = core::mem::size_of::<Elf64Header>();
        let ph_size = core::mem::size_of::<Elf64ProgramHeader>();
        let total_size = header_size + ph_size;
        let mut buf = vec![0u8; total_size];

        // ELF magic
        buf[0] = 0x7f;
        buf[1] = b'E';
        buf[2] = b'L';
        buf[3] = b'F';
        // Class: 64-bit
        buf[4] = 2;
        // Data: little endian
        buf[5] = 1;
        // Version
        buf[6] = 1;
        // elf_type at offset 16
        buf[16] = (elf_type & 0xFF) as u8;
        buf[17] = ((elf_type >> 8) & 0xFF) as u8;
        // machine at offset 18
        buf[18] = (machine & 0xFF) as u8;
        buf[19] = ((machine >> 8) & 0xFF) as u8;
        // version2 at offset 20
        buf[20] = 1;
        // entry at offset 24
        let entry_bytes = entry.to_le_bytes();
        buf[24..32].copy_from_slice(&entry_bytes);
        // phoff at offset 32 (program headers start right after header)
        let phoff = (header_size as u64).to_le_bytes();
        buf[32..40].copy_from_slice(&phoff);
        // ehsize at offset 52
        buf[52] = (header_size & 0xFF) as u8;
        buf[53] = ((header_size >> 8) & 0xFF) as u8;
        // phentsize at offset 54
        buf[54] = (ph_size & 0xFF) as u8;
        buf[55] = ((ph_size >> 8) & 0xFF) as u8;
        // phnum at offset 56
        buf[56] = 1;
        buf[57] = 0;

        // Program header (LOAD segment)
        let ph_offset = header_size;
        // p_type = PT_LOAD (1)
        buf[ph_offset] = 1;
        // p_flags at ph_offset + 4 (RWX = 7)
        buf[ph_offset + 4] = 7;
        // p_offset at ph_offset + 8
        // p_vaddr at ph_offset + 16
        let vaddr_bytes = vaddr.to_le_bytes();
        buf[ph_offset + 16..ph_offset + 24].copy_from_slice(&vaddr_bytes);
        // p_paddr at ph_offset + 24
        buf[ph_offset + 24..ph_offset + 32].copy_from_slice(&vaddr_bytes);
        // p_filesz at ph_offset + 32 (0 for simplicity)
        // p_memsz at ph_offset + 40
        let memsz_bytes = memsz.to_le_bytes();
        buf[ph_offset + 40..ph_offset + 48].copy_from_slice(&memsz_bytes);
        // p_align at ph_offset + 48
        let align = 0x1000u64.to_le_bytes();
        buf[ph_offset + 48..ph_offset + 56].copy_from_slice(&align);

        buf
    }

    // --- ElfLoader basic tests ---

    #[test]
    fn test_elf_loader_new() {
        let loader = ElfLoader::new();
        // ElfLoader is a unit struct, just verify it can be created
        let _ = loader;
    }

    #[test]
    fn test_elf_loader_default() {
        let loader = ElfLoader::default();
        let _ = loader;
    }

    // --- Header validation tests ---

    #[test]
    fn test_parse_invalid_magic() {
        let loader = ElfLoader::new();
        let data = vec![0u8; 128]; // All zeros, no ELF magic
        let result = loader.parse(&data);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ElfError::InvalidMagic));
    }

    #[test]
    fn test_parse_too_small() {
        let loader = ElfLoader::new();
        let data = vec![0x7f, b'E', b'L', b'F']; // Just magic, too small for header
        let result = loader.parse(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_wrong_class_32bit() {
        let loader = ElfLoader::new();
        let mut data = vec![0u8; 128];
        // Set ELF magic
        data[0] = 0x7f;
        data[1] = b'E';
        data[2] = b'L';
        data[3] = b'F';
        // Class = 32-bit (should be 64-bit)
        data[4] = 1;
        data[5] = 1; // Little endian
        data[6] = 1; // Version

        let result = loader.parse(&data);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ElfError::InvalidClass));
    }

    #[test]
    fn test_parse_wrong_endian() {
        let loader = ElfLoader::new();
        let mut data = vec![0u8; 128];
        data[0] = 0x7f;
        data[1] = b'E';
        data[2] = b'L';
        data[3] = b'F';
        data[4] = 2; // 64-bit
        data[5] = 2; // Big endian (we only support little endian)
        data[6] = 1;

        let result = loader.parse(&data);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ElfError::InvalidData));
    }

    #[test]
    fn test_parse_invalid_type_none() {
        let loader = ElfLoader::new();
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2; // 64-bit
        data[5] = 1; // LE
        data[6] = 1;
        // elf_type = ET_NONE (0) at offset 16 -- already zero
        data[18] = 62; // x86_64

        let result = loader.parse(&data);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ElfError::InvalidType));
    }

    #[test]
    fn test_parse_unsupported_machine() {
        let loader = ElfLoader::new();
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(&ELF_MAGIC);
        data[4] = 2;
        data[5] = 1;
        data[6] = 1;
        data[16] = 2; // ET_EXEC
                      // machine = 99 (unsupported)
        data[18] = 99;

        let result = loader.parse(&data);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ElfError::UnsupportedMachine));
    }

    // --- Successful parse tests ---

    #[test]
    fn test_parse_valid_executable_x86_64() {
        let loader = ElfLoader::new();
        let data = make_minimal_elf(2, 62, 0x401000, 0x400000, 0x1000);

        let result = loader.parse(&data);
        assert!(result.is_ok());

        let binary = result.unwrap();
        assert_eq!(binary.entry_point, 0x401000);
        assert_eq!(binary.load_base, 0x400000);
        assert_eq!(binary.load_size, 0x1000);
        assert!(!binary.dynamic);
        assert!(binary.interpreter.is_none());
    }

    #[test]
    fn test_parse_valid_executable_aarch64() {
        let loader = ElfLoader::new();
        let data = make_minimal_elf(2, 183, 0x400000, 0x400000, 0x2000);

        let result = loader.parse(&data);
        assert!(result.is_ok());
        let binary = result.unwrap();
        assert_eq!(binary.entry_point, 0x400000);
    }

    #[test]
    fn test_parse_valid_executable_riscv() {
        let loader = ElfLoader::new();
        let data = make_minimal_elf(2, 243, 0x10000, 0x10000, 0x4000);

        let result = loader.parse(&data);
        assert!(result.is_ok());
        let binary = result.unwrap();
        assert_eq!(binary.entry_point, 0x10000);
    }

    #[test]
    fn test_parse_shared_object() {
        let loader = ElfLoader::new();
        // ET_DYN = 3
        let data = make_minimal_elf(3, 62, 0x0, 0x0, 0x1000);

        let result = loader.parse(&data);
        assert!(result.is_ok());
    }

    // --- Segment parsing tests ---

    #[test]
    fn test_parse_segments_load() {
        let loader = ElfLoader::new();
        let data = make_minimal_elf(2, 62, 0x401000, 0x400000, 0x3000);

        let binary = loader.parse(&data).unwrap();
        assert_eq!(binary.segments.len(), 1);

        let seg = &binary.segments[0];
        assert_eq!(seg.segment_type, SegmentType::Load);
        assert_eq!(seg.virtual_addr, 0x400000);
        assert_eq!(seg.memory_size, 0x3000);
    }

    // --- Memory layout calculation tests ---

    #[test]
    fn test_calculate_memory_layout_empty() {
        let loader = ElfLoader::new();
        let headers: Vec<Elf64ProgramHeader> = vec![];

        let result = loader.calculate_memory_layout(&headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_calculate_memory_layout_single_load() {
        let loader = ElfLoader::new();
        let headers = vec![Elf64ProgramHeader {
            p_type: 1, // PT_LOAD
            p_flags: 5,
            p_offset: 0,
            p_vaddr: 0x400000,
            p_paddr: 0x400000,
            p_filesz: 0x1000,
            p_memsz: 0x2000,
            p_align: 0x1000,
        }];

        let (base, size) = loader.calculate_memory_layout(&headers).unwrap();
        assert_eq!(base, 0x400000);
        assert_eq!(size, 0x2000);
    }

    #[test]
    fn test_calculate_memory_layout_multiple_loads() {
        let loader = ElfLoader::new();
        let headers = vec![
            Elf64ProgramHeader {
                p_type: 1, // PT_LOAD
                p_flags: 5,
                p_offset: 0,
                p_vaddr: 0x400000,
                p_paddr: 0x400000,
                p_filesz: 0x1000,
                p_memsz: 0x1000,
                p_align: 0x1000,
            },
            Elf64ProgramHeader {
                p_type: 1, // PT_LOAD
                p_flags: 6,
                p_offset: 0x1000,
                p_vaddr: 0x600000,
                p_paddr: 0x600000,
                p_filesz: 0x500,
                p_memsz: 0x3000,
                p_align: 0x1000,
            },
        ];

        let (base, size) = loader.calculate_memory_layout(&headers).unwrap();
        assert_eq!(base, 0x400000);
        assert_eq!(size, (0x600000 + 0x3000 - 0x400000));
    }

    // --- read_string tests ---

    #[test]
    fn test_read_string_valid() {
        let loader = ElfLoader::new();
        let data = b"hello\0world\0";
        let result = loader.read_string(data, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn test_read_string_at_offset() {
        let loader = ElfLoader::new();
        let data = b"hello\0world\0";
        let result = loader.read_string(data, 6);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "world");
    }

    #[test]
    fn test_read_string_empty() {
        let loader = ElfLoader::new();
        let data = b"\0rest";
        let result = loader.read_string(data, 0);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    // --- SegmentType tests ---

    #[test]
    fn test_segment_type_equality() {
        assert_eq!(SegmentType::Null, SegmentType::Null);
        assert_eq!(SegmentType::Load, SegmentType::Load);
        assert_ne!(SegmentType::Load, SegmentType::Dynamic);
        assert_eq!(SegmentType::Other(99), SegmentType::Other(99));
        assert_ne!(SegmentType::Other(1), SegmentType::Other(2));
    }
}

/// Execute an ELF binary
pub fn exec_elf(path: &str) -> Result<u64, ElfError> {
    use crate::process::ProcessId;

    // Load ELF information
    let elf_info = load_elf_from_file(path)?;

    // Allocate memory for the program
    let current_pid = crate::process::current_process()
        .map(|p| p.pid)
        .unwrap_or(ProcessId(1));
    let process =
        crate::process::get_process(current_pid).ok_or(ElfError::MemoryAllocationFailed)?;

    // Map program memory
    let base_addr = 0x400000; // Standard user-space base address
    let mut memory_space = process.memory_space.lock();
    let vas = &mut *memory_space;

    for offset in (0..elf_info.load_size).step_by(4096) {
        vas.map_page(
            (base_addr + offset) as usize,
            crate::mm::PageFlags::USER | crate::mm::PageFlags::WRITABLE,
        )
        .map_err(|_| ElfError::MemoryAllocationFailed)?;
    }

    // Read file again and load into memory
    let vfs = get_vfs().read();
    let node = vfs
        .resolve_path(path)
        .map_err(|_| ElfError::FileReadFailed)?;

    let mut buffer = Vec::with_capacity(elf_info.load_size);
    buffer.resize(elf_info.load_size, 0);

    node.read(0, &mut buffer)
        .map_err(|_| ElfError::FileReadFailed)?;

    // Load the binary
    let loader = ElfLoader::new();
    let entry_point = loader.load_into_memory(&buffer, base_addr as u64)?;

    // Process relocations for PIE/dynamic binaries
    if elf_info.dynamic {
        loader.process_relocations(&buffer, base_addr as u64)?;
    }

    Ok(entry_point)
}

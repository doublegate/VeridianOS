//! ELF64 type definitions
//!
//! Contains all ELF64 struct, enum, and error type definitions used by the
//! loader. Separated from `mod.rs` for maintainability.

use alloc::{string::String, vec::Vec};

/// ELF magic number
pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

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
#[derive(Debug)]
pub struct ElfBinary {
    pub entry_point: u64,
    pub load_base: u64,
    pub load_size: usize,
    /// Offset of the program header table within the ELF file (e_phoff).
    /// AT_PHDR is derived from it by [`ElfBinary::phdr_vaddr`].
    pub phoff: u64,
    /// Number of program header entries (e_phnum).
    pub phnum: u16,
    /// Size of each program header entry in bytes (e_phentsize, typically 56).
    pub phentsize: u16,
    pub segments: Vec<ElfSegment>,
    pub interpreter: Option<String>,
    pub dynamic: bool,
}

impl ElfBinary {
    /// Virtual address of the program header table once loaded (AT_PHDR):
    /// PT_PHDR's `p_vaddr` if present, otherwise the address of `e_phoff`
    /// inside the PT_LOAD segment whose file range contains it. `None` if
    /// the table is not part of any loaded segment.
    ///
    /// `load_base + e_phoff` (and plain `load_base`) assumed the lowest
    /// PT_LOAD maps file offset 0, which a linker need not do (review of
    /// the v0.26.0 stack, PR #14). Addresses are as linked: the loader
    /// applies no load bias.
    pub fn phdr_vaddr(&self) -> Option<u64> {
        if let Some(p) = self
            .segments
            .iter()
            .find(|s| s.segment_type == SegmentType::Phdr)
        {
            return Some(p.virtual_addr);
        }
        self.segments
            .iter()
            .find(|s| {
                s.segment_type == SegmentType::Load
                    && self.phoff >= s.file_offset
                    && self.phoff - s.file_offset < s.file_size
            })
            .and_then(|s| s.virtual_addr.checked_add(self.phoff - s.file_offset))
    }

    /// AT_PHDR for the auxiliary vector: [`Self::phdr_vaddr`], or, for a
    /// table outside every segment, `load_base + e_phoff`. `None` if that
    /// sum overflows: the header values are untrusted input, so an
    /// out-of-range address is an error, not a wrapped or panicking
    /// computation (review of the v0.26.0 stack, PR #15).
    pub fn phdr_address(&self) -> Option<u64> {
        self.phdr_vaddr()
            .or_else(|| self.load_base.checked_add(self.phoff))
    }
}

/// Layout of the initial x86_64 (TLS variant II) TLS block for a PT_TLS
/// segment of `memsz` bytes and alignment `p_align`: returns
/// `(block_size, tcb_offset)`, where the TLS image starts at the block base
/// and the TCB (the thread pointer, FS_BASE) sits at `tcb_offset`.
///
/// The thread pointer must be `p_align`-aligned and the image ends exactly
/// at it, so the image size is rounded up to the alignment (at least 8,
/// for the TCB self-pointer). Placing the TCB at `base + memsz` misaligned
/// TLS whenever `memsz` was not a multiple of `p_align` (review of the
/// v0.26.0 stack, PR #14). The block base is page-aligned, so an alignment
/// that is not a power of two or exceeds a page is refused (`None`).
pub fn tls_layout(memsz: usize, p_align: u64) -> Option<(usize, usize)> {
    const TCB_SIZE: usize = 8; // the TCB self-pointer
    let align = (p_align as usize).max(8);
    if !align.is_power_of_two() || align > 4096 {
        return None;
    }
    let aligned = memsz.checked_next_multiple_of(align)?;
    let block_size = aligned
        .checked_add(TCB_SIZE)?
        .checked_next_multiple_of(16)?;
    Some((block_size, aligned))
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

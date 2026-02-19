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

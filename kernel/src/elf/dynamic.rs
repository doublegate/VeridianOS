//! ELF Dynamic Linker Bootstrap Support
//!
//! Provides auxiliary vector construction, interpreter discovery, and dynamic
//! linker preparation for running dynamically-linked ELF binaries.

// ELF dynamic linker bootstrap

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};

use super::types::{Elf64ProgramHeader, ElfBinary, ProgramType};
use crate::{error::KernelError, mm::PAGE_SIZE};

// ---------------------------------------------------------------------------
// Auxiliary Vector Types
// ---------------------------------------------------------------------------

/// Auxiliary vector entry type identifiers (from the ELF specification / Linux
/// ABI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum AuxType {
    /// End of vector (sentinel).
    AtNull = 0,
    /// Program headers address in memory.
    AtPhdr = 3,
    /// Size of one program header entry.
    AtPhent = 4,
    /// Number of program headers.
    AtPhnum = 5,
    /// System page size.
    AtPagesz = 6,
    /// Interpreter base address.
    AtBase = 7,
    /// Entry point of the binary.
    AtEntry = 9,
    /// Address of 16 random bytes (for stack canaries / ASLR).
    AtRandom = 25,
    /// Filename of the program being run.
    AtExecfn = 31,
}

/// A single entry in the auxiliary vector passed to the dynamic linker.
#[derive(Debug, Clone, Copy)]
pub struct AuxVecEntry {
    /// The type of this entry.
    pub type_id: AuxType,
    /// The value associated with this type.
    pub value: u64,
}

impl AuxVecEntry {
    /// Create a new auxiliary vector entry.
    pub fn new(type_id: AuxType, value: u64) -> Self {
        Self { type_id, value }
    }
}

// ---------------------------------------------------------------------------
// Dynamic Linker Info
// ---------------------------------------------------------------------------

/// Information required to set up the dynamic linker for a binary.
#[cfg(feature = "alloc")]
pub struct DynamicLinkerInfo {
    /// Path to the interpreter (e.g., `/lib/ld-linux.so.2`).
    pub interp_path: String,
    /// Base address at which the interpreter was loaded.
    pub interp_base: u64,
    /// Entry point of the interpreter.
    pub interp_entry: u64,
    /// Auxiliary vector to place on the initial user stack.
    pub aux_vector: Vec<AuxVecEntry>,
}

// ---------------------------------------------------------------------------
// Interpreter Discovery
// ---------------------------------------------------------------------------

/// Find the interpreter path from a list of program headers.
///
/// Scans for a `PT_INTERP` segment and extracts the null-terminated path
/// string from the ELF data. Returns `None` for statically linked binaries.
#[cfg(feature = "alloc")]
pub fn find_interpreter(
    data: &[u8],
    program_headers: &[Elf64ProgramHeader],
) -> Result<Option<String>, KernelError> {
    for ph in program_headers {
        if ph.p_type == ProgramType::Interp as u32 {
            let offset = ph.p_offset as usize;
            let size = ph.p_filesz as usize;

            if size == 0 {
                return Err(KernelError::InvalidArgument {
                    name: "PT_INTERP",
                    value: "empty interpreter path",
                });
            }

            if offset.checked_add(size).is_none() || offset + size > data.len() {
                return Err(KernelError::InvalidArgument {
                    name: "PT_INTERP",
                    value: "segment extends past end of file",
                });
            }

            let interp_bytes = &data[offset..offset + size];

            // Remove trailing null terminator if present.
            let path_bytes = if interp_bytes.last() == Some(&0) {
                &interp_bytes[..size - 1]
            } else {
                interp_bytes
            };

            let path =
                core::str::from_utf8(path_bytes).map_err(|_| KernelError::InvalidArgument {
                    name: "PT_INTERP",
                    value: "interpreter path is not valid UTF-8",
                })?;

            return Ok(Some(String::from(path)));
        }
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Auxiliary Vector Construction
// ---------------------------------------------------------------------------

/// Build an auxiliary vector for the dynamic linker.
///
/// The auxiliary vector is placed on the user stack before the process starts.
/// It provides the dynamic linker with information about the loaded binary.
///
/// # Arguments
/// * `elf_binary` - Parsed ELF binary information.
/// * `phdr_addr`  - Address where program headers are loaded in memory.
/// * `interp_base` - Base address of the loaded interpreter (0 if none).
/// * `random_addr` - Address of 16 random bytes for AT_RANDOM.
/// * `execfn_addr` - Address of the filename string.
#[cfg(feature = "alloc")]
pub fn build_aux_vector(
    elf_binary: &ElfBinary,
    phdr_addr: u64,
    interp_base: u64,
    random_addr: u64,
    execfn_addr: u64,
) -> Vec<AuxVecEntry> {
    let phdr_count = elf_binary
        .segments
        .iter()
        .filter(|s| s.segment_type == super::types::SegmentType::Phdr)
        .count();

    // If no PHDR segment, count all segments as accessible via phdr_addr.
    let phnum = if phdr_count > 0 {
        phdr_count as u64
    } else {
        elf_binary.segments.len() as u64
    };

    let phent_size = core::mem::size_of::<Elf64ProgramHeader>() as u64;

    let mut aux = Vec::with_capacity(10);

    aux.push(AuxVecEntry::new(AuxType::AtPhdr, phdr_addr));
    aux.push(AuxVecEntry::new(AuxType::AtPhent, phent_size));
    aux.push(AuxVecEntry::new(AuxType::AtPhnum, phnum));
    aux.push(AuxVecEntry::new(AuxType::AtPagesz, PAGE_SIZE as u64));
    aux.push(AuxVecEntry::new(AuxType::AtEntry, elf_binary.entry_point));

    if interp_base != 0 {
        aux.push(AuxVecEntry::new(AuxType::AtBase, interp_base));
    }

    if random_addr != 0 {
        aux.push(AuxVecEntry::new(AuxType::AtRandom, random_addr));
    }

    if execfn_addr != 0 {
        aux.push(AuxVecEntry::new(AuxType::AtExecfn, execfn_addr));
    }

    // Terminate the vector.
    aux.push(AuxVecEntry::new(AuxType::AtNull, 0));

    aux
}

// ---------------------------------------------------------------------------
// Dynamic Linking Preparation
// ---------------------------------------------------------------------------

/// Prepare everything needed to run a dynamically linked ELF binary.
///
/// This function:
/// 1. Checks whether the binary has an interpreter.
/// 2. Loads the interpreter (if present) using the ELF loader.
/// 3. Builds the auxiliary vector for the dynamic linker.
///
/// Returns a [`DynamicLinkerInfo`] with all information needed to start the
/// process. If the binary is statically linked, returns `None`.
///
/// # Arguments
/// * `_elf_data`   - Raw bytes of the ELF file (reserved for future use).
/// * `elf_binary`  - Pre-parsed ELF binary metadata.
/// * `load_base`   - Base address where the main binary was loaded.
#[cfg(feature = "alloc")]
pub fn prepare_dynamic_linking(
    _elf_data: &[u8],
    elf_binary: &ElfBinary,
    load_base: u64,
) -> Result<Option<DynamicLinkerInfo>, KernelError> {
    // Check whether the binary has an interpreter.
    let interp_path = match &elf_binary.interpreter {
        Some(path) => path.clone(),
        None => return Ok(None), // Statically linked.
    };

    // Load the interpreter ELF binary from the filesystem.
    let interp_elf =
        super::load_elf_from_file(&interp_path).map_err(|_| KernelError::NotFound {
            resource: "interpreter",
            id: 0,
        })?;

    let interp_base = interp_elf.load_base;
    let interp_entry = interp_elf.entry_point;

    // The program headers are located at load_base + phoff. For a standard
    // ELF the PHDR segment points to them. Use load_base as the phdr address
    // for now; the kernel stack setup will place them correctly.
    let phdr_addr = load_base;

    // Build auxiliary vector. AT_RANDOM and AT_EXECFN addresses will be set
    // up later when the user stack is constructed.
    let aux_vector = build_aux_vector(elf_binary, phdr_addr, interp_base, 0, 0);

    Ok(Some(DynamicLinkerInfo {
        interp_path,
        interp_base,
        interp_entry,
        aux_vector,
    }))
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn test_aux_type_values() {
        assert_eq!(AuxType::AtNull as u64, 0);
        assert_eq!(AuxType::AtPhdr as u64, 3);
        assert_eq!(AuxType::AtPhent as u64, 4);
        assert_eq!(AuxType::AtPhnum as u64, 5);
        assert_eq!(AuxType::AtPagesz as u64, 6);
        assert_eq!(AuxType::AtBase as u64, 7);
        assert_eq!(AuxType::AtEntry as u64, 9);
        assert_eq!(AuxType::AtRandom as u64, 25);
        assert_eq!(AuxType::AtExecfn as u64, 31);
    }

    #[test]
    fn test_aux_vec_entry_new() {
        let entry = AuxVecEntry::new(AuxType::AtPagesz, 4096);
        assert_eq!(entry.type_id, AuxType::AtPagesz);
        assert_eq!(entry.value, 4096);
    }

    #[test]
    fn test_find_interpreter_no_interp() {
        // Empty program headers -- no interpreter.
        let headers: Vec<Elf64ProgramHeader> = Vec::new();
        let data = &[];
        let result = find_interpreter(data, &headers);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_find_interpreter_with_interp() {
        let interp_path = b"/lib/ld-linux.so.2\0";
        let mut data = vec![0u8; 128];
        let offset = 64usize;
        data[offset..offset + interp_path.len()].copy_from_slice(interp_path);

        let headers = vec![Elf64ProgramHeader {
            p_type: ProgramType::Interp as u32,
            p_flags: 4, // PF_R
            p_offset: offset as u64,
            p_vaddr: 0,
            p_paddr: 0,
            p_filesz: interp_path.len() as u64,
            p_memsz: interp_path.len() as u64,
            p_align: 1,
        }];

        let result = find_interpreter(&data, &headers);
        assert!(result.is_ok());
        let interp = result.unwrap();
        assert!(interp.is_some());
        assert_eq!(interp.unwrap(), "/lib/ld-linux.so.2");
    }

    #[test]
    fn test_find_interpreter_invalid_offset() {
        let headers = vec![Elf64ProgramHeader {
            p_type: ProgramType::Interp as u32,
            p_flags: 4,
            p_offset: 1000, // Past end of data.
            p_vaddr: 0,
            p_paddr: 0,
            p_filesz: 20,
            p_memsz: 20,
            p_align: 1,
        }];

        let data = &[0u8; 10];
        let result = find_interpreter(data, &headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_aux_vector_terminates_with_null() {
        use alloc::vec;

        let binary = ElfBinary {
            entry_point: 0x401000,
            load_base: 0x400000,
            load_size: 0x2000,
            segments: vec![],
            interpreter: None,
            dynamic: false,
        };

        let aux = build_aux_vector(&binary, 0x400040, 0, 0, 0);

        // Must end with AT_NULL.
        let last = aux.last().expect("aux vector should not be empty");
        assert_eq!(last.type_id, AuxType::AtNull);
        assert_eq!(last.value, 0);
    }

    #[test]
    fn test_build_aux_vector_contains_essential_entries() {
        use alloc::vec;

        let binary = ElfBinary {
            entry_point: 0x401000,
            load_base: 0x400000,
            load_size: 0x2000,
            segments: vec![],
            interpreter: Some(String::from("/lib/ld.so.1")),
            dynamic: true,
        };

        let aux = build_aux_vector(&binary, 0x400040, 0x7F000000, 0xBEEF, 0xCAFE);

        let has = |t: AuxType| aux.iter().any(|e| e.type_id == t);
        assert!(has(AuxType::AtPhdr));
        assert!(has(AuxType::AtPhent));
        assert!(has(AuxType::AtPhnum));
        assert!(has(AuxType::AtPagesz));
        assert!(has(AuxType::AtEntry));
        assert!(has(AuxType::AtBase));
        assert!(has(AuxType::AtRandom));
        assert!(has(AuxType::AtExecfn));
        assert!(has(AuxType::AtNull));
    }
}

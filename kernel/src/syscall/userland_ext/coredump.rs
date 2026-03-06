//! Core Dump - ELF Core File Generation
//!
//! Generates ELF core dump files containing process state (registers,
//! memory mappings) for post-mortem debugging.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::ptrace::RegisterState;

// ============================================================================
// ELF Constants
// ============================================================================

/// ELF class (64-bit)
const ELFCLASS64: u8 = 2;
/// ELF data (little-endian)
const ELFDATA2LSB: u8 = 1;
/// ELF version (current)
const EV_CURRENT: u8 = 1;
/// ELF OS/ABI (System V)
const ELFOSABI_NONE: u8 = 0;
/// ELF type: core dump
const ET_CORE: u16 = 4;
/// ELF machine: x86_64
const EM_X86_64: u16 = 62;

/// Program header type: note segment
const PT_NOTE: u32 = 4;
/// Program header type: loadable segment
const PT_LOAD: u32 = 1;

/// Note type: prstatus (register state)
const NT_PRSTATUS: u32 = 1;
/// Note type: prpsinfo (process info)
const NT_PRPSINFO: u32 = 3;
/// Note type: auxv (auxiliary vector)
const NT_AUXV: u32 = 6;
/// Note type: file mappings
const NT_FILE: u32 = 0x46494C45;

/// Note name for core dumps
const CORE_NOTE_NAME: &[u8] = b"CORE\0\0\0\0"; // padded to 8 bytes

/// ELF file header size (64-bit)
const ELF64_EHDR_SIZE: usize = 64;
/// ELF program header size (64-bit)
const ELF64_PHDR_SIZE: usize = 56;

// ============================================================================
// Types
// ============================================================================

/// Core dump error types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreDumpError {
    /// Process not found
    ProcessNotFound,
    /// No memory mappings available
    NoMappings,
    /// Buffer allocation failure
    OutOfMemory,
    /// Permission denied (cannot dump another process)
    PermissionDenied,
    /// Core dumps disabled for this process
    Disabled,
    /// Internal error during generation
    InternalError,
}

/// Memory segment descriptor for a core dump
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoreMemorySegment {
    /// Virtual address start
    pub vaddr: u64,
    /// Size in bytes
    pub size: u64,
    /// Flags (PF_R=4, PF_W=2, PF_X=1)
    pub flags: u32,
    /// Offset in the core file where data is stored
    pub file_offset: u64,
}

/// Process status information for NT_PRSTATUS note
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrStatus {
    /// Signal that caused the dump
    pub signal: i32,
    /// Process ID
    pub pid: u64,
    /// Parent process ID
    pub ppid: u64,
    /// Process group ID
    pub pgrp: u64,
    /// Session ID
    pub sid: u64,
    /// User time (microseconds, integer)
    pub user_time_us: u64,
    /// System time (microseconds, integer)
    pub sys_time_us: u64,
    /// Register state at time of dump
    pub registers: RegisterState,
}

/// Process info for NT_PRPSINFO note
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrPsInfo {
    /// Process state character ('R', 'S', 'D', 'T', 'Z', etc.)
    pub state: u8,
    /// Filename of the executable (up to 16 chars)
    pub fname: [u8; 16],
    /// Command line arguments (up to 80 chars)
    pub psargs: [u8; 80],
    /// Process ID
    pub pid: u64,
    /// Parent PID
    pub ppid: u64,
    /// Process group ID
    pub pgrp: u64,
    /// Session ID
    pub sid: u64,
    /// User ID
    pub uid: u32,
    /// Group ID
    pub gid: u32,
}

impl Default for PrPsInfo {
    fn default() -> Self {
        Self {
            state: b'R',
            fname: [0u8; 16],
            psargs: [0u8; 80],
            pid: 0,
            ppid: 0,
            pgrp: 0,
            sid: 0,
            uid: 0,
            gid: 0,
        }
    }
}

impl PrPsInfo {
    /// Set the filename from a string
    pub fn set_fname(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = core::cmp::min(bytes.len(), 15);
        self.fname[..len].copy_from_slice(&bytes[..len]);
        self.fname[len] = 0;
    }

    /// Set the command arguments from a string
    pub fn set_psargs(&mut self, args: &str) {
        let bytes = args.as_bytes();
        let len = core::cmp::min(bytes.len(), 79);
        self.psargs[..len].copy_from_slice(&bytes[..len]);
        self.psargs[len] = 0;
    }
}

// ============================================================================
// CoreDumpWriter
// ============================================================================

/// Core dump writer
#[derive(Debug)]
pub struct CoreDumpWriter {
    /// Process status
    pub prstatus: PrStatus,
    /// Process info
    pub prpsinfo: PrPsInfo,
    /// Memory segments
    pub segments: Vec<CoreMemorySegment>,
    /// Memory content for each segment (indexed by segment index)
    pub segment_data: Vec<Vec<u8>>,
}

impl Default for CoreDumpWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreDumpWriter {
    /// Create a new core dump writer
    pub fn new() -> Self {
        Self {
            prstatus: PrStatus::default(),
            prpsinfo: PrPsInfo::default(),
            segments: Vec::new(),
            segment_data: Vec::new(),
        }
    }

    /// Add a memory segment to the core dump
    pub fn add_segment(&mut self, vaddr: u64, flags: u32, data: Vec<u8>) {
        let seg = CoreMemorySegment {
            vaddr,
            size: data.len() as u64,
            flags,
            file_offset: 0, // computed during write
        };
        self.segments.push(seg);
        self.segment_data.push(data);
    }

    /// Write a u16 in little-endian to a buffer
    fn write_u16(buf: &mut Vec<u8>, val: u16) {
        buf.extend_from_slice(&val.to_le_bytes());
    }

    /// Write a u32 in little-endian to a buffer
    fn write_u32(buf: &mut Vec<u8>, val: u32) {
        buf.extend_from_slice(&val.to_le_bytes());
    }

    /// Write a u64 in little-endian to a buffer
    fn write_u64(buf: &mut Vec<u8>, val: u64) {
        buf.extend_from_slice(&val.to_le_bytes());
    }

    /// Write an i32 in little-endian to a buffer
    fn write_i32(buf: &mut Vec<u8>, val: i32) {
        buf.extend_from_slice(&val.to_le_bytes());
    }

    /// Build the ELF header
    fn build_elf_header(&self, phnum: u16, buf: &mut Vec<u8>) {
        // e_ident: magic, class, data, version, OS/ABI, padding
        buf.extend_from_slice(&[0x7F, b'E', b'L', b'F']); // magic
        buf.push(ELFCLASS64); // 64-bit
        buf.push(ELFDATA2LSB); // little-endian
        buf.push(EV_CURRENT); // ELF version
        buf.push(ELFOSABI_NONE); // OS/ABI
        buf.extend_from_slice(&[0u8; 8]); // padding
        Self::write_u16(buf, ET_CORE); // e_type
        Self::write_u16(buf, EM_X86_64); // e_machine
        Self::write_u32(buf, 1); // e_version
        Self::write_u64(buf, 0); // e_entry
        Self::write_u64(buf, ELF64_EHDR_SIZE as u64); // e_phoff (immediately after header)
        Self::write_u64(buf, 0); // e_shoff (no section headers)
        Self::write_u32(buf, 0); // e_flags
        Self::write_u16(buf, ELF64_EHDR_SIZE as u16); // e_ehsize
        Self::write_u16(buf, ELF64_PHDR_SIZE as u16); // e_phentsize
        Self::write_u16(buf, phnum); // e_phnum
        Self::write_u16(buf, 0); // e_shentsize
        Self::write_u16(buf, 0); // e_shnum
        Self::write_u16(buf, 0); // e_shstrndx
    }

    /// Build a program header
    #[allow(clippy::too_many_arguments)]
    fn build_phdr(
        buf: &mut Vec<u8>,
        p_type: u32,
        p_flags: u32,
        p_offset: u64,
        p_vaddr: u64,
        p_paddr: u64,
        p_filesz: u64,
        p_memsz: u64,
        p_align: u64,
    ) {
        Self::write_u32(buf, p_type);
        Self::write_u32(buf, p_flags);
        Self::write_u64(buf, p_offset);
        Self::write_u64(buf, p_vaddr);
        Self::write_u64(buf, p_paddr);
        Self::write_u64(buf, p_filesz);
        Self::write_u64(buf, p_memsz);
        Self::write_u64(buf, p_align);
    }

    /// Build a note entry
    fn build_note(buf: &mut Vec<u8>, name: &[u8], note_type: u32, desc: &[u8]) {
        let namesz = name.len() as u32;
        let descsz = desc.len() as u32;
        Self::write_u32(buf, namesz);
        Self::write_u32(buf, descsz);
        Self::write_u32(buf, note_type);
        // Name (padded to 4-byte boundary)
        buf.extend_from_slice(name);
        let name_pad = (4 - (namesz as usize % 4)) % 4;
        for _ in 0..name_pad {
            buf.push(0);
        }
        // Descriptor (padded to 4-byte boundary)
        buf.extend_from_slice(desc);
        let desc_pad = (4 - (descsz as usize % 4)) % 4;
        for _ in 0..desc_pad {
            buf.push(0);
        }
    }

    /// Build NT_PRSTATUS note descriptor
    fn build_prstatus_desc(&self) -> Vec<u8> {
        let mut desc = Vec::with_capacity(336);
        // si_signo, si_code, si_errno
        Self::write_i32(&mut desc, self.prstatus.signal);
        Self::write_i32(&mut desc, 0); // code
        Self::write_i32(&mut desc, 0); // errno
                                       // cursig, sigpend, sighold
        Self::write_u16(&mut desc, self.prstatus.signal as u16);
        desc.extend_from_slice(&[0u8; 6]); // padding
        Self::write_u64(&mut desc, 0); // sigpend
        Self::write_u64(&mut desc, 0); // sighold
                                       // pid, ppid, pgrp, sid
        Self::write_u32(&mut desc, self.prstatus.pid as u32);
        Self::write_u32(&mut desc, self.prstatus.ppid as u32);
        Self::write_u32(&mut desc, self.prstatus.pgrp as u32);
        Self::write_u32(&mut desc, self.prstatus.sid as u32);
        // user time, system time (timeval: sec + usec)
        Self::write_u64(&mut desc, self.prstatus.user_time_us / 1_000_000);
        Self::write_u64(&mut desc, self.prstatus.user_time_us % 1_000_000);
        Self::write_u64(&mut desc, self.prstatus.sys_time_us / 1_000_000);
        Self::write_u64(&mut desc, self.prstatus.sys_time_us % 1_000_000);
        // Registers (all 27 u64 fields of RegisterState)
        let regs = &self.prstatus.registers;
        for &val in &[
            regs.r15,
            regs.r14,
            regs.r13,
            regs.r12,
            regs.rbp,
            regs.rbx,
            regs.r11,
            regs.r10,
            regs.r9,
            regs.r8,
            regs.rax,
            regs.rcx,
            regs.rdx,
            regs.rsi,
            regs.rdi,
            regs.orig_rax,
            regs.rip,
            regs.cs,
            regs.rflags,
            regs.rsp,
            regs.ss,
            regs.fs_base,
            regs.gs_base,
            regs.ds,
            regs.es,
            regs.fs,
            regs.gs,
        ] {
            Self::write_u64(&mut desc, val);
        }
        desc
    }

    /// Build NT_PRPSINFO note descriptor
    fn build_prpsinfo_desc(&self) -> Vec<u8> {
        let mut desc = Vec::with_capacity(136);
        desc.push(self.prpsinfo.state); // pr_state
        desc.extend_from_slice(&self.prpsinfo.fname); // pr_fname
        desc.extend_from_slice(&[0u8; 3]); // padding
        desc.extend_from_slice(&self.prpsinfo.psargs); // pr_psargs
        Self::write_u32(&mut desc, self.prpsinfo.pid as u32);
        Self::write_u32(&mut desc, self.prpsinfo.ppid as u32);
        Self::write_u32(&mut desc, self.prpsinfo.pgrp as u32);
        Self::write_u32(&mut desc, self.prpsinfo.sid as u32);
        Self::write_u32(&mut desc, self.prpsinfo.uid);
        Self::write_u32(&mut desc, self.prpsinfo.gid);
        desc
    }

    /// Generate the complete core dump as a byte vector
    pub fn write_core_dump(&mut self) -> Result<Vec<u8>, CoreDumpError> {
        // Number of program headers: 1 for PT_NOTE + 1 per segment
        let num_segments = self.segments.len();
        let phnum = (1 + num_segments) as u16;

        // Build the notes section
        let mut notes = Vec::new();
        let prstatus_desc = self.build_prstatus_desc();
        Self::build_note(&mut notes, CORE_NOTE_NAME, NT_PRSTATUS, &prstatus_desc);
        let prpsinfo_desc = self.build_prpsinfo_desc();
        Self::build_note(&mut notes, CORE_NOTE_NAME, NT_PRPSINFO, &prpsinfo_desc);

        // Calculate offsets
        let headers_size = ELF64_EHDR_SIZE + (phnum as usize) * ELF64_PHDR_SIZE;
        let notes_offset = headers_size;
        let notes_size = notes.len();

        // Data starts after headers + notes
        let mut data_offset = notes_offset + notes_size;
        // Align to 4096 for segment data
        data_offset = (data_offset + 4095) & !4095;

        // Update segment file offsets
        let mut current_offset = data_offset as u64;
        for seg in &mut self.segments {
            seg.file_offset = current_offset;
            current_offset += seg.size;
        }

        // Build the complete file
        let mut output = Vec::new();

        // ELF header
        self.build_elf_header(phnum, &mut output);

        // PT_NOTE program header
        Self::build_phdr(
            &mut output,
            PT_NOTE,
            0,
            notes_offset as u64,
            0,
            0,
            notes_size as u64,
            notes_size as u64,
            4,
        );

        // PT_LOAD program headers for each segment
        for seg in &self.segments {
            Self::build_phdr(
                &mut output,
                PT_LOAD,
                seg.flags,
                seg.file_offset,
                seg.vaddr,
                0,
                seg.size,
                seg.size,
                4096,
            );
        }

        // Notes
        output.extend_from_slice(&notes);

        // Pad to data offset
        while output.len() < data_offset {
            output.push(0);
        }

        // Segment data
        for data in &self.segment_data {
            output.extend_from_slice(data);
        }

        Ok(output)
    }

    /// Get the total number of segments
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Estimated size of the core dump
    pub fn estimated_size(&self) -> usize {
        let headers = ELF64_EHDR_SIZE + (1 + self.segments.len()) * ELF64_PHDR_SIZE;
        let notes = 512; // estimated
        let data: u64 = self.segments.iter().map(|s| s.size).sum();
        headers + notes + data as usize
    }
}

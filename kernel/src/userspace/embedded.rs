//! Embedded minimal ELF binaries for user-space init and shell
//!
//! This module provides pre-built minimal ELF64 binaries that are embedded
//! directly in the kernel image. During bootstrap, these binaries are written
//! to the RamFS so that the ELF loader can find them at standard paths
//! (`/sbin/init`, `/bin/vsh`).
//!
//! Currently only x86_64 binaries are provided. Other architectures fall back
//! to the existing `create_minimal_init()` path.

#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
extern crate alloc;

#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// ELF64 layout constants
// ---------------------------------------------------------------------------

/// Size of ELF64 header (bytes).
#[cfg(target_arch = "x86_64")]
const ELF64_EHDR_SIZE: usize = 64;

/// Size of one ELF64 program header entry (bytes).
#[cfg(target_arch = "x86_64")]
const ELF64_PHDR_SIZE: usize = 56;

/// File offset where code begins (immediately after headers).
#[cfg(target_arch = "x86_64")]
const CODE_OFFSET: usize = ELF64_EHDR_SIZE + ELF64_PHDR_SIZE; // 0x78

// ---------------------------------------------------------------------------
// x86_64 machine code for the init process
// ---------------------------------------------------------------------------

/// x86_64 machine code for the minimal init process.
///
/// Writes "VeridianOS init started\n" to stdout (fd 1) using VeridianOS
/// syscall FileWrite (53), then exits with code 0 via ProcessExit (11).
///
/// Disassembly:
/// ```text
///   0: bf 01 00 00 00          mov  edi, 1           ; fd = stdout
///   5: 48 8d 35 15 00 00 00    lea  rsi, [rip+0x15]  ; buf = &msg
///  12: ba 18 00 00 00          mov  edx, 24          ; len = 24
///  17: b8 35 00 00 00          mov  eax, 53          ; SYS_WRITE (FileWrite)
///  22: 0f 05                   syscall
///  24: 31 ff                   xor  edi, edi         ; exit_code = 0
///  26: b8 0b 00 00 00          mov  eax, 11          ; SYS_EXIT (ProcessExit)
///  31: 0f 05                   syscall
///  33: "VeridianOS init started\n"
/// ```
#[cfg(target_arch = "x86_64")]
const INIT_CODE: &[u8] = &[
    // mov edi, 1
    0xBF, 0x01, 0x00, 0x00, 0x00,
    // lea rsi, [rip+0x15]  (displacement = 33 - 12 = 21 = 0x15)
    0x48, 0x8D, 0x35, 0x15, 0x00, 0x00, 0x00, // mov edx, 24
    0xBA, 0x18, 0x00, 0x00, 0x00, // mov eax, 53  (FileWrite)
    0xB8, 0x35, 0x00, 0x00, 0x00, // syscall
    0x0F, 0x05, // xor edi, edi
    0x31, 0xFF, // mov eax, 11  (ProcessExit)
    0xB8, 0x0B, 0x00, 0x00, 0x00, // syscall
    0x0F, 0x05, // msg: "VeridianOS init started\n" (24 bytes)
    b'V', b'e', b'r', b'i', b'd', b'i', b'a', b'n', b'O', b'S', b' ', b'i', b'n', b'i', b't', b' ',
    b's', b't', b'a', b'r', b't', b'e', b'd', b'\n',
];

// ---------------------------------------------------------------------------
// x86_64 machine code for the shell (vsh)
// ---------------------------------------------------------------------------

/// x86_64 machine code for the minimal shell process.
///
/// Writes "vsh> " to stdout, then exits with code 0. A future sprint will
/// extend this to read input and dispatch commands.
///
/// Disassembly:
/// ```text
///   0: bf 01 00 00 00          mov  edi, 1           ; fd = stdout
///   5: 48 8d 35 15 00 00 00    lea  rsi, [rip+0x15]  ; buf = &msg
///  12: ba 05 00 00 00          mov  edx, 5           ; len = 5
///  17: b8 35 00 00 00          mov  eax, 53          ; SYS_WRITE (FileWrite)
///  22: 0f 05                   syscall
///  24: 31 ff                   xor  edi, edi         ; exit_code = 0
///  26: b8 0b 00 00 00          mov  eax, 11          ; SYS_EXIT (ProcessExit)
///  31: 0f 05                   syscall
///  33: "vsh> \n"
/// ```
#[cfg(target_arch = "x86_64")]
const SHELL_CODE: &[u8] = &[
    // mov edi, 1
    0xBF, 0x01, 0x00, 0x00, 0x00,
    // lea rsi, [rip+0x15]  (displacement = 33 - 12 = 21 = 0x15)
    0x48, 0x8D, 0x35, 0x15, 0x00, 0x00, 0x00, // mov edx, 6
    0xBA, 0x06, 0x00, 0x00, 0x00, // mov eax, 53  (FileWrite)
    0xB8, 0x35, 0x00, 0x00, 0x00, // syscall
    0x0F, 0x05, // xor edi, edi
    0x31, 0xFF, // mov eax, 11  (ProcessExit)
    0xB8, 0x0B, 0x00, 0x00, 0x00, // syscall
    0x0F, 0x05, // msg: "vsh> \n" (6 bytes)
    b'v', b's', b'h', b'>', b' ', b'\n',
];

// ---------------------------------------------------------------------------
// Public accessors for raw machine code
// ---------------------------------------------------------------------------

/// Return the raw x86_64 machine code for the init process.
///
/// This is the position-independent code that will be copied to a user-space
/// page. It uses RIP-relative addressing so it can run at any address.
#[cfg(target_arch = "x86_64")]
pub fn init_code_bytes() -> &'static [u8] {
    INIT_CODE
}

// ---------------------------------------------------------------------------
// ELF64 builder
// ---------------------------------------------------------------------------

/// Build a minimal ELF64 executable from raw machine code.
///
/// Returns a `Vec<u8>` containing a valid ELF64 binary with one PT_LOAD
/// segment. The binary is laid out as:
///
/// | Offset  | Content               | Size     |
/// |---------|-----------------------|----------|
/// | 0x00    | ELF64 header          | 64 bytes |
/// | 0x40    | Program header (LOAD) | 56 bytes |
/// | 0x78    | Machine code          | variable |
///
/// The entry point is set to `load_addr + 0x78` so execution begins at the
/// first byte of `code`.
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
fn build_minimal_elf(code: &[u8], load_addr: u64) -> Vec<u8> {
    let total_size = CODE_OFFSET + code.len();
    let entry_point = load_addr + CODE_OFFSET as u64;

    let mut elf = Vec::with_capacity(total_size);

    // -----------------------------------------------------------------------
    // ELF64 Header (64 bytes)
    // -----------------------------------------------------------------------

    // e_ident[0..4]: magic
    elf.extend_from_slice(&[0x7F, b'E', b'L', b'F']);
    // e_ident[4]: class = ELFCLASS64
    elf.push(2);
    // e_ident[5]: data = ELFDATA2LSB (little-endian)
    elf.push(1);
    // e_ident[6]: version = EV_CURRENT
    elf.push(1);
    // e_ident[7]: OS/ABI = ELFOSABI_NONE
    elf.push(0);
    // e_ident[8..16]: padding
    elf.extend_from_slice(&[0u8; 8]);

    // e_type: ET_EXEC (2)
    elf.extend_from_slice(&2u16.to_le_bytes());
    // e_machine: EM_X86_64 (62)
    elf.extend_from_slice(&62u16.to_le_bytes());
    // e_version: EV_CURRENT (1)
    elf.extend_from_slice(&1u32.to_le_bytes());
    // e_entry: entry point
    elf.extend_from_slice(&entry_point.to_le_bytes());
    // e_phoff: program header offset (immediately after ELF header)
    elf.extend_from_slice(&(ELF64_EHDR_SIZE as u64).to_le_bytes());
    // e_shoff: section header offset (none)
    elf.extend_from_slice(&0u64.to_le_bytes());
    // e_flags
    elf.extend_from_slice(&0u32.to_le_bytes());
    // e_ehsize: ELF header size
    elf.extend_from_slice(&(ELF64_EHDR_SIZE as u16).to_le_bytes());
    // e_phentsize: program header entry size
    elf.extend_from_slice(&(ELF64_PHDR_SIZE as u16).to_le_bytes());
    // e_phnum: number of program headers
    elf.extend_from_slice(&1u16.to_le_bytes());
    // e_shentsize: section header entry size (none)
    elf.extend_from_slice(&0u16.to_le_bytes());
    // e_shnum: number of section headers
    elf.extend_from_slice(&0u16.to_le_bytes());
    // e_shstrndx: section name string table index
    elf.extend_from_slice(&0u16.to_le_bytes());

    debug_assert_eq!(elf.len(), ELF64_EHDR_SIZE);

    // -----------------------------------------------------------------------
    // Program Header: PT_LOAD (56 bytes)
    // -----------------------------------------------------------------------

    // p_type: PT_LOAD (1)
    elf.extend_from_slice(&1u32.to_le_bytes());
    // p_flags: PF_R | PF_X (0x5)
    elf.extend_from_slice(&5u32.to_le_bytes());
    // p_offset: load from start of file
    elf.extend_from_slice(&0u64.to_le_bytes());
    // p_vaddr: virtual load address
    elf.extend_from_slice(&load_addr.to_le_bytes());
    // p_paddr: physical load address (same as vaddr)
    elf.extend_from_slice(&load_addr.to_le_bytes());
    // p_filesz: file size of segment
    elf.extend_from_slice(&(total_size as u64).to_le_bytes());
    // p_memsz: memory size of segment (same as filesz)
    elf.extend_from_slice(&(total_size as u64).to_le_bytes());
    // p_align: alignment
    elf.extend_from_slice(&0x1000u64.to_le_bytes());

    debug_assert_eq!(elf.len(), CODE_OFFSET);

    // -----------------------------------------------------------------------
    // Code
    // -----------------------------------------------------------------------

    elf.extend_from_slice(code);

    debug_assert_eq!(elf.len(), total_size);
    elf
}

// ---------------------------------------------------------------------------
// RamFS population
// ---------------------------------------------------------------------------

/// Populate the RamFS with embedded init and shell binaries.
///
/// Creates `/sbin/init` and `/bin/vsh` in the VFS so that
/// `load_init_process()` and `load_shell()` can find real ELF executables
/// instead of falling back to stub processes.
///
/// Must be called after VFS is initialized (Stage 4) but before
/// `create_init_process()` (Stage 6).
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
pub fn populate_initramfs() -> Result<(), crate::error::KernelError> {
    use crate::fs;

    // User-space load address for both binaries.
    // 0x40_0000 (4 MiB) is a conventional user-space base address,
    // well within the lower-half user address space.
    const LOAD_ADDR: u64 = 0x40_0000;

    let init_elf = build_minimal_elf(INIT_CODE, LOAD_ADDR);
    let shell_elf = build_minimal_elf(SHELL_CODE, LOAD_ADDR);

    // /sbin and /bin are already created by fs::init(), but guard against
    // them being absent. Ignore AlreadyExists errors.
    let vfs = fs::get_vfs().read();
    let _ = vfs.mkdir("/sbin", fs::Permissions::default());
    let _ = vfs.mkdir("/bin", fs::Permissions::default());
    drop(vfs);

    // Write init binary to /sbin/init
    fs::write_file("/sbin/init", &init_elf)?;

    // Write shell binary to /bin/vsh
    fs::write_file("/bin/vsh", &shell_elf)?;

    Ok(())
}

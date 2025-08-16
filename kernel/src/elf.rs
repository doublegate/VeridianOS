//! ELF64 loader for user program execution

use core::mem;
use core::slice;
use alloc::vec::Vec;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Header {
    pub magic: [u8; 4],
    pub class: u8,
    pub data: u8,
    pub version: u8,
    pub osabi: u8,
    pub abiversion: u8,
    pub pad: [u8; 7],
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

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64ProgramHeader {
    pub p_type: u32,
    pub flags: u32,
    pub offset: u64,
    pub vaddr: u64,
    pub paddr: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub align: u64,
}

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELF_CLASS_64: u8 = 2;
const ELF_DATA_2LSB: u8 = 1;
const ELF_VERSION_CURRENT: u8 = 1;

const _PT_NULL: u32 = 0;
const PT_LOAD: u32 = 1;
const _PT_DYNAMIC: u32 = 2;
const _PT_INTERP: u32 = 3;
const _PT_NOTE: u32 = 4;
const _PT_SHLIB: u32 = 5;
const _PT_PHDR: u32 = 6;
const _PT_TLS: u32 = 7;

const PF_X: u32 = 0x1;
const PF_W: u32 = 0x2;
const PF_R: u32 = 0x4;

pub struct ElfLoader;

impl ElfLoader {
    pub fn validate_header(data: &[u8]) -> Result<&Elf64Header, &'static str> {
        if data.len() < mem::size_of::<Elf64Header>() {
            return Err("ELF file too small");
        }

        let header = unsafe { &*(data.as_ptr() as *const Elf64Header) };

        if header.magic != ELF_MAGIC {
            return Err("Invalid ELF magic");
        }

        if header.class != ELF_CLASS_64 {
            return Err("Not a 64-bit ELF");
        }

        if header.data != ELF_DATA_2LSB {
            return Err("Not little-endian");
        }

        if header.version != ELF_VERSION_CURRENT {
            return Err("Invalid ELF version");
        }

        #[cfg(target_arch = "x86_64")]
        if header.machine != 0x3E {
            return Err("Not an x86_64 executable");
        }

        #[cfg(target_arch = "aarch64")]
        if header.machine != 0xB7 {
            return Err("Not an AArch64 executable");
        }

        #[cfg(target_arch = "riscv64")]
        if header.machine != 0xF3 {
            return Err("Not a RISC-V executable");
        }

        Ok(header)
    }

    pub fn get_program_headers(data: &[u8], header: &Elf64Header) -> Result<Vec<Elf64ProgramHeader>, &'static str> {
        let mut headers = Vec::new();
        
        if header.phoff == 0 || header.phnum == 0 {
            return Ok(headers);
        }

        let ph_start = header.phoff as usize;
        let ph_size = header.phentsize as usize;
        let ph_count = header.phnum as usize;

        if ph_start + ph_size * ph_count > data.len() {
            return Err("Program headers out of bounds");
        }

        for i in 0..ph_count {
            let offset = ph_start + i * ph_size;
            let ph = unsafe {
                &*(data.as_ptr().add(offset) as *const Elf64ProgramHeader)
            };
            headers.push(*ph);
        }

        Ok(headers)
    }

    pub fn load_segments(
        data: &[u8],
        headers: &[Elf64ProgramHeader],
        vas: &mut crate::mm::vas::VirtualAddressSpace,
    ) -> Result<(), &'static str> {
        use crate::mm::PAGE_SIZE;
        use crate::mm::PageFlags;

        for header in headers {
            if header.p_type != PT_LOAD {
                continue;
            }

            if header.filesz > header.memsz {
                return Err("Invalid segment size");
            }

            let vaddr = header.vaddr as usize;
            let memsz = header.memsz as usize;
            let filesz = header.filesz as usize;
            let offset = header.offset as usize;

            if offset + filesz > data.len() {
                return Err("Segment data out of bounds");
            }

            let page_start = vaddr & !(PAGE_SIZE - 1);
            let page_end = ((vaddr + memsz + PAGE_SIZE - 1) / PAGE_SIZE) * PAGE_SIZE;
            let num_pages = (page_end - page_start) / PAGE_SIZE;

            let mut flags = PageFlags::USER | PageFlags::PRESENT;
            if header.flags & PF_W != 0 {
                flags |= PageFlags::WRITABLE;
            }
            if header.flags & PF_X == 0 {
                flags |= PageFlags::NO_EXECUTE;
            }

            for i in 0..num_pages {
                let page_addr = page_start + i * PAGE_SIZE;
                vas.map_page(page_addr, flags)?;
            }

            unsafe {
                let segment_data = &data[offset..offset + filesz];
                let dest = slice::from_raw_parts_mut(vaddr as *mut u8, filesz);
                dest.copy_from_slice(segment_data);

                if memsz > filesz {
                    let zero_start = vaddr + filesz;
                    let zero_size = memsz - filesz;
                    let zeros = slice::from_raw_parts_mut(zero_start as *mut u8, zero_size);
                    zeros.fill(0);
                }
            }
        }

        Ok(())
    }

    pub fn load(data: &[u8], vas: &mut crate::mm::vas::VirtualAddressSpace) -> Result<u64, &'static str> {
        let header = Self::validate_header(data)?;
        let program_headers = Self::get_program_headers(data, header)?;
        Self::load_segments(data, &program_headers, vas)?;
        Ok(header.entry)
    }
}
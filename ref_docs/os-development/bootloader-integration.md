# Bootloader Integration Guide

**Current Status:** Phase 1 COMPLETE (v0.2.1 - June 17, 2025)
- Latest release: v0.2.1 - Maintenance Release
- All three architectures (x86_64, AArch64, RISC-V) boot to Stage 6
- Zero warnings and clippy-clean across all architectures
- Ready for Phase 2 User Space Foundation development

This document covers bootloader integration patterns for VeridianOS.

## UEFI Boot Process

### Basic UEFI Application Structure
```rust
#![no_main]
#![no_std]
#![feature(abi_efiapi)]

use r_efi::efi;

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[export_name = "efi_main"]
pub extern "efiapi" fn main(handle: efi::Handle, st: *mut efi::SystemTable) -> efi::Status {
    // Early initialization
    // Set up graphics
    // Load kernel
    // Jump to kernel entry
    efi::Status::SUCCESS
}
```

### UEFI with Standard Library
```rust
#![feature(uefi_std)]

use r_efi::{efi, protocols::simple_text_output};
use std::{
    ffi::OsString,
    os::uefi::{env, ffi::OsStrExt}
};

pub fn main() {
    println!("Starting VeridianOS Bootloader...");
    
    let st = env::system_table().as_ptr() as *mut efi::SystemTable;
    // Continue boot process
}
```

## Multiboot2 Specification

For BIOS-based systems, Multiboot2 provides a standard interface:

### Multiboot2 Header
```rust
#[repr(C, align(8))]
struct Multiboot2Header {
    magic: u32,
    architecture: u32,
    header_length: u32,
    checksum: u32,
    // Tags follow
}

const MULTIBOOT2_HEADER_MAGIC: u32 = 0xe85250d6;
const MULTIBOOT2_HEADER_ARCH_I386: u32 = 0;
```

## Custom Bootloader Considerations

### Memory Map Requirements
1. Identity map first 1MB for legacy compatibility
2. Map kernel at higher half (0xFFFF_8000_0000_0000)
3. Set up page tables before jumping to kernel

### Boot Information Structure
```rust
#[repr(C)]
pub struct BootInfo {
    pub memory_map: &'static [MemoryRegion],
    pub framebuffer: Option<FramebufferInfo>,
    pub rsdp_addr: Option<u64>,
    pub kernel_addr: u64,
    pub kernel_size: u64,
}

#[repr(C)]
pub struct MemoryRegion {
    pub start: u64,
    pub length: u64,
    pub kind: MemoryRegionKind,
}

#[repr(u32)]
pub enum MemoryRegionKind {
    Usable = 1,
    Reserved = 2,
    AcpiReclaimable = 3,
    AcpiNvs = 4,
    BadMemory = 5,
    Bootloader = 6,
    KernelAndModules = 7,
}
```

## Linker Script Integration

### Basic Kernel Linker Script
```ld
ENTRY(_start)

SECTIONS {
    . = 0xFFFF800000000000;  /* Higher half kernel */
    
    .text : {
        *(.text.boot)       /* Boot code first */
        *(.text .text.*)
    }
    
    .rodata : {
        *(.rodata .rodata.*)
    }
    
    .data : {
        *(.data .data.*)
    }
    
    .bss : {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }
    
    /DISCARD/ : {
        *(.comment)
        *(.note.*)
    }
}
```

## Boot Sequence Checklist

1. **CPU Initialization**
   - Set up GDT
   - Enable A20 line (for legacy BIOS)
   - Switch to long mode (x86_64)
   - Set up initial page tables

2. **Memory Detection**
   - Parse E820 memory map (BIOS) or UEFI memory map
   - Reserve kernel and bootloader regions

3. **Load Kernel**
   - Parse ELF headers
   - Load segments to proper addresses
   - Clear BSS section

4. **Prepare Boot Information**
   - Fill BootInfo structure
   - Pass memory map to kernel
   - Pass framebuffer info if available

5. **Jump to Kernel**
   - Set up initial stack
   - Clear registers
   - Jump to kernel entry point

## Platform-Specific Notes

### x86_64
- Requires long mode setup
- GDT with 64-bit code segment
- Identity mapping for first 2MB

### AArch64
- Set up exception levels (EL2 → EL1)
- Configure MMU and page tables
- Set up stack pointer for each exception level

### RISC-V
- Configure PMP (Physical Memory Protection)
- Set up page tables
- Switch to supervisor mode

## Testing with QEMU

### Direct Kernel Boot (bypass bootloader)
```bash
qemu-system-x86_64 -kernel kernel.elf -append "console=ttyS0" -serial stdio
```

### With UEFI Firmware
```bash
qemu-system-x86_64 -bios /usr/share/ovmf/OVMF.fd -drive file=boot.img,format=raw
```

### Debugging Boot Process
```bash
qemu-system-x86_64 -s -S -kernel kernel.elf
# In another terminal: gdb kernel.elf -ex "target remote :1234"
```
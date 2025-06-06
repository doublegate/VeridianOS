# Veridian OS: Bootstrap Implementation Guide

## Table of Contents

1. [Introduction](#introduction)
2. [Development Environment Setup](#development-environment-setup)
3. [Project Structure Initialization](#project-structure-initialization)
4. [Stage 0: Minimal Bootloader](#stage-0-minimal-bootloader)
5. [Stage 1: Kernel Entry](#stage-1-kernel-entry)
6. [Stage 2: Basic Output](#stage-2-basic-output)
7. [Stage 3: Memory Detection](#stage-3-memory-detection)
8. [Stage 4: Page Table Setup](#stage-4-page-table-setup)
9. [Stage 5: Higher Half Kernel](#stage-5-higher-half-kernel)
10. [Stage 6: Memory Management](#stage-6-memory-management)
11. [Stage 7: Interrupt Handling](#stage-7-interrupt-handling)
12. [Stage 8: Multitasking Foundation](#stage-8-multitasking-foundation)
13. [Stage 9: System Call Interface](#stage-9-system-call-interface)
14. [Stage 10: Initial Capability System](#stage-10-initial-capability-system)
15. [Testing Each Stage](#testing-each-stage)
16. [Debugging Techniques](#debugging-techniques)
17. [Common Pitfalls](#common-pitfalls)
18. [Next Steps](#next-steps)

## Introduction

This guide provides a hands-on, step-by-step approach to bootstrapping Veridian OS from zero to a minimal working kernel. Each stage builds upon the previous one, with working code that can be tested immediately.

### Prerequisites

- Rust nightly toolchain
- QEMU for testing
- Basic understanding of OS concepts
- Familiarity with x86_64 architecture (we'll start with this)

### Approach

We'll implement each component incrementally:
1. Get something minimal working
2. Test it thoroughly
3. Refactor for cleanliness
4. Add features progressively

## Development Environment Setup

### Required Tools Installation

```bash
# Install Rust nightly
rustup toolchain install nightly-2025-01-15
rustup default nightly-2025-01-15
rustup component add rust-src llvm-tools-preview

# Install development tools
cargo install bootimage
cargo install cargo-xbuild
cargo install xargo

# Install QEMU (Ubuntu/Debian)
sudo apt-get update
sudo apt-get install qemu-system-x86 qemu-utils

# Install debugging tools
sudo apt-get install gdb
```

### Workspace Configuration

Create `.cargo/config.toml`:

```toml
[unstable]
build-std-features = ["compiler-builtins-mem"]
build-std = ["core", "compiler_builtins", "alloc"]

[build]
target = "x86_64-veridian.json"

[target.'cfg(target_os = "none")']
runner = "bootimage runner"
```

### Custom Target Specification

Create `x86_64-veridian.json`:

```json
{
    "llvm-target": "x86_64-unknown-none",
    "data-layout": "e-m:e-i64:64-f80:128-n8:16:32:64-S128",
    "arch": "x86_64",
    "target-endian": "little",
    "target-pointer-width": "64",
    "target-c-int-width": "32",
    "os": "none",
    "executables": true,
    "linker-flavor": "ld.lld",
    "linker": "rust-lld",
    "panic-strategy": "abort",
    "disable-redzone": true,
    "features": "-mmx,-sse,+soft-float"
}
```

## Project Structure Initialization

### Create Initial Directory Structure

```bash
mkdir -p veridian-os
cd veridian-os

# Create workspace structure
mkdir -p kernel/src/arch/x86_64
mkdir -p bootloader/src
mkdir -p libs/libveridian/src
mkdir -p tools
mkdir -p tests
```

### Workspace Cargo.toml

```toml
[workspace]
members = [
    "bootloader",
    "kernel",
    "libs/libveridian",
]
resolver = "2"

[profile.dev]
panic = "abort"
opt-level = 1

[profile.release]
panic = "abort"
opt-level = 3
lto = true
codegen-units = 1
```

## Stage 0: Minimal Bootloader

### Basic UEFI Bootloader

Create `bootloader/Cargo.toml`:

```toml
[package]
name = "veridian-bootloader"
version = "0.1.0"
edition = "2021"

[dependencies]
uefi = "0.20"
uefi-services = "0.17"
log = "0.4"
xmas-elf = "0.9"

[[bin]]
name = "bootloader"
path = "src/main.rs"
```

Create `bootloader/src/main.rs`:

```rust
#![no_std]
#![no_main]
#![feature(abi_efiapi)]

use core::mem;
use log::info;
use uefi::prelude::*;
use uefi::proto::console::text::Output;
use uefi::proto::media::file::{File, FileAttribute, FileMode};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::table::boot::{AllocateType, MemoryDescriptor, MemoryType};

/// UEFI entry point
#[entry]
fn main(image_handle: Handle, mut system_table: SystemTable<Boot>) -> Status {
    // Initialize UEFI services
    uefi_services::init(&mut system_table).unwrap();
    
    info!("Veridian OS Bootloader v0.1.0");
    info!("Initializing...");
    
    // Clear screen
    system_table.stdout().clear().unwrap();
    
    // Get boot services
    let boot_services = system_table.boot_services();
    
    // Load kernel
    let kernel_data = load_kernel(boot_services)?;
    info!("Kernel loaded: {} bytes", kernel_data.len());
    
    // Get memory map
    let mmap_size = boot_services.memory_map_size();
    let mut mmap_buffer = vec![0u8; mmap_size + 1024];
    let (_, mmap_iter) = boot_services
        .memory_map(&mut mmap_buffer)
        .expect("Failed to get memory map");
    
    // Convert memory map to our format
    let memory_map = create_memory_map(mmap_iter);
    
    // Parse kernel ELF
    let kernel_elf = xmas_elf::ElfFile::new(&kernel_data)
        .expect("Failed to parse kernel ELF");
    
    // Load kernel segments
    let entry_point = load_elf(&kernel_elf, boot_services)?;
    info!("Kernel entry point: {:#x}", entry_point);
    
    // Create boot info structure
    let boot_info = create_boot_info(memory_map);
    
    // Exit boot services
    let (_runtime_table, _mmap_iter) = 
        system_table.exit_boot_services(image_handle, &mut mmap_buffer)?;
    
    // Jump to kernel
    let kernel_entry: extern "C" fn(&BootInfo) -> ! = 
        unsafe { mem::transmute(entry_point) };
    kernel_entry(&boot_info);
}

fn load_kernel(boot_services: &BootServices) -> Result<Vec<u8>, Status> {
    // Open root directory
    let mut fs = boot_services
        .locate_protocol::<SimpleFileSystem>()?;
    let mut root = unsafe { &mut *fs.get() }.open_volume()?;
    
    // Open kernel file
    let mut kernel_file = root
        .open(
            cstr16!("\\kernel.elf"),
            FileMode::Read,
            FileAttribute::empty(),
        )?
        .into_regular_file()
        .ok_or(Status::NOT_FOUND)?;
    
    // Get file size
    let info = kernel_file.get_boxed_info::<FileInfo>()?;
    let kernel_size = info.file_size() as usize;
    
    // Read kernel
    let mut kernel_data = vec![0u8; kernel_size];
    kernel_file.read(&mut kernel_data)?;
    
    Ok(kernel_data)
}

#[repr(C)]
pub struct BootInfo {
    pub memory_map: &'static [MemoryRegion],
    pub framebuffer: Option<FramebufferInfo>,
    pub rsdp_address: Option<u64>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub start: u64,
    pub end: u64,
    pub kind: MemoryRegionKind,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegionKind {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    BootloaderReclaimable,
    KernelAndModules,
    Framebuffer,
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    log::error!("PANIC: {}", info);
    loop {}
}
```

### Legacy BIOS Bootloader (Alternative)

For BIOS systems, create a simpler bootloader:

```rust
// bootloader/src/bios.rs
#![no_std]
#![no_main]

use core::arch::asm;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        // Set up segments
        asm!(
            "xor ax, ax",
            "mov ds, ax",
            "mov es, ax",
            "mov ss, ax",
            "mov sp, 0x7c00",
        );
        
        // Clear screen (BIOS int 10h)
        asm!(
            "mov ah, 0x00",
            "mov al, 0x03",
            "int 0x10",
        );
        
        // Print message
        let msg = b"Veridian OS Loading...\r\n";
        for &byte in msg {
            print_char(byte);
        }
        
        // Load kernel from disk
        load_kernel_from_disk();
        
        // Jump to kernel
        unsafe {
            asm!(
                "jmp 0x10000",
                options(noreturn)
            );
        }
    }
}

fn print_char(c: u8) {
    unsafe {
        asm!(
            "mov ah, 0x0e",
            "int 0x10",
            in("al") c,
        );
    }
}
```

## Stage 1: Kernel Entry

### Kernel Entry Point

Create `kernel/src/main.rs`:

```rust
#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;

/// Boot information passed from bootloader
#[repr(C)]
pub struct BootInfo {
    pub memory_map: &'static [MemoryRegion],
    pub framebuffer: Option<FramebufferInfo>,
    pub rsdp_address: Option<u64>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub start: u64,
    pub end: u64,
    pub kind: MemoryRegionKind,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegionKind {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    BootloaderReclaimable,
    KernelAndModules,
    Framebuffer,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    pub buffer_start: u64,
    pub buffer_len: usize,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub pixel_format: PixelFormat,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum PixelFormat {
    Rgb,
    Bgr,
}

/// Kernel entry point
#[no_mangle]
pub extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    // Initialize kernel
    kernel_init(boot_info);
    
    // Should never return
    loop {
        x86_64::instructions::hlt();
    }
}

fn kernel_init(boot_info: &'static BootInfo) {
    // Stage 1: We're running!
    // For now, just loop
    
    #[cfg(test)]
    test_main();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // For now, just loop
    loop {
        x86_64::instructions::hlt();
    }
}

#[cfg(test)]
fn test_runner(tests: &[&dyn Fn()]) {
    for test in tests {
        test();
    }
}
```

### Linker Script

Create `kernel/src/linker.ld`:

```ld
ENTRY(_start)
OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)

KERNEL_BASE = 0xFFFFFFFF80000000;

SECTIONS {
    . = KERNEL_BASE + 0x100000;
    
    .text : AT(ADDR(.text) - KERNEL_BASE) {
        _kernel_start = .;
        *(.text .text.*)
    }
    
    .rodata : AT(ADDR(.rodata) - KERNEL_BASE) {
        *(.rodata .rodata.*)
    }
    
    .data : AT(ADDR(.data) - KERNEL_BASE) {
        *(.data .data.*)
    }
    
    .bss : AT(ADDR(.bss) - KERNEL_BASE) {
        _bss_start = .;
        *(.bss .bss.*)
        . = ALIGN(4096);
        _bss_end = .;
    }
    
    _kernel_end = .;
    
    /DISCARD/ : {
        *(.comment)
        *(.eh_frame)
        *(.note.*)
    }
}
```

## Stage 2: Basic Output

### Serial Output for Debugging

Create `kernel/src/serial.rs`:

```rust
//! Serial port driver for early debugging output

use core::fmt;
use lazy_static::lazy_static;
use spin::Mutex;
use uart_16550::SerialPort;

lazy_static! {
    pub static ref SERIAL1: Mutex<SerialPort> = {
        let mut serial_port = unsafe { SerialPort::new(0x3F8) };
        serial_port.init();
        Mutex::new(serial_port)
    };
}

/// Print to serial output
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_print(format_args!($($arg)*));
    };
}

/// Print line to serial output
#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($fmt:expr) => ($crate::serial_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => ($crate::serial_print!(
        concat!($fmt, "\n"), $($arg)*));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    use x86_64::instructions::interrupts;
    
    interrupts::without_interrupts(|| {
        SERIAL1.lock().write_fmt(args).unwrap();
    });
}

/// Early println for use before serial is initialized
pub unsafe fn early_println(s: &str) {
    use x86_64::instructions::port::Port;
    
    let mut port = Port::new(0x3F8);
    for byte in s.bytes() {
        port.write(byte);
    }
    port.write(b'\n');
}
```

### VGA Text Mode Output

Create `kernel/src/vga.rs`:

```rust
//! VGA text mode driver

use core::fmt;
use lazy_static::lazy_static;
use spin::Mutex;
use volatile::Volatile;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct ColorCode(u8);

impl ColorCode {
    fn new(foreground: Color, background: Color) -> ColorCode {
        ColorCode((background as u8) << 4 | (foreground as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct ScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;

#[repr(transparent)]
struct Buffer {
    chars: [[Volatile<ScreenChar>; BUFFER_WIDTH]; BUFFER_HEIGHT],
}

pub struct Writer {
    column_position: usize,
    color_code: ColorCode,
    buffer: &'static mut Buffer,
}

impl Writer {
    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            byte => {
                if self.column_position >= BUFFER_WIDTH {
                    self.new_line();
                }
                
                let row = BUFFER_HEIGHT - 1;
                let col = self.column_position;
                
                let color_code = self.color_code;
                self.buffer.chars[row][col].write(ScreenChar {
                    ascii_character: byte,
                    color_code,
                });
                self.column_position += 1;
            }
        }
    }
    
    pub fn write_string(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                // Printable ASCII byte or newline
                0x20..=0x7e | b'\n' => self.write_byte(byte),
                // Not part of printable ASCII range
                _ => self.write_byte(0xfe),
            }
        }
    }
    
    fn new_line(&mut self) {
        for row in 1..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let character = self.buffer.chars[row][col].read();
                self.buffer.chars[row - 1][col].write(character);
            }
        }
        self.clear_row(BUFFER_HEIGHT - 1);
        self.column_position = 0;
    }
    
    fn clear_row(&mut self, row: usize) {
        let blank = ScreenChar {
            ascii_character: b' ',
            color_code: self.color_code,
        };
        for col in 0..BUFFER_WIDTH {
            self.buffer.chars[row][col].write(blank);
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

lazy_static! {
    pub static ref WRITER: Mutex<Writer> = Mutex::new(Writer {
        column_position: 0,
        color_code: ColorCode::new(Color::Yellow, Color::Black),
        buffer: unsafe { &mut *(0xb8000 as *mut Buffer) },
    });
}

/// Print to VGA buffer
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::vga::_print(format_args!($($arg)*)));
}

/// Print line to VGA buffer
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    use x86_64::instructions::interrupts;
    
    interrupts::without_interrupts(|| {
        WRITER.lock().write_fmt(args).unwrap();
    });
}
```

### Update Kernel Main

```rust
// Add to kernel/src/main.rs

mod serial;
mod vga;

fn kernel_init(boot_info: &'static BootInfo) {
    // Initialize display
    println!("Veridian OS v0.1.0");
    println!("Kernel initialized");
    
    // Initialize serial for debugging
    serial_println!("Serial output initialized");
    
    // Print memory map
    println!("Memory regions:");
    for region in boot_info.memory_map.iter() {
        println!("  {:#x} - {:#x} {:?}", 
                 region.start, region.end, region.kind);
    }
}
```

## Stage 3: Memory Detection

### Physical Memory Manager Foundation

Create `kernel/src/memory/mod.rs`:

```rust
//! Memory management subsystem

pub mod frame_allocator;
pub mod paging;

use crate::BootInfo;

/// Physical address
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PhysAddr(u64);

impl PhysAddr {
    pub const fn new(addr: u64) -> Self {
        Self(addr & 0x000F_FFFF_FFFF_FFFF)
    }
    
    pub const fn as_u64(self) -> u64 {
        self.0
    }
    
    pub const fn is_aligned(self, align: u64) -> bool {
        self.0 % align == 0
    }
    
    pub const fn align_down(self, align: u64) -> Self {
        Self::new(self.0 & !(align - 1))
    }
    
    pub const fn align_up(self, align: u64) -> Self {
        Self::new((self.0 + align - 1) & !(align - 1))
    }
}

/// Virtual address
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct VirtAddr(u64);

impl VirtAddr {
    pub const fn new(addr: u64) -> Self {
        // Sign extend to canonical form
        Self(((addr << 16) as i64 >> 16) as u64)
    }
    
    pub const fn as_u64(self) -> u64 {
        self.0
    }
    
    pub const fn as_ptr<T>(self) -> *const T {
        self.0 as *const T
    }
    
    pub const fn as_mut_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }
}

/// Physical memory frame
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysFrame {
    start_address: PhysAddr,
}

impl PhysFrame {
    pub const SIZE: u64 = 4096;
    
    pub const fn containing_address(address: PhysAddr) -> Self {
        Self {
            start_address: address.align_down(Self::SIZE),
        }
    }
    
    pub const fn start_address(self) -> PhysAddr {
        self.start_address
    }
}

/// Initialize memory subsystem
pub fn init(boot_info: &'static BootInfo) {
    println!("Initializing memory management...");
    
    // Initialize frame allocator
    let mut frame_allocator = unsafe {
        frame_allocator::BootFrameAllocator::init(&boot_info.memory_map)
    };
    
    // Count available memory
    let total_memory = boot_info.memory_map.iter()
        .filter(|r| r.kind == crate::MemoryRegionKind::Usable)
        .map(|r| r.end - r.start)
        .sum::<u64>();
    
    println!("Total usable memory: {} MB", total_memory / 1024 / 1024);
}
```

### Boot Frame Allocator

Create `kernel/src/memory/frame_allocator.rs`:

```rust
//! Physical frame allocator

use super::{PhysAddr, PhysFrame};
use crate::{MemoryRegion, MemoryRegionKind};

/// Simple boot frame allocator
pub struct BootFrameAllocator {
    next: PhysAddr,
    end: PhysAddr,
    memory_map: &'static [MemoryRegion],
    current_region: usize,
}

impl BootFrameAllocator {
    /// Initialize allocator with memory map
    pub unsafe fn init(memory_map: &'static [MemoryRegion]) -> Self {
        // Find first usable region
        let mut current_region = 0;
        let mut next = PhysAddr::new(0);
        let mut end = PhysAddr::new(0);
        
        for (i, region) in memory_map.iter().enumerate() {
            if region.kind == MemoryRegionKind::Usable {
                current_region = i;
                next = PhysAddr::new(region.start).align_up(PhysFrame::SIZE);
                end = PhysAddr::new(region.end);
                break;
            }
        }
        
        Self {
            next,
            end,
            memory_map,
            current_region,
        }
    }
    
    /// Allocate a physical frame
    pub fn allocate_frame(&mut self) -> Option<PhysFrame> {
        loop {
            if self.next + PhysFrame::SIZE <= self.end {
                let frame = PhysFrame::containing_address(self.next);
                self.next = PhysAddr::new(self.next.as_u64() + PhysFrame::SIZE);
                return Some(frame);
            }
            
            // Move to next region
            self.current_region += 1;
            if self.current_region >= self.memory_map.len() {
                return None; // Out of memory
            }
            
            let region = &self.memory_map[self.current_region];
            if region.kind == MemoryRegionKind::Usable {
                self.next = PhysAddr::new(region.start).align_up(PhysFrame::SIZE);
                self.end = PhysAddr::new(region.end);
            }
        }
    }
}

pub trait FrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame>;
}

impl FrameAllocator for BootFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.allocate_frame()
    }
}
```

## Stage 4: Page Table Setup

### Page Table Management

Create `kernel/src/memory/paging.rs`:

```rust
//! Page table management for x86_64

use super::{PhysAddr, VirtAddr, PhysFrame, FrameAllocator};
use core::ops::{Index, IndexMut};

/// Page table entry flags
bitflags::bitflags! {
    pub struct PageTableFlags: u64 {
        const PRESENT =         1 << 0;
        const WRITABLE =        1 << 1;
        const USER_ACCESSIBLE = 1 << 2;
        const WRITE_THROUGH =   1 << 3;
        const NO_CACHE =        1 << 4;
        const ACCESSED =        1 << 5;
        const DIRTY =           1 << 6;
        const HUGE_PAGE =       1 << 7;
        const GLOBAL =          1 << 8;
        const NO_EXECUTE =      1 << 63;
    }
}

/// Page table entry
#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct PageTableEntry {
    entry: u64,
}

impl PageTableEntry {
    pub const fn new() -> Self {
        Self { entry: 0 }
    }
    
    pub const fn is_unused(&self) -> bool {
        self.entry == 0
    }
    
    pub fn flags(&self) -> PageTableFlags {
        PageTableFlags::from_bits_truncate(self.entry)
    }
    
    pub fn frame(&self) -> Option<PhysFrame> {
        if self.flags().contains(PageTableFlags::PRESENT) {
            Some(PhysFrame::containing_address(
                PhysAddr::new(self.entry & 0x000F_FFFF_FFFF_F000)
            ))
        } else {
            None
        }
    }
    
    pub fn set_frame(&mut self, frame: PhysFrame, flags: PageTableFlags) {
        assert!(!self.flags().contains(PageTableFlags::HUGE_PAGE));
        self.entry = frame.start_address().as_u64() | flags.bits();
    }
}

/// Page table level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageTableLevel {
    Four = 4,
    Three = 3,
    Two = 2,
    One = 1,
}

/// Page table (all levels)
#[repr(align(4096))]
#[repr(C)]
pub struct PageTable {
    entries: [PageTableEntry; 512],
}

impl PageTable {
    pub const fn new() -> Self {
        const EMPTY: PageTableEntry = PageTableEntry::new();
        Self {
            entries: [EMPTY; 512],
        }
    }
    
    pub fn zero(&mut self) {
        for entry in &mut self.entries {
            entry.entry = 0;
        }
    }
}

impl Index<usize> for PageTable {
    type Output = PageTableEntry;
    
    fn index(&self, index: usize) -> &Self::Output {
        &self.entries[index]
    }
}

impl IndexMut<usize> for PageTable {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.entries[index]
    }
}

/// Virtual page
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Page {
    start_address: VirtAddr,
}

impl Page {
    pub const SIZE: u64 = 4096;
    
    pub const fn containing_address(address: VirtAddr) -> Self {
        Self {
            start_address: VirtAddr::new(address.as_u64() & !0xfff),
        }
    }
    
    pub const fn start_address(self) -> VirtAddr {
        self.start_address
    }
    
    pub const fn p4_index(self) -> usize {
        (self.start_address.as_u64() >> 39) as usize & 0o777
    }
    
    pub const fn p3_index(self) -> usize {
        (self.start_address.as_u64() >> 30) as usize & 0o777
    }
    
    pub const fn p2_index(self) -> usize {
        (self.start_address.as_u64() >> 21) as usize & 0o777
    }
    
    pub const fn p1_index(self) -> usize {
        (self.start_address.as_u64() >> 12) as usize & 0o777
    }
}

/// Active page table mapper
pub struct Mapper {
    p4: &'static mut PageTable,
}

impl Mapper {
    /// Create mapper for current active page table
    pub unsafe fn new() -> Self {
        let p4_addr = x86_64::registers::control::Cr3::read().0.as_u64();
        let p4 = &mut *(p4_addr as *mut PageTable);
        Self { p4 }
    }
    
    /// Map a page to a frame
    pub fn map_to<A: FrameAllocator>(
        &mut self,
        page: Page,
        frame: PhysFrame,
        flags: PageTableFlags,
        allocator: &mut A,
    ) -> Result<(), MapToError> {
        let p4 = &mut self.p4;
        let p3 = self.get_or_create_table(p4, page.p4_index(), allocator)?;
        let p2 = self.get_or_create_table(p3, page.p3_index(), allocator)?;
        let p1 = self.get_or_create_table(p2, page.p2_index(), allocator)?;
        
        if !p1[page.p1_index()].is_unused() {
            return Err(MapToError::PageAlreadyMapped);
        }
        
        p1[page.p1_index()].set_frame(frame, flags | PageTableFlags::PRESENT);
        Ok(())
    }
    
    fn get_or_create_table<A: FrameAllocator>(
        &mut self,
        table: &mut PageTable,
        index: usize,
        allocator: &mut A,
    ) -> Result<&mut PageTable, MapToError> {
        if table[index].is_unused() {
            let frame = allocator.allocate_frame()
                .ok_or(MapToError::FrameAllocationFailed)?;
            table[index].set_frame(frame, 
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
            let table_ptr = frame.start_address().as_u64() as *mut PageTable;
            unsafe {
                (*table_ptr).zero();
            }
        }
        
        let table_addr = table[index].frame().unwrap().start_address().as_u64();
        Ok(unsafe { &mut *(table_addr as *mut PageTable) })
    }
}

#[derive(Debug)]
pub enum MapToError {
    FrameAllocationFailed,
    PageAlreadyMapped,
}
```

## Stage 5: Higher Half Kernel

### Update Boot Process for Higher Half

```rust
// Update kernel/src/main.rs

use memory::{VirtAddr, PhysAddr, Page, PhysFrame, PageTableFlags};

fn kernel_init(boot_info: &'static BootInfo) {
    println!("Veridian OS v0.1.0");
    println!("Kernel initialized at higher half");
    
    // Initialize memory management
    memory::init(boot_info);
    
    // Set up kernel heap
    heap::init();
    
    println!("Memory subsystem initialized");
}

// Add heap module
mod heap {
    use linked_list_allocator::LockedHeap;
    
    #[global_allocator]
    static ALLOCATOR: LockedHeap = LockedHeap::empty();
    
    pub const HEAP_START: usize = 0xFFFF_8000_0000_0000;
    pub const HEAP_SIZE: usize = 100 * 1024 * 1024; // 100 MB
    
    pub fn init() {
        use crate::memory::{Page, VirtAddr, PageTableFlags};
        
        // Map heap pages
        let mut mapper = unsafe { crate::memory::paging::Mapper::new() };
        let mut frame_allocator = unsafe {
            crate::memory::frame_allocator::BootFrameAllocator::init(
                &crate::BOOT_INFO.memory_map
            )
        };
        
        let page_range = {
            let heap_start = VirtAddr::new(HEAP_START as u64);
            let heap_end = VirtAddr::new((HEAP_START + HEAP_SIZE) as u64);
            let start_page = Page::containing_address(heap_start);
            let end_page = Page::containing_address(heap_end);
            Page::range(start_page, end_page)
        };
        
        for page in page_range {
            let frame = frame_allocator
                .allocate_frame()
                .expect("Failed to allocate frame for heap");
            let flags = PageTableFlags::PRESENT 
                | PageTableFlags::WRITABLE 
                | PageTableFlags::NO_EXECUTE;
            mapper.map_to(page, frame, flags, &mut frame_allocator)
                .expect("Failed to map heap page");
        }
        
        unsafe {
            ALLOCATOR.lock().init(HEAP_START, HEAP_SIZE);
        }
    }
}

#[alloc_error_handler]
fn alloc_error_handler(layout: alloc::alloc::Layout) -> ! {
    panic!("Allocation error: {:?}", layout)
}
```

## Stage 6: Memory Management

### Buddy Allocator Implementation

Create `kernel/src/memory/buddy.rs`:

```rust
//! Buddy allocator for physical memory management

use super::{PhysAddr, PhysFrame};
use alloc::vec::Vec;
use core::cmp::min;

const MAX_ORDER: usize = 11; // Up to 2^11 * 4KB = 8MB blocks

pub struct BuddyAllocator {
    free_lists: [Vec<PhysFrame>; MAX_ORDER + 1],
    base: PhysAddr,
    size: usize,
}

impl BuddyAllocator {
    pub fn new(base: PhysAddr, size: usize) -> Self {
        let mut allocator = Self {
            free_lists: Default::default(),
            base,
            size,
        };
        
        // Add all memory to free lists
        let mut current = base;
        let end = PhysAddr::new(base.as_u64() + size as u64);
        
        while current < end {
            let remaining = (end.as_u64() - current.as_u64()) as usize;
            let order = min(
                remaining.trailing_zeros() as usize,
                MAX_ORDER
            );
            let block_size = (1 << order) * PhysFrame::SIZE as usize;
            
            if remaining >= block_size {
                allocator.free_lists[order].push(
                    PhysFrame::containing_address(current)
                );
                current = PhysAddr::new(current.as_u64() + block_size as u64);
            }
        }
        
        allocator
    }
    
    pub fn allocate(&mut self, order: usize) -> Option<PhysFrame> {
        assert!(order <= MAX_ORDER);
        
        // Find a free block
        for current_order in order..=MAX_ORDER {
            if let Some(frame) = self.free_lists[current_order].pop() {
                // Split larger blocks if necessary
                for split_order in (order..current_order).rev() {
                    let buddy = PhysFrame::containing_address(
                        PhysAddr::new(
                            frame.start_address().as_u64() + 
                            (1 << split_order) * PhysFrame::SIZE
                        )
                    );
                    self.free_lists[split_order].push(buddy);
                }
                
                return Some(frame);
            }
        }
        
        None
    }
    
    pub fn deallocate(&mut self, frame: PhysFrame, order: usize) {
        assert!(order <= MAX_ORDER);
        
        let mut current_frame = frame;
        let mut current_order = order;
        
        // Try to merge with buddies
        while current_order < MAX_ORDER {
            let buddy_addr = PhysAddr::new(
                current_frame.start_address().as_u64() ^ 
                ((1 << current_order) * PhysFrame::SIZE)
            );
            let buddy_frame = PhysFrame::containing_address(buddy_addr);
            
            // Check if buddy is free
            let mut found_buddy = false;
            if let Some(pos) = self.free_lists[current_order]
                .iter()
                .position(|&f| f == buddy_frame) 
            {
                self.free_lists[current_order].swap_remove(pos);
                found_buddy = true;
            }
            
            if !found_buddy {
                break;
            }
            
            // Merge with buddy
            current_frame = PhysFrame::containing_address(
                PhysAddr::new(min(
                    current_frame.start_address().as_u64(),
                    buddy_frame.start_address().as_u64()
                ))
            );
            current_order += 1;
        }
        
        self.free_lists[current_order].push(current_frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_buddy_allocator() {
        let base = PhysAddr::new(0x100000);
        let size = 16 * 1024 * 1024; // 16MB
        let mut allocator = BuddyAllocator::new(base, size);
        
        // Allocate some frames
        let frame1 = allocator.allocate(0).unwrap();
        let frame2 = allocator.allocate(1).unwrap();
        let frame3 = allocator.allocate(2).unwrap();
        
        // Deallocate and check merging
        allocator.deallocate(frame1, 0);
        allocator.deallocate(frame2, 1);
        allocator.deallocate(frame3, 2);
    }
}
```

## Stage 7: Interrupt Handling

### IDT Setup

Create `kernel/src/interrupts.rs`:

```rust
//! Interrupt handling

use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin::Mutex;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: Mutex<ChainedPics> = Mutex::new(unsafe {
    ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET)
});

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        
        // Exceptions
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.double_fault.set_handler_fn(double_fault_handler);
        
        // Hardware interrupts
        idt[InterruptIndex::Timer.as_usize()]
            .set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_usize()]
            .set_handler_fn(keyboard_interrupt_handler);
        
        idt
    };
}

pub fn init() {
    IDT.load();
    unsafe {
        PICS.lock().initialize();
    }
    x86_64::instructions::interrupts::enable();
    println!("Interrupts enabled");
}

extern "x86-interrupt" fn breakpoint_handler(
    stack_frame: InterruptStackFrame
) {
    println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    use x86_64::registers::control::Cr2;
    
    println!("EXCEPTION: PAGE FAULT");
    println!("Accessed Address: {:?}", Cr2::read());
    println!("Error Code: {:?}", error_code);
    println!("{:#?}", stack_frame);
    
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn timer_interrupt_handler(
    _stack_frame: InterruptStackFrame
) {
    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }
}

extern "x86-interrupt" fn keyboard_interrupt_handler(
    _stack_frame: InterruptStackFrame
) {
    use x86_64::instructions::port::Port;
    
    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };
    
    println!("Keyboard scancode: {}", scancode);
    
    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}
```

## Stage 8: Multitasking Foundation

### Task Structure

Create `kernel/src/task/mod.rs`:

```rust
//! Task management

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

pub mod context;
pub mod scheduler;

static NEXT_PID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProcessId(u64);

impl ProcessId {
    fn new() -> Self {
        Self(NEXT_PID.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,
    Terminated,
}

pub struct Task {
    pub id: ProcessId,
    pub state: TaskState,
    pub context: context::TaskContext,
    pub kernel_stack: Vec<u8>,
}

impl Task {
    pub fn new(entry_point: VirtAddr) -> Self {
        const KERNEL_STACK_SIZE: usize = 4096 * 5; // 20KB
        
        let mut kernel_stack = vec![0u8; KERNEL_STACK_SIZE];
        let stack_top = VirtAddr::new(
            kernel_stack.as_ptr() as u64 + KERNEL_STACK_SIZE as u64
        );
        
        Self {
            id: ProcessId::new(),
            state: TaskState::Ready,
            context: context::TaskContext::new(entry_point, stack_top),
            kernel_stack,
        }
    }
}
```

### Context Switching

Create `kernel/src/task/context.rs`:

```rust
//! Task context for context switching

use crate::memory::VirtAddr;
use core::arch::asm;

#[derive(Debug, Clone)]
#[repr(C)]
pub struct TaskContext {
    // Callee-saved registers
    pub rbp: u64,
    pub rbx: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rsp: u64,
    pub rip: u64,
}

impl TaskContext {
    pub fn new(entry_point: VirtAddr, stack_top: VirtAddr) -> Self {
        Self {
            rbp: 0,
            rbx: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rsp: stack_top.as_u64(),
            rip: entry_point.as_u64(),
        }
    }
}

/// Switch from current context to new context
#[naked]
pub unsafe extern "C" fn switch_context(
    current: *mut TaskContext,
    next: *const TaskContext,
) {
    asm!(
        // Save current context
        "mov [rdi + 0x00], rbp",
        "mov [rdi + 0x08], rbx",
        "mov [rdi + 0x10], r12",
        "mov [rdi + 0x18], r13",
        "mov [rdi + 0x20], r14",
        "mov [rdi + 0x28], r15",
        "mov [rdi + 0x30], rsp",
        "lea rax, [rip + 1f]",
        "mov [rdi + 0x38], rax",
        
        // Load next context
        "mov rbp, [rsi + 0x00]",
        "mov rbx, [rsi + 0x08]",
        "mov r12, [rsi + 0x10]",
        "mov r13, [rsi + 0x18]",
        "mov r14, [rsi + 0x20]",
        "mov r15, [rsi + 0x28]",
        "mov rsp, [rsi + 0x30]",
        "mov rax, [rsi + 0x38]",
        "jmp rax",
        
        "1:",
        "ret",
        options(noreturn)
    );
}
```

### Simple Scheduler

Create `kernel/src/task/scheduler.rs`:

```rust
//! Simple round-robin scheduler

use super::{Task, TaskState, ProcessId};
use alloc::collections::VecDeque;
use spin::Mutex;
use lazy_static::lazy_static;

lazy_static! {
    static ref SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::new());
}

pub struct Scheduler {
    ready_queue: VecDeque<Task>,
    current_task: Option<Task>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
            current_task: None,
        }
    }
    
    pub fn add_task(&mut self, task: Task) {
        self.ready_queue.push_back(task);
    }
    
    pub fn schedule(&mut self) -> Option<Task> {
        // Save current task if running
        if let Some(mut current) = self.current_task.take() {
            if current.state == TaskState::Running {
                current.state = TaskState::Ready;
                self.ready_queue.push_back(current);
            }
        }
        
        // Get next task
        self.ready_queue.pop_front().map(|mut task| {
            task.state = TaskState::Running;
            self.current_task = Some(task.clone());
            task
        })
    }
}

pub fn spawn(entry_point: VirtAddr) -> ProcessId {
    let task = Task::new(entry_point);
    let id = task.id;
    SCHEDULER.lock().add_task(task);
    id
}

pub fn yield_now() {
    // TODO: Implement actual context switching
    x86_64::instructions::interrupts::disable();
    
    let mut scheduler = SCHEDULER.lock();
    if let Some(next_task) = scheduler.schedule() {
        // Perform context switch
        drop(scheduler);
        unsafe {
            // switch_context implementation needed
        }
    }
    
    x86_64::instructions::interrupts::enable();
}
```

## Stage 9: System Call Interface

### System Call Handler

Create `kernel/src/syscall/mod.rs`:

```rust
//! System call interface

use core::arch::asm;

#[derive(Debug, Clone, Copy)]
#[repr(u64)]
pub enum Syscall {
    Exit = 0,
    Write = 1,
    Read = 2,
    Open = 3,
    Close = 4,
    Yield = 5,
    Spawn = 6,
}

/// System call handler entry point
#[no_mangle]
pub extern "C" fn syscall_handler(
    syscall: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
) -> i64 {
    match syscall {
        0 => sys_exit(arg1 as i32),
        1 => sys_write(arg1, arg2 as *const u8, arg3),
        2 => sys_read(arg1, arg2 as *mut u8, arg3),
        5 => sys_yield(),
        6 => sys_spawn(arg1 as *const u8, arg2),
        _ => -1, // ENOSYS
    }
}

fn sys_exit(code: i32) -> i64 {
    println!("Process exiting with code: {}", code);
    // TODO: Actually terminate process
    loop {
        x86_64::instructions::hlt();
    }
}

fn sys_write(fd: u64, buf: *const u8, count: u64) -> i64 {
    if fd != 1 && fd != 2 {
        return -1; // EBADF
    }
    
    let slice = unsafe {
        core::slice::from_raw_parts(buf, count as usize)
    };
    
    if let Ok(s) = core::str::from_utf8(slice) {
        print!("{}", s);
        count as i64
    } else {
        -1 // EINVAL
    }
}

fn sys_read(_fd: u64, _buf: *mut u8, _count: u64) -> i64 {
    // TODO: Implement
    -1
}

fn sys_yield() -> i64 {
    crate::task::scheduler::yield_now();
    0
}

fn sys_spawn(path: *const u8, len: u64) -> i64 {
    // TODO: Implement process spawning
    -1
}

/// Enable syscall/sysret instructions
pub fn init() {
    use x86_64::registers::model_specific::{Efer, EferFlags, Star, LStar};
    use x86_64::VirtAddr;
    
    unsafe {
        // Enable syscall/sysret
        Efer::update(|flags| {
            *flags |= EferFlags::SYSTEM_CALL_EXTENSIONS;
        });
        
        // Set up syscall handler
        LStar::write(VirtAddr::new(syscall_entry as u64));
        
        // Set up segments for syscall/sysret
        Star::write(0x0013_0008_0000_0000).unwrap();
    }
    
    println!("System calls enabled");
}

/// Low-level syscall entry point
#[naked]
extern "C" fn syscall_entry() {
    unsafe {
        asm!(
            // Save user stack
            "mov gs:8, rsp",
            // Switch to kernel stack
            "mov rsp, gs:0",
            
            // Save registers
            "push rcx",  // User RIP
            "push r11",  // User RFLAGS
            "push rbp",
            "push rbx",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            
            // Call handler
            "mov rdi, rax",  // Syscall number
            "mov rsi, rdi",  // arg1
            "mov rdx, rsi",  // arg2
            "mov rcx, rdx",  // arg3
            "mov r8, r10",   // arg4
            "mov r9, r8",    // arg5
            "call syscall_handler",
            
            // Restore registers
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop rbx",
            "pop rbp",
            "pop r11",   // User RFLAGS
            "pop rcx",   // User RIP
            
            // Restore user stack
            "mov rsp, gs:8",
            
            // Return to user mode
            "sysretq",
            options(noreturn)
        );
    }
}
```

## Stage 10: Initial Capability System

### Capability Implementation

Create `kernel/src/capability/mod.rs`:

```rust
//! Capability-based security system

use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::RwLock;

static NEXT_CAP_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapabilityId(u64);

impl CapabilityId {
    fn new() -> Self {
        Self(NEXT_CAP_ID.fetch_add(1, Ordering::Relaxed))
    }
}

bitflags::bitflags! {
    pub struct CapabilityRights: u64 {
        const READ      = 0b0000_0001;
        const WRITE     = 0b0000_0010;
        const EXECUTE   = 0b0000_0100;
        const DELETE    = 0b0000_1000;
        const GRANT     = 0b0001_0000;
        const REVOKE    = 0b0010_0000;
    }
}

#[derive(Debug, Clone)]
pub struct Capability {
    id: CapabilityId,
    object_id: ObjectId,
    rights: CapabilityRights,
    badge: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(u64);

pub struct CapabilityTable {
    capabilities: RwLock<BTreeMap<CapabilityId, Capability>>,
}

impl CapabilityTable {
    pub fn new() -> Self {
        Self {
            capabilities: RwLock::new(BTreeMap::new()),
        }
    }
    
    pub fn create_capability(
        &self,
        object_id: ObjectId,
        rights: CapabilityRights,
    ) -> CapabilityId {
        let cap = Capability {
            id: CapabilityId::new(),
            object_id,
            rights,
            badge: 0,
        };
        
        let id = cap.id;
        self.capabilities.write().insert(id, cap);
        id
    }
    
    pub fn lookup(&self, id: CapabilityId) -> Option<Capability> {
        self.capabilities.read().get(&id).cloned()
    }
    
    pub fn derive(
        &self,
        parent_id: CapabilityId,
        new_rights: CapabilityRights,
        badge: u64,
    ) -> Option<CapabilityId> {
        let caps = self.capabilities.read();
        let parent = caps.get(&parent_id)?;
        
        // Check that new rights are subset of parent
        if !new_rights.is_subset(parent.rights) {
            return None;
        }
        
        drop(caps);
        
        let derived = Capability {
            id: CapabilityId::new(),
            object_id: parent.object_id,
            rights: new_rights,
            badge,
        };
        
        let id = derived.id;
        self.capabilities.write().insert(id, derived);
        Some(id)
    }
    
    pub fn revoke(&self, id: CapabilityId) -> bool {
        self.capabilities.write().remove(&id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_capability_creation() {
        let table = CapabilityTable::new();
        let object_id = ObjectId(1);
        let rights = CapabilityRights::READ | CapabilityRights::WRITE;
        
        let cap_id = table.create_capability(object_id, rights);
        let cap = table.lookup(cap_id).unwrap();
        
        assert_eq!(cap.object_id, object_id);
        assert_eq!(cap.rights, rights);
    }
    
    #[test]
    fn test_capability_derivation() {
        let table = CapabilityTable::new();
        let object_id = ObjectId(1);
        let parent_rights = CapabilityRights::all();
        
        let parent_id = table.create_capability(object_id, parent_rights);
        let child_rights = CapabilityRights::READ;
        let child_id = table.derive(parent_id, child_rights, 42).unwrap();
        
        let child = table.lookup(child_id).unwrap();
        assert_eq!(child.rights, child_rights);
        assert_eq!(child.badge, 42);
    }
}
```

## Testing Each Stage

### QEMU Test Script

Create `scripts/test.sh`:

```bash
#!/bin/bash

# Build kernel
cargo build --release

# Create disk image
dd if=/dev/zero of=disk.img bs=1M count=64
mkfs.fat -F 32 disk.img

# Copy kernel to disk
mkdir -p mnt
sudo mount disk.img mnt
sudo cp target/x86_64-veridian/release/kernel mnt/kernel.elf
sudo umount mnt

# Run in QEMU
qemu-system-x86_64 \
    -drive if=pflash,format=raw,file=/usr/share/OVMF/OVMF_CODE.fd,readonly=on \
    -drive if=pflash,format=raw,file=/usr/share/OVMF/OVMF_VARS.fd \
    -drive format=raw,file=disk.img \
    -serial stdio \
    -m 512M \
    -d int,cpu_reset \
    -no-reboot \
    -no-shutdown
```

### Unit Test Framework

```rust
// Add to kernel/src/main.rs

#[cfg(test)]
pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;
    
    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}

#[test_case]
fn test_breakpoint_exception() {
    x86_64::instructions::interrupts::int3();
}
```

## Debugging Techniques

### GDB Integration

Create `.gdbinit`:

```gdb
# Connect to QEMU
target remote :1234

# Load symbols
symbol-file target/x86_64-veridian/release/kernel

# Useful commands
define print-page-table
    set $pml4 = $cr3 & ~0xfff
    set $i = 0
    while $i < 512
        set $entry = *(unsigned long*)($pml4 + $i * 8)
        if $entry & 1
            printf "PML4[%d] = 0x%lx\n", $i, $entry
        end
        set $i = $i + 1
    end
end

# Breakpoints
break _start
break kernel_init
break panic
```

### Debug Macros

```rust
/// Debug print macro
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        $crate::serial_println!("[DEBUG] {}", format_args!($($arg)*));
    };
}

/// Kernel assertion
#[macro_export]
macro_rules! kernel_assert {
    ($cond:expr) => {
        if !$cond {
            panic!("Assertion failed: {}", stringify!($cond));
        }
    };
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            panic!("Assertion failed: {}: {}", 
                   stringify!($cond), format_args!($($arg)*));
        }
    };
}
```

## Common Pitfalls

### 1. Stack Alignment Issues

**Problem**: x86_64 requires 16-byte stack alignment for function calls.

```rust
// Bad: Misaligned stack
unsafe {
    asm!("sub rsp, 15");  // Misaligns stack
}

// Good: Maintain alignment
unsafe {
    asm!("sub rsp, 16");  // Keeps alignment
}
```

### 2. Identity Mapping Removal

**Problem**: Removing identity mapping too early causes triple fault.

```rust
// Ensure higher-half is mapped before removing identity mapping
fn remove_identity_mapping(mapper: &mut Mapper) {
    // First ensure kernel is accessible at higher half
    ensure_higher_half_mapped(mapper);
    
    // Then remove identity mapping
    for page in 0..512 {
        let virt = VirtAddr::new(page * Page::SIZE);
        mapper.unmap(Page::containing_address(virt))
            .expect("Failed to unmap")
            .1.flush();
    }
}
```

### 3. Interrupt Safety

**Problem**: Race conditions with interrupt handlers.

```rust
// Bad: Not interrupt-safe
static mut COUNTER: u64 = 0;
pub fn increment() {
    unsafe { COUNTER += 1; }  // Race condition!
}

// Good: Interrupt-safe
use core::sync::atomic::{AtomicU64, Ordering};
static COUNTER: AtomicU64 = AtomicU64::new(0);
pub fn increment() {
    COUNTER.fetch_add(1, Ordering::Relaxed);
}
```

### 4. Page Fault Loops

**Problem**: Page fault handler causes another page fault.

```rust
// Ensure page fault handler doesn't access unmapped memory
extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    // Don't use println! if VGA buffer might be unmapped
    unsafe {
        serial_println!("Page fault at {:?}", Cr2::read());
    }
    
    // Halt instead of panic to avoid double fault
    loop {
        x86_64::instructions::hlt();
    }
}
```

### 5. Null Pointer Dereferences

**Problem**: Accessing address 0x0 in kernel space.

```rust
// Ensure first page is never mapped
fn setup_page_tables(mapper: &mut Mapper) {
    // Never map the zero page
    const ZERO_PAGE: Page = Page::containing_address(VirtAddr::new(0));
    // This ensures null pointer dereferences always fault
}
```

## Next Steps

### Immediate Priorities

1. **Implement Heap Allocator**
   - Linked list allocator
   - Slab allocator for common sizes
   - Thread-safe allocation

2. **File System Support**
   - VFS layer
   - Initial RAM disk
   - Simple file system implementation

3. **Userspace Support**
   - User mode switching
   - ELF loader
   - System call improvements

4. **Driver Framework**
   - PCI enumeration
   - USB stack basics
   - Network card driver

### Architecture Expansion

1. **Multi-Core Support**
   - AP processor startup
   - Per-CPU data structures
   - IPI implementation

2. **ACPI Support**
   - ACPI table parsing
   - Power management
   - Device enumeration

3. **64-bit Time**
   - High-resolution timers
   - Monotonic clock
   - Time keeping

### Advanced Features Roadmap

1. **Month 1-2: Core Stability**
   - Bug fixes and testing
   - Performance profiling
   - Documentation

2. **Month 3-4: Essential Services**
   - Networking stack
   - Storage drivers
   - Basic shell

3. **Month 5-6: Advanced Features**
   - Graphics support
   - Sound subsystem
   - Advanced scheduling

4. **Month 7+: Production Features**
   - Security hardening
   - Cloud integration
   - Container support

## Conclusion

This bootstrap guide provides a solid foundation for building Veridian OS. Each stage builds incrementally on the previous one, allowing you to test and verify functionality at each step.

### Key Takeaways

1. **Start Simple**: Get a minimal kernel booting first
2. **Test Early**: Verify each component before moving on
3. **Debug Tools**: Invest in good debugging infrastructure
4. **Safety First**: Use Rust's type system to prevent bugs
5. **Document Everything**: Future you will thank present you

### Resources for Continued Learning

- [OSDev Wiki](https://wiki.osdev.org/): Comprehensive OS development resource
- [Phil Opp's Blog](https://os.phil-opp.com/): Excellent Rust OS tutorial
- [Intel/AMD Manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html): Essential reference
- [Rust Embedded Book](https://docs.rust-embedded.org/book/): Low-level Rust programming

### Community and Support

- Join the Veridian OS Discord for real-time help
- Post questions on the GitHub Discussions
- Share your progress and get feedback
- Contribute back improvements

Remember: OS development is a marathon, not a sprint. Take breaks, celebrate small victories, and enjoy the journey of building something from scratch!

Happy kernel hacking! 🚀
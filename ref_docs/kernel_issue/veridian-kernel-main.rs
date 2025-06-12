// VeridianOS Kernel Main Entry Point
//
// This is where the kernel begins execution after the assembly bootstrap
// code has set up the initial environment. At this point:
// - We're running in 64-bit long mode
// - Paging is enabled with identity and higher-half mappings
// - We're executing at virtual address 0xFFFFFFFF80000000+
// - Stack is set up and ready for use

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(const_mut_refs)]

use core::panic::PanicInfo;

// External assembly symbols from boot.s
extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
    static __bss_start: u8;
    static __bss_end: u8;
    static __kernel_end: u8;
}

/// Kernel entry point called from assembly
///
/// # Arguments
/// * `multiboot_info_addr` - Virtual address of Multiboot2 information structure
///
/// # Safety
/// This function is called exactly once by the bootstrap assembly code
/// with a valid Multiboot2 info pointer.
#[no_mangle]
pub unsafe extern "C" fn kernel_main(multiboot_info_addr: usize) -> ! {
    // Initialize early console for debugging output
    // Note: VGA buffer is at 0xB8000, but we need to map it to virtual memory
    let vga_buffer = 0xFFFFFFFF800B8000 as *mut u16;
    
    // Clear screen and print boot message
    for i in 0..80 * 25 {
        vga_buffer.add(i).write_volatile(0x0F20); // White space on black
    }
    
    // Print "VeridianOS" at top of screen
    let msg = b"VeridianOS Kernel v0.2.0 - Booting...";
    for (i, &byte) in msg.iter().enumerate() {
        vga_buffer.add(i).write_volatile(0x0F00 | byte as u16);
    }
    
    // Print kernel memory layout information
    print_kernel_info(vga_buffer);
    
    // Verify we're running at the correct address
    let kernel_addr = kernel_main as *const () as usize;
    if kernel_addr < 0xFFFFFFFF80000000 {
        panic!("Kernel not running in higher half!");
    }
    
    // Initialize kernel subsystems
    init_kernel_subsystems(multiboot_info_addr);
    
    // Main kernel loop
    loop {
        // TODO: Implement scheduler and run tasks
        x86_64::instructions::hlt();
    }
}

/// Print kernel memory layout information for debugging
fn print_kernel_info(vga_buffer: *mut u16) {
    unsafe {
        // Helper to print hex number at position
        fn print_hex(vga_buffer: *mut u16, row: usize, col: usize, label: &[u8], value: usize) {
            let pos = row * 80 + col;
            
            // Print label
            for (i, &byte) in label.iter().enumerate() {
                vga_buffer.add(pos + i).write_volatile(0x0A00 | byte as u16);
            }
            
            // Print hex value
            let hex_chars = b"0123456789ABCDEF";
            let value_start = pos + label.len() + 2;
            vga_buffer.add(value_start).write_volatile(0x0F00 | b'0' as u16);
            vga_buffer.add(value_start + 1).write_volatile(0x0F00 | b'x' as u16);
            
            for i in 0..16 {
                let nibble = (value >> (60 - i * 4)) & 0xF;
                let ch = hex_chars[nibble];
                vga_buffer.add(value_start + 2 + i).write_volatile(0x0F00 | ch as u16);
            }
        }
        
        // Print section addresses
        print_hex(vga_buffer, 2, 0, b".text:  ", &__text_start as *const _ as usize);
        print_hex(vga_buffer, 3, 0, b".rodata:", &__rodata_start as *const _ as usize);
        print_hex(vga_buffer, 4, 0, b".data:  ", &__data_start as *const _ as usize);
        print_hex(vga_buffer, 5, 0, b".bss:   ", &__bss_start as *const _ as usize);
        print_hex(vga_buffer, 6, 0, b"kernel_end:", &__kernel_end as *const _ as usize);
    }
}

/// Initialize kernel subsystems
fn init_kernel_subsystems(multiboot_info_addr: usize) {
    // TODO: Parse Multiboot2 information
    // TODO: Initialize memory manager
    // TODO: Set up interrupt handlers
    // TODO: Initialize device drivers
    // TODO: Start scheduler
    
    // For now, just verify the multiboot magic
    unsafe {
        let magic = *(multiboot_info_addr as *const u32);
        if magic != 0x36d76289 {
            panic!("Invalid Multiboot2 magic!");
        }
    }
}

/// Panic handler for kernel panics
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Print panic message to VGA buffer
    let vga_buffer = 0xFFFFFFFF800B8000 as *mut u16;
    let panic_msg = b"KERNEL PANIC: ";
    
    unsafe {
        // Red background for panic
        let panic_row = 24;
        for i in 0..80 {
            vga_buffer.add(panic_row * 80 + i).write_volatile(0x4F20);
        }
        
        // Print panic message
        for (i, &byte) in panic_msg.iter().enumerate() {
            vga_buffer.add(panic_row * 80 + i).write_volatile(0x4F00 | byte as u16);
        }
        
        // Print location if available
        if let Some(location) = info.location() {
            let file_bytes = location.file().as_bytes();
            let start_col = panic_msg.len() + 1;
            
            // Print filename (truncated if necessary)
            let max_len = core::cmp::min(file_bytes.len(), 40);
            for (i, &byte) in file_bytes[..max_len].iter().enumerate() {
                vga_buffer.add(panic_row * 80 + start_col + i)
                    .write_volatile(0x4F00 | byte as u16);
            }
        }
    }
    
    // Halt the CPU
    loop {
        x86_64::instructions::cli();
        x86_64::instructions::hlt();
    }
}

// Module declarations
mod arch;
mod memory;
mod process;
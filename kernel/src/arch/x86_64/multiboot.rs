// Multiboot2 header for GRUB compatibility

use core::arch::global_asm;

// Simplified multiboot2 header without problematic relocations
global_asm!(
    r#"
.section .multiboot_header, "aw"
.align 8

multiboot_header_start:
    .long 0xe85250d6                // magic number (multiboot2)
    .long 0                         // architecture (0 = i386/x86_64)
    .long multiboot_header_end - multiboot_header_start  // header length

    // checksum
    .long -(0xe85250d6 + 0 + (multiboot_header_end - multiboot_header_start))

    // Information request tag
    .word 1                         // type
    .word 0                         // flags
    .long 20                        // size
    .long 4                         // basic memory info
    .long 6                         // memory map

    // End tag
    .word 0                         // type
    .word 0                         // flags  
    .long 8                         // size
multiboot_header_end:

.section .text.boot, "ax"
.global _multiboot_entry
_multiboot_entry:
    // Set up stack
    mov rsp, 0xFFFFFFFF80200000     // Use higher-half stack
    
    // Save multiboot info
    push rdi                        // multiboot2 magic
    push rsi                        // multiboot2 info struct
    
    // Enable SSE for Rust
    mov rax, cr0
    and ax, 0xFFFB                  // Clear coprocessor emulation CR0.EM
    or ax, 0x0002                   // Set coprocessor monitoring  CR0.MP
    mov cr0, rax
    mov rax, cr4
    or ax, 3 << 9                   // Set CR4.OSFXSR and CR4.OSXMMEXCPT
    mov cr4, rax

    // Call our multiboot main
    pop rsi                         // multiboot info
    pop rdi                         // multiboot magic  
    call multiboot_main
    
.halt_loop:
    hlt
    jmp .halt_loop
"#
);

#[no_mangle]
pub extern "C" fn multiboot_main(magic: u64, info_addr: u64) -> ! {
    // SAFETY: 0xb8000 is the standard VGA text buffer address on x86 PCs.
    // write_volatile ensures the write is not optimized away. The buffer is
    // always mapped during early boot and each u16 encodes a character + attribute.
    unsafe {
        let vga = 0xb8000 as *mut u16;
        vga.write_volatile(0x0F4D); // 'M' for multiboot
        vga.offset(1).write_volatile(0x0F42); // 'B'
    }

    // Verify multiboot2 magic
    if magic != 0x36d76289 {
        // SAFETY: VGA text buffer at 0xb8000 is always mapped during early boot.
        // write_volatile ensures the error indicator is displayed.
        unsafe {
            let vga = 0xb8000 as *mut u16;
            vga.offset(2).write_volatile(0x4F45); // 'E' in red (error)
            vga.offset(3).write_volatile(0x4F52); // 'R'
        }
        loop {
            // SAFETY: hlt halts the CPU until the next interrupt. Safe in a panic loop.
            unsafe { core::arch::asm!("hlt") };
        }
    }

    // Initialize early serial for debugging
    let mut serial_port = crate::arch::x86_64::serial_init();
    use core::fmt::Write;
    let _ = writeln!(serial_port, "[MULTIBOOT] Multiboot2 entry successful!");
    let _ = writeln!(
        serial_port,
        "[MULTIBOOT] Magic: 0x{:x}, Info: 0x{:x}",
        magic, info_addr
    );

    // Set up minimal boot info structure
    // For now, we'll skip the full multiboot info parsing and use defaults
    // SAFETY: BOOT_INFO is a static mut Option written once during early boot
    // before any other code runs. No concurrent access is possible at this stage.
    unsafe {
        crate::arch::x86_64::boot::BOOT_INFO = None; // Multiboot doesn't use
                                                     // bootloader_api BootInfo
    }

    // Initialize early architecture
    crate::arch::x86_64::entry::arch_early_init();

    let _ = writeln!(
        serial_port,
        "[MULTIBOOT] Starting bootstrap initialization..."
    );

    // Run bootstrap directly
    crate::bootstrap::run();
}

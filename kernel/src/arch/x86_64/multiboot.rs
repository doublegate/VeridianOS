// Multiboot header for GRUB compatibility

use core::arch::global_asm;

// Multiboot2 header
global_asm!(
    r#"
.section .multiboot_header, "aw"
.align 8

multiboot_header_start:
    .long 0xe85250d6                // magic number
    .long 0                         // architecture (0 = i386)
    .long multiboot_header_end - multiboot_header_start  // header length

    // checksum
    .long -(0xe85250d6 + 0 + (multiboot_header_end - multiboot_header_start))

    // End tag
    .word 0                         // type
    .word 0                         // flags  
    .long 8                         // size
multiboot_header_end:
"#
);

#[no_mangle]
pub extern "C" fn multiboot_main(magic: u32, info_addr: u32) -> ! {
    // Verify multiboot magic
    if magic != 0x36d76289 {
        panic!("Invalid multiboot magic: 0x{:x}", magic);
    }
    
    // Call our normal kernel entry
    crate::kernel_main_impl()
}
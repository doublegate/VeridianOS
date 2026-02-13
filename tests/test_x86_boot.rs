#![no_std]
#![no_main]

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;

entry_point!(test_main);

fn test_main(_boot_info: &'static BootInfo) -> ! {
    // Write to VGA buffer
    let vga = 0xb8000 as *mut u16;
    unsafe {
        vga.offset(0).write_volatile(0x0f48); // 'H'
        vga.offset(1).write_volatile(0x0f45); // 'E'
        vga.offset(2).write_volatile(0x0f4c); // 'L'
        vga.offset(3).write_volatile(0x0f4c); // 'L'
        vga.offset(4).write_volatile(0x0f4f); // 'O'
    }
    
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
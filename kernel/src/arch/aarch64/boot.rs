// AArch64 boot code - matches x86_64 pattern

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

#[no_mangle]
#[link_section = ".text.boot"]
pub extern "C" fn _start_rust() -> ! {
    // BSS symbols from linker script
    extern "C" {
        static mut __bss_start: u8;
        static mut __bss_end: u8;
    }

    unsafe {
        // Clear BSS first
        let bss_start = &raw const __bss_start as *mut u8;
        let bss_end = &raw const __bss_end as *mut u8;
        let bss_size = bss_end as usize - bss_start as usize;
        core::ptr::write_bytes(bss_start, 0, bss_size);

        // Write startup message
        let uart = 0x0900_0000 as *mut u8;
        core::ptr::write_volatile(uart, b'R');
        core::ptr::write_volatile(uart, b'U');
        core::ptr::write_volatile(uart, b'S');
        core::ptr::write_volatile(uart, b'T');
        core::ptr::write_volatile(uart, b'\n');

        // Call kernel_main with proper ABI
        crate::kernel_main()
    }
}

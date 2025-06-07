// Boot entry point for AArch64

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

#[no_mangle]
pub extern "C" fn _start_rust() -> ! {
    // Write 'D' to show we reached Rust code
    unsafe {
        let uart = 0x0900_0000 as *mut u8;
        core::ptr::write_volatile(uart, b'D');
    }

    // Jump to the main kernel entry
    crate::kernel_main()
}

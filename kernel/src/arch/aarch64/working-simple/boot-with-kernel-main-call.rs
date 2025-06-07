// AArch64 boot code - matches x86_64 pattern

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

// Forward declaration for kernel_main
extern "C" {
    fn kernel_main() -> !;
}

#[no_mangle]
pub unsafe extern "C" fn _start_rust() -> ! {
    // Write startup message
    let uart = 0x0900_0000 as *mut u8;
    *uart = b'R';
    *uart = b'U';
    *uart = b'S';
    *uart = b'T';
    *uart = b'\n';
    
    // Call kernel_main
    kernel_main()
}
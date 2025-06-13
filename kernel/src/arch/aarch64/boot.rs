// AArch64 boot code - matches x86_64 pattern

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

#[no_mangle]
pub unsafe extern "C" fn _start_rust() -> ! {
    // Write startup message
    let uart = 0x0900_0000 as *mut u8;
    *uart = b'R';
    *uart = b'U';
    *uart = b'S';
    *uart = b'T';
    *uart = b'\n';

    // Write test message before kernel_main
    *uart = b'T';
    *uart = b'E';
    *uart = b'S';
    *uart = b'T';
    *uart = b'\n';

    // Try calling kernel_main through a pointer to see if it's a linking issue
    *uart = b'C';
    *uart = b'A';
    *uart = b'L';
    *uart = b'L';
    *uart = b'\n';

    // Direct call to kernel_main
    crate::kernel_main()
}

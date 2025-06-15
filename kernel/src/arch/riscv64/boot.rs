// Boot entry point for RISC-V 64

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

#[no_mangle]
pub extern "C" fn _start_rust() -> ! {
    // Call the kernel main function from main.rs
    extern "C" {
        fn kernel_main() -> !;
    }
    unsafe { kernel_main() }
}

// Boot entry point for RISC-V 64

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

#[no_mangle]
pub extern "C" fn _start_rust() -> ! {
    // Very early debug output - try without waiting for LSR
    unsafe {
        // Direct write to UART at 0x10000000
        let uart_base = 0x1000_0000 as *mut u8;
        
        // Just write directly without checking LSR first
        // QEMU's UART should be ready immediately
        uart_base.write_volatile(b'B');
        uart_base.write_volatile(b'O');
        uart_base.write_volatile(b'O');
        uart_base.write_volatile(b'T');
        uart_base.write_volatile(b'\n');
    }
    
    // Call the kernel main function from main.rs
    extern "C" {
        fn kernel_main() -> !;
    }
    unsafe { kernel_main() }
}

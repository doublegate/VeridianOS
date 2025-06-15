// AArch64 boot code - matches x86_64 pattern

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

/// Entry point from assembly code
///
/// # Safety
///
/// This function is called from assembly with:
/// - Stack properly initialized
/// - BSS section cleared
/// - Running in EL1 with MMU disabled
#[no_mangle]
#[link_section = ".text.boot"]
pub unsafe extern "C" fn _start_rust() -> ! {
    // PL011 UART base address for QEMU virt machine
    let uart_base = 0x0900_0000_usize;
    // For QEMU's PL011, we can write directly to the data register at offset 0
    let uart_dr = uart_base as *mut u8;

    // Write startup message
    *uart_dr = b'R';
    *uart_dr = b'U';
    *uart_dr = b'S';
    *uart_dr = b'T';
    *uart_dr = b'\n';

    // Write pre-kernel_main marker
    *uart_dr = b'P';
    *uart_dr = b'R';
    *uart_dr = b'E';
    *uart_dr = b'\n';

    // Call kernel_main from main.rs
    extern "C" {
        fn kernel_main() -> !;
    }
    kernel_main()
}

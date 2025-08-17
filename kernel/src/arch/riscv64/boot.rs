// Boot entry point for RISC-V 64

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

#[no_mangle]
pub extern "C" fn _start_rust() -> ! {
    // Use SBI console output to show we reached Rust code
    unsafe {
        // SBI console putchar
        sbi_putchar(b'B');
        sbi_putchar(b'O');
        sbi_putchar(b'O');
        sbi_putchar(b'T');
        sbi_putchar(b'\n');
    }
    
    // Call the kernel main function from main.rs
    extern "C" {
        fn kernel_main() -> !;
    }
    unsafe { kernel_main() }
}

/// SBI console putchar using ecall
#[inline]
unsafe fn sbi_putchar(ch: u8) {
    core::arch::asm!(
        "ecall",
        in("a0") ch as usize,     // Character to print
        in("a7") 0x01usize,       // SBI function ID 0x01 = console_putchar (legacy)
        options(nostack, nomem)
    );
}

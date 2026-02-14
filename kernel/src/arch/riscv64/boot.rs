//! RISC-V 64 boot entry point.
//!
//! Includes the assembly startup code (`boot.S`) and the Rust `_start_rust`
//! entry that prints an early banner via SBI console putchar and calls
//! `kernel_main`.

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

#[no_mangle]
pub extern "C" fn _start_rust() -> ! {
    // SAFETY: sbi_putchar invokes the SBI legacy console putchar (ecall with
    // a7=0x01). Used for early boot output before any Rust infrastructure is
    // available. Always safe to call from supervisor mode.
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
    // SAFETY: kernel_main is an extern "C" function defined in main.rs that
    // performs the full kernel initialization. It is called exactly once after
    // early boot setup is complete.
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

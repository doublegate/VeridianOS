//! RISC-V 64 boot entry point.
//!
//! Includes the assembly startup code (`boot.S`) and the Rust `_start_rust`
//! entry that prints an early banner via SBI console putchar and calls
//! `kernel_main`.

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

extern "C" {
    /// Boot hart id saved by boot.S from `a0`.
    static veridian_boot_hartid: u64;
}

/// Hardware id of the boot hart.
pub fn boot_hartid() -> u64 {
    // SAFETY: written once by boot.S before any Rust code runs; read-only
    // afterwards.
    unsafe { core::ptr::addr_of!(veridian_boot_hartid).read_volatile() }
}

#[no_mangle]
pub extern "C" fn _start_rust() -> ! {
    // The boot hart is logical CPU 0; `tp` now points at its per-CPU block.
    // SAFETY: first Rust code on the boot hart, before anything reads `tp`.
    unsafe { crate::arch::percpu::install(0, boot_hartid() as u32) };

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

    // Arm boot-stack overflow detection before anything deep runs.
    crate::arch::stack_canary::install();

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
///
/// # Safety
///
/// Must run in S-mode under SBI firmware (OpenSBI) that implements the
/// legacy console_putchar extension.
#[inline]
unsafe fn sbi_putchar(ch: u8) {
    // SAFETY: the SBI legacy console_putchar call (a7 = 1) only asks the
    // firmware to print one character; it touches no kernel memory. SBI
    // firmware is present per this function's contract.
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a0") ch as usize,     // Character to print
            in("a7") 0x01usize,       // SBI function ID 0x01 = console_putchar (legacy)
            options(nostack, nomem)
        )
    };
}

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
    /// The kernel's satp, with the paging mode boot.S found.
    static veridian_kernel_satp: u64;
}

/// satp MODE values (privileged spec, "satp").
pub const SATP_MODE_SV39: u64 = 8;
pub const SATP_MODE_SV48: u64 = 9;

/// The paging mode boot.S turned on: [`SATP_MODE_SV48`] if the hart has
/// Sv48, else [`SATP_MODE_SV39`] (ADR 0018). Every address space uses it.
pub fn satp_mode() -> u64 {
    // SAFETY: written once by boot.S before any Rust code runs; read-only
    // afterwards.
    unsafe { core::ptr::addr_of!(veridian_kernel_satp).read_volatile() >> 60 }
}

/// Page-table levels of the paging mode: 3 under Sv39, 4 under Sv48.
pub fn paging_levels() -> u8 {
    if satp_mode() == SATP_MODE_SV39 {
        3
    } else {
        4
    }
}

/// The kernel image's link address minus its load address (`link.ld`):
/// boot.S maps the image at `physical + KERNEL_OFFSET` (E-RV3).
pub const KERNEL_OFFSET: u64 = 0xFFFF_FFFF_0000_0000;

/// Where boot.S maps physical memory (1 GiB pages, [`PHYS_MAP_SIZE`]):
/// virtual = physical + PHYS_MAP_BASE. Inside Sv39's kernel half, so the
/// same layout serves Sv39 and Sv48 (ADR 0018).
pub const PHYS_MAP_BASE: u64 = 0xFFFF_FFC0_0000_0000;

/// How much physical memory the map covers: physical addresses at or above
/// it are not used (the device tree's memory is clamped to it).
pub const PHYS_MAP_SIZE: u64 = 128 << 30;

/// The physical address of a kernel-image address (code, statics).
pub fn kernel_phys(va: u64) -> u64 {
    va - KERNEL_OFFSET
}

/// Hardware id of the boot hart.
pub fn boot_hartid() -> u64 {
    // SAFETY: written once by boot.S before any Rust code runs; read-only
    // afterwards.
    unsafe { core::ptr::addr_of!(veridian_boot_hartid).read_volatile() }
}

/// Rust entry of a secondary hart (from `veridian_hart_entry` in boot.S).
/// Defined in every build because boot.S references it; harts are only
/// started with the `smp` feature.
#[no_mangle]
extern "C" fn veridian_secondary_rust(
    hartid: usize,
    args: &'static crate::arch::smp_boot::ApBootArgs,
) -> ! {
    let cpu = args.cpu_id.load(core::sync::atomic::Ordering::Acquire) as usize;
    // SAFETY: first Rust code on this hart; `cpu` is the logical id the
    // boot hart assigned it, and nothing else touches that block now.
    unsafe { crate::arch::percpu::install(cpu, hartid as u32) };
    #[cfg(feature = "smp")]
    crate::arch::smp_boot::ap_main();
    #[cfg(not(feature = "smp"))]
    loop {
        // SAFETY: waits for an interrupt; supervisor interrupts are off.
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)) };
    }
}

#[no_mangle]
pub extern "C" fn _start_rust() -> ! {
    // boot.S turned paging on: physical memory is reached through its map
    // from here on (`mm::phys_to_virt_addr`).
    crate::mm::set_phys_mem_offset(PHYS_MAP_BASE);
    // The user limit follows the mode boot.S turned on (ADR 0018).
    crate::mm::user_layout::set_riscv_paging_levels(u32::from(paging_levels()));

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
pub(super) unsafe fn sbi_putchar(ch: u8) {
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

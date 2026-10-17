//! AArch64 boot entry point.
//!
//! Includes the assembly startup code (`boot.S`) and the Rust `_start_rust`
//! entry that prints an early banner via direct UART and calls `kernel_main`.

use core::arch::global_asm;

// Include the assembly boot code
global_asm!(include_str!("boot.S"));

extern "C" {
    /// The device tree pointer boot.S saved from x0 (0 if none was passed).
    static veridian_dtb_pa: u64;
    /// The kernel's virtual minus physical address, measured by boot.S.
    static veridian_kernel_offset: u64;
}

/// Where boot.S maps all physical memory (ADR 0015): `mm::PHYS_MEM_OFFSET`.
pub const PHYS_MAP_BASE: u64 = 0xFFFF_8000_0000_0000;

/// How much physical memory the map covers (512 1 GiB blocks): physical
/// addresses at or above it are not used.
pub const PHYS_MAP_SIZE: u64 = 512 << 30;

/// The physical address of a kernel-image address (code, statics, linker
/// symbols). The offset is the one boot.S measured, so this holds wherever
/// the image was loaded.
pub fn kernel_phys(va: u64) -> u64 {
    // SAFETY: written once by boot.S before any Rust code runs; read-only
    // afterwards.
    va - unsafe { core::ptr::addr_of!(veridian_kernel_offset).read_volatile() }
}

/// Where QEMU puts the device tree for a bare-metal ELF on `virt`, which it
/// does not pass in x0.
const QEMU_VIRT_DTB_PA: u64 = 0x4000_0000;

/// VBAR_EL1 as `_start_rust` found it (written once, before anything else).
static ENTRY_VBAR: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Whether the exception vectors were already installed when Rust code
/// started, so that a fault in the earliest Rust code is reported.
pub fn vectors_installed_at_entry() -> bool {
    ENTRY_VBAR.load(core::sync::atomic::Ordering::Relaxed) == super::exceptions::vectors_base()
}

/// The device tree pointer boot.S saved from x0.
fn passed_dtb_pa() -> u64 {
    // SAFETY: written once by boot.S before any Rust code runs; read-only
    // afterwards.
    unsafe { core::ptr::addr_of!(veridian_dtb_pa).read_volatile() }
}

/// Whether firmware passed the device tree in x0 (the arm64 Image boot
/// protocol), rather than the bare-ELF fallback being used.
pub fn dtb_passed_in_x0() -> bool {
    passed_dtb_pa() != 0
}

/// The device tree's physical address: the one firmware passed in x0, or
/// QEMU `virt`'s fixed location for a bare ELF (HA-01).
pub fn dtb_pa() -> u64 {
    match passed_dtb_pa() {
        0 => QEMU_VIRT_DTB_PA,
        pa => pa,
    }
}

/// Rust entry of a secondary CPU (from `veridian_secondary_entry` in
/// boot.S, which already set the stack and TPIDR_EL1). Defined in every
/// build because boot.S references it; CPUs are only started with the
/// `smp` feature.
#[no_mangle]
extern "C" fn veridian_secondary_rust(args: &'static crate::arch::smp_boot::ApBootArgs) -> ! {
    let cpu = args.cpu_id.load(core::sync::atomic::Ordering::Acquire) as usize;
    let mpidr: u64;
    // SAFETY: reading MPIDR_EL1 at EL1 has no side effects.
    unsafe { core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr, options(nomem, nostack)) };
    // SAFETY: first Rust code on this CPU; `cpu` is the logical id the boot
    // CPU assigned it, and nothing else touches that block now.
    unsafe { crate::arch::percpu::install(cpu, (mpidr & 0x00FF_FFFF) as u32) };
    #[cfg(feature = "smp")]
    crate::arch::smp_boot::ap_main();
    #[cfg(not(feature = "smp"))]
    loop {
        // SAFETY: waits for an event; interrupts are masked.
        unsafe { core::arch::asm!("wfe", options(nomem, nostack)) };
    }
}

/// Entry point from assembly code
///
/// # Safety
///
/// This function is called from assembly with:
/// - Stack properly initialized
/// - BSS section cleared
/// - Running in EL1 with the MMU on, at the link address (ADR 0015)
#[no_mangle]
#[link_section = ".text.boot"]
pub unsafe extern "C" fn _start_rust() -> ! {
    // First: where exceptions vectored when Rust was entered, for the
    // early-vectors boot test.
    let vbar: u64;
    // SAFETY: reading VBAR_EL1 has no side effects.
    unsafe { core::arch::asm!("mrs {0}, vbar_el1", out(reg) vbar, options(nomem, nostack)) };
    ENTRY_VBAR.store(vbar, core::sync::atomic::Ordering::Relaxed);

    // boot.S turned the MMU on: physical memory is reached through its map
    // from here on (`mm::phys_to_virt_addr`).
    crate::mm::set_phys_mem_offset(PHYS_MAP_BASE);

    // Use direct_uart for proper string output
    use crate::arch::aarch64::direct_uart::uart_write_str;

    // Write startup messages
    // SAFETY: boot.S mapped the PL011 UART of the QEMU virt machine this
    // boot path targets as Device memory in the physical-memory map
    // (`platform::PL011`).
    unsafe {
        uart_write_str("[BOOT] AArch64 Rust entry point reached\n");
        uart_write_str("[BOOT] Stack initialized and BSS cleared\n");
        uart_write_str("[BOOT] Preparing to enter kernel_main...\n");
    }

    // The boot CPU is logical CPU 0; TPIDR_EL1 now points at its per-CPU
    // block. Its hardware id is the MPIDR affinity (Aff0..Aff2).
    let mpidr: u64;
    // SAFETY: reading MPIDR_EL1 at EL1 has no side effects.
    unsafe { core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr, options(nomem, nostack)) };
    // SAFETY: first Rust code on the boot CPU, before anything reads
    // TPIDR_EL1.
    unsafe { crate::arch::percpu::install(0, (mpidr & 0x00FF_FFFF) as u32) };

    // Arm boot-stack overflow detection before anything deep runs.
    crate::arch::stack_canary::install();

    // Call kernel_main from main.rs
    extern "C" {
        fn kernel_main() -> !;
    }
    // SAFETY: kernel_main is the kernel's `extern "C" fn() -> !` entry point
    // defined in main.rs; it takes no arguments and expects exactly the
    // state this function's contract provides (stack set up, BSS cleared,
    // EL1).
    unsafe { kernel_main() }
}

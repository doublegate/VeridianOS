//! AArch64 exception vectors (VBAR_EL1).
//!
//! Before this table existed VBAR_EL1 was never written, so any exception
//! -- a data abort, an undefined instruction, an interrupt -- vectored to
//! address 0.
//!
//! - IRQ taken at EL1 with SP_EL1 (the kernel's mode): the interrupted context
//!   is saved -- x0-x18, x29, x30, ELR/SPSR -- the GIC is serviced, and the
//!   context returns. The FP/SIMD registers are not saved: the kernel is built
//!   soft-float (`aarch64-unknown-none-softfloat`, N-178), so no kernel code
//!   uses them, and user FP state is switched with the task (as Linux arm64
//!   does), not saved on every exception.
//! - Everything else (synchronous exceptions, FIQ, SError, any exception from
//!   EL0 or taken on SP_EL0) is a kernel bug or an unsupported path: it is
//!   reported with ESR/ELR/FAR on the UART and the system stops.

use core::arch::global_asm;

/// Saved-context layout (192 bytes, 16-byte aligned): x0-x18 and x29 at
/// 0..160, x30 and ELR at 160, SPSR at 176, padding to 192.
const _FRAME_SIZE: usize = 192;

global_asm!(
    r#"
.macro VERIDIAN_SAVE_CONTEXT
    sub sp, sp, #192
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x29, [sp, #144]
    mrs x0, elr_el1
    stp x30, x0, [sp, #160]
    mrs x1, spsr_el1
    str x1, [sp, #176]
.endm

.macro VERIDIAN_RESTORE_CONTEXT
    ldp x30, x0, [sp, #160]
    msr elr_el1, x0
    ldr x1, [sp, #176]
    msr spsr_el1, x1
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x29, [sp, #144]
    ldp x0, x1, [sp, #0]
    add sp, sp, #192
.endm

.macro VERIDIAN_FATAL_VECTOR kind
    .balign 128
    mov x0, #\kind
    b veridian_aarch64_fatal_entry
.endm

.section .text
.balign 2048
.global veridian_aarch64_vectors
veridian_aarch64_vectors:
    // Current EL, SP_EL0: sync, IRQ, FIQ, SError
    VERIDIAN_FATAL_VECTOR 0
    VERIDIAN_FATAL_VECTOR 1
    VERIDIAN_FATAL_VECTOR 2
    VERIDIAN_FATAL_VECTOR 3
    // Current EL, SP_ELx: sync
    VERIDIAN_FATAL_VECTOR 4
    // Current EL, SP_ELx: IRQ
    .balign 128
    b veridian_aarch64_irq_entry
    // Current EL, SP_ELx: FIQ, SError
    VERIDIAN_FATAL_VECTOR 6
    VERIDIAN_FATAL_VECTOR 7
    // Lower EL, AArch64: sync, IRQ, FIQ, SError
    VERIDIAN_FATAL_VECTOR 8
    VERIDIAN_FATAL_VECTOR 9
    VERIDIAN_FATAL_VECTOR 10
    VERIDIAN_FATAL_VECTOR 11
    // Lower EL, AArch32: sync, IRQ, FIQ, SError
    VERIDIAN_FATAL_VECTOR 12
    VERIDIAN_FATAL_VECTOR 13
    VERIDIAN_FATAL_VECTOR 14
    VERIDIAN_FATAL_VECTOR 15

veridian_aarch64_irq_entry:
    VERIDIAN_SAVE_CONTEXT
    bl {irq}
    VERIDIAN_RESTORE_CONTEXT
    eret

veridian_aarch64_fatal_entry:
    bl {fatal}
1:  wfe
    b 1b
"#,
    irq = sym aarch64_irq,
    fatal = sym aarch64_fatal,
);

extern "C" {
    static veridian_aarch64_vectors: u8;
}

/// GIC interrupt ID of the EL1 virtual timer (PPI 11 = INTID 27).
pub const VIRTUAL_TIMER_INTID: u32 = 27;

/// The vector table's address. boot.S writes it to VBAR_EL1 on every CPU
/// as soon as the CPU has a stack, before any Rust code runs.
pub fn vectors_base() -> u64 {
    core::ptr::addr_of!(veridian_aarch64_vectors) as u64
}

/// IRQ handler: acknowledge and dispatch everything pending at the GIC.
/// Bounded so a stuck level-triggered source cannot hold the CPU here.
extern "C" fn aarch64_irq() {
    for _ in 0..16 {
        let Some(intid) = super::gic::acknowledge_raw() else {
            return;
        };
        if intid == VIRTUAL_TIMER_INTID {
            super::timer::handle_interrupt();
        }
        super::gic::end_of_interrupt_raw(intid);
    }
}

/// Report an unhandled exception and stop.
extern "C" fn aarch64_fatal(kind: u64) -> ! {
    let (esr, elr, far): (u64, u64, u64);
    // SAFETY: reading the EL1 syndrome registers has no side effects.
    unsafe {
        core::arch::asm!("mrs {0}, esr_el1", "mrs {1}, elr_el1", "mrs {2}, far_el1",
            out(reg) esr, out(reg) elr, out(reg) far, options(nomem, nostack));
    }
    const KINDS: [&str; 16] = [
        "sync (SP_EL0)",
        "IRQ (SP_EL0)",
        "FIQ (SP_EL0)",
        "SError (SP_EL0)",
        "sync",
        "IRQ",
        "FIQ",
        "SError",
        "sync from EL0",
        "IRQ from EL0",
        "FIQ from EL0",
        "SError from EL0",
        "sync from AArch32",
        "IRQ from AArch32",
        "FIQ from AArch32",
        "SError from AArch32",
    ];
    // Write to the PL011 directly: no console lock in a fatal exception.
    let _ = core::fmt::Write::write_fmt(
        &mut super::direct_uart::writer(),
        format_args!(
            "\n[EXCEPTION] unhandled {} ESR={:#x} ELR={:#x} FAR={:#x}\n",
            KINDS.get(kind as usize).copied().unwrap_or("?"),
            esr,
            elr,
            far
        ),
    );
    panic!("unhandled AArch64 exception");
}

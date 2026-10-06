//! AArch64 exception vectors (VBAR_EL1).
//!
//! Before this table existed VBAR_EL1 was never written, so any exception
//! -- a data abort, an undefined instruction, an interrupt -- vectored to
//! address 0.
//!
//! - IRQ taken at EL1 with SP_EL1 (the kernel's mode): the interrupted context
//!   is saved -- x0-x18, x29, x30, ELR/SPSR, and all of q0-q31 with FPCR/FPSR,
//!   because the kernel runs with FP/SIMD enabled and LLVM uses the vector
//!   registers -- the GIC is serviced, and the context returns.
//! - Everything else (synchronous exceptions, FIQ, SError, any exception from
//!   EL0 or taken on SP_EL0) is a kernel bug or an unsupported path: it is
//!   reported with ESR/ELR/FAR on the UART and the system stops.

use core::arch::global_asm;

/// Saved-context layout (720 bytes, 16-byte aligned): x0-x18 and x29 at
/// 0..160, x30 and ELR at 160, SPSR at 176, FPCR/FPSR at 184, padding,
/// q0-q31 at 208..720. X-register pairs must stay within the +504 offset
/// range of STP/LDP.
const _FRAME_SIZE: usize = 720;

global_asm!(
    r#"
.macro VERIDIAN_SAVE_CONTEXT
    sub sp, sp, #720
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
    stp q0, q1, [sp, #208]
    stp q2, q3, [sp, #240]
    stp q4, q5, [sp, #272]
    stp q6, q7, [sp, #304]
    stp q8, q9, [sp, #336]
    stp q10, q11, [sp, #368]
    stp q12, q13, [sp, #400]
    stp q14, q15, [sp, #432]
    stp q16, q17, [sp, #464]
    stp q18, q19, [sp, #496]
    stp q20, q21, [sp, #528]
    stp q22, q23, [sp, #560]
    stp q24, q25, [sp, #592]
    stp q26, q27, [sp, #624]
    stp q28, q29, [sp, #656]
    stp q30, q31, [sp, #688]
    mrs x0, fpcr
    mrs x1, fpsr
    stp x0, x1, [sp, #184]
.endm

.macro VERIDIAN_RESTORE_CONTEXT
    ldp x0, x1, [sp, #184]
    msr fpcr, x0
    msr fpsr, x1
    ldp q0, q1, [sp, #208]
    ldp q2, q3, [sp, #240]
    ldp q4, q5, [sp, #272]
    ldp q6, q7, [sp, #304]
    ldp q8, q9, [sp, #336]
    ldp q10, q11, [sp, #368]
    ldp q12, q13, [sp, #400]
    ldp q14, q15, [sp, #432]
    ldp q16, q17, [sp, #464]
    ldp q18, q19, [sp, #496]
    ldp q20, q21, [sp, #528]
    ldp q22, q23, [sp, #560]
    ldp q24, q25, [sp, #592]
    ldp q26, q27, [sp, #624]
    ldp q28, q29, [sp, #656]
    ldp q30, q31, [sp, #688]
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
    add sp, sp, #720
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

/// Install the vector table. Must run before interrupts are unmasked.
pub fn install() {
    let base = core::ptr::addr_of!(veridian_aarch64_vectors) as u64;
    // SAFETY: the table is 2 KiB aligned and holds the 16 architected
    // entries; writing VBAR_EL1 only changes where exceptions vector.
    unsafe {
        core::arch::asm!("msr vbar_el1, {0}", "isb", in(reg) base, options(nostack));
    }
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
    // println! is a no-op on AArch64; write to the PL011 directly.
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

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
//! - A synchronous exception at EL1 with SP_EL1: a fault in a user copy
//!   (`usercopy.rs`) is offered to demand paging and otherwise resumes at the
//!   copy's fixup (EFAULT); anything else is a kernel bug, reported.
//! - Synchronous exceptions and IRQs from EL0 (ADR 0016) land on the running
//!   task's kernel stack (SP_EL1, left at its top by the return to EL0), saved
//!   whole as a `switch::UserFrame`: system calls (`svc`), page faults, IRQs;
//!   anything else ends the process with a signal.
//! - Everything else (FIQ, SError, exceptions taken on SP_EL0, AArch32) is a
//!   kernel bug or an unsupported path: it is reported with ESR/ELR/FAR on the
//!   UART and the system stops.

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
    .balign 128
    b veridian_aarch64_kernel_sync
    // Current EL, SP_ELx: IRQ
    .balign 128
    b veridian_aarch64_irq_entry
    // Current EL, SP_ELx: FIQ, SError
    VERIDIAN_FATAL_VECTOR 6
    VERIDIAN_FATAL_VECTOR 7
    // Lower EL, AArch64: sync, IRQ, FIQ, SError
    .balign 128
    sub sp, sp, #272
    stp x0, x1, [sp, #0]
    mov x1, #8
    b veridian_aarch64_user_entry
    .balign 128
    sub sp, sp, #272
    stp x0, x1, [sp, #0]
    mov x1, #9
    b veridian_aarch64_user_entry
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

veridian_aarch64_kernel_sync:
    VERIDIAN_SAVE_CONTEXT
    mov x0, sp
    bl {ksync}
    VERIDIAN_RESTORE_CONTEXT
    eret

// From EL0: x0 and x1 are saved and x1 holds the vector (8 sync, 9 IRQ);
// sp is the UserFrame on the task's kernel stack.
veridian_aarch64_user_entry:
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x2, sp_el0
    mrs x3, elr_el1
    stp x2, x3, [sp, #248]
    mrs x2, spsr_el1
    str x2, [sp, #264]
    // The per-CPU block (TPIDR_EL1) records the frame of the exception in
    // progress (`syscall_frame`, offset 0x10).
    mrs x2, tpidr_el1
    mov x3, sp
    str x3, [x2, #0x10]
    mov x29, #0
    mov x0, sp
    bl {user}
    // Fall through: return to EL0 with the frame at sp.
.global veridian_aarch64_return_to_user
veridian_aarch64_return_to_user:
    ldp x2, x3, [sp, #248]
    msr sp_el0, x2
    msr elr_el1, x3
    ldr x2, [sp, #264]
    msr spsr_el1, x2
    ldp x0, x1, [sp, #0]
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x19, [sp, #144]
    ldp x20, x21, [sp, #160]
    ldp x22, x23, [sp, #176]
    ldp x24, x25, [sp, #192]
    ldp x26, x27, [sp, #208]
    ldp x28, x29, [sp, #224]
    ldr x30, [sp, #240]
    add sp, sp, #272
    eret

veridian_aarch64_fatal_entry:
    bl {fatal}
1:  wfe
    b 1b
"#,
    irq = sym aarch64_irq,
    fatal = sym aarch64_fatal,
    ksync = sym aarch64_kernel_sync,
    user = sym aarch64_user_exception,
);

/// Exception classes (ESR_EL1.EC) the handlers act on.
const EC_UNKNOWN: u64 = 0x00;
const EC_FP_ACCESS: u64 = 0x07;
const EC_ILLEGAL_STATE: u64 = 0x0E;
const EC_SVC64: u64 = 0x15;
const EC_SYSREG: u64 = 0x18;
const EC_IABORT_LOWER: u64 = 0x20;
const EC_PC_ALIGN: u64 = 0x22;
const EC_DABORT_LOWER: u64 = 0x24;
const EC_DABORT_SAME: u64 = 0x25;
const EC_SP_ALIGN: u64 = 0x26;
const EC_BRK: u64 = 0x3C;

fn syndrome() -> (u64, u64, u64) {
    let (esr, elr, far): (u64, u64, u64);
    // SAFETY: reading the EL1 syndrome registers has no side effects.
    unsafe {
        core::arch::asm!("mrs {0}, esr_el1", "mrs {1}, elr_el1", "mrs {2}, far_el1",
            out(reg) esr, out(reg) elr, out(reg) far, options(nomem, nostack));
    }
    (esr, elr, far)
}

/// A synchronous exception at EL1. A data abort in a user copy is offered
/// to demand paging (the page may just not be backed yet) and otherwise
/// resumes at the copy's fixup, which returns EFAULT; anything else is a
/// kernel bug. `frame` is the save area (`VERIDIAN_SAVE_CONTEXT`): ELR at
/// offset 168.
extern "C" fn aarch64_kernel_sync(frame: *mut u64) {
    let (esr, elr, far) = syndrome();
    if esr >> 26 == EC_DABORT_SAME {
        if let Some(fixup) = super::usercopy::fixup_for(elr as usize) {
            let info = crate::mm::page_fault::from_aarch64(esr, far, elr);
            if !crate::mm::page_fault::resolve_user_copy_fault(&info) {
                // SAFETY: `frame` is this exception's save area; word 21
                // (offset 168) is the ELR it restores.
                unsafe { frame.add(21).write(fixup as u64) };
            }
            return;
        }
    }
    aarch64_fatal(4)
}

/// An exception from EL0, on the task's kernel stack with its registers in
/// `f`: `vector` 8 for a synchronous exception, 9 for an IRQ. Returns to
/// `veridian_aarch64_return_to_user`.
extern "C" fn aarch64_user_exception(f: &mut super::switch::UserFrame, vector: u64) {
    use super::switch;
    use crate::process::signals::*;
    // The first argument of the system call this return ends, for
    // SA_RESTART (the return value overwrites x0).
    let mut syscall = None;
    if vector == 9 {
        aarch64_irq();
    } else {
        let (esr, elr, far) = syndrome();
        match esr >> 26 {
            EC_SVC64 => {
                // ELR is already past the svc. The call runs with IRQs on,
                // as on x86_64 and RISC-V.
                switch::irq_enable();
                let nr = f.x[switch::UserFrame::X8] as usize;
                // Never restart rt_sigreturn: the context it restored is
                // already the one to return to.
                if nr != RT_SIGRETURN {
                    syscall = Some(f.x[0]);
                }
                let a = |i: usize| f.x[i] as usize;
                let ret = crate::syscall::aarch64_syscall(nr, a(0), a(1), a(2), a(3), a(4));
                f.x[switch::UserFrame::X0] = ret as u64;
            }
            EC_IABORT_LOWER | EC_DABORT_LOWER => {
                switch::irq_enable();
                let info = crate::mm::page_fault::from_aarch64(esr, far, elr);
                let code = if info.reason == crate::mm::page_fault::PageFaultReason::NotPresent {
                    SEGV_MAPERR
                } else {
                    SEGV_ACCERR
                };
                if crate::mm::page_fault::handle_page_fault(info).is_err() {
                    fault(SIGSEGV, code, far, elr, f.x[30]);
                }
            }
            EC_UNKNOWN | EC_FP_ACCESS | EC_ILLEGAL_STATE | EC_SYSREG => {
                fault(SIGILL, ILL_ILLOPC, elr, elr, f.x[30])
            }
            EC_BRK => fault(SIGTRAP, TRAP_BRKPT, elr, elr, f.x[30]),
            EC_PC_ALIGN => fault(SIGBUS, BUS_ADRALN, elr, elr, f.x[30]),
            EC_SP_ALIGN => fault(SIGBUS, BUS_ADRALN, f.sp, elr, f.x[30]),
            _ => fault(SIGBUS, SI_KERNEL, 0, elr, f.x[30]),
        }
    }
    exit_to_user(f, syscall);
}

/// arm64's rt_sigreturn number.
const RT_SIGRETURN: usize = 139;
const SIGILL: usize = 4;
const SIGTRAP: usize = 5;
const SIGBUS: usize = 7;
const SIGSEGV: usize = 11;

/// A user fault's signal: queued for its handler if the thread has one
/// that does not block it, else the process ends (Linux forces the
/// default action for a blocked or ignored synchronous signal) and the
/// fault is logged, as Linux's show_unhandled_signals does.
fn fault(sig: usize, code: i32, addr: u64, pc: u64, lr: u64) {
    if !crate::process::signals::queue_fault(crate::process::signals::SigInfo::fault(
        sig, code, addr,
    )) {
        crate::println!(
            "[FAULT] pid {}: unhandled signal {} (code {}) at {:#x}, pc {:#x}, lr {:#x}",
            crate::process::current_process().map_or(0, |p| p.pid.0),
            sig,
            code,
            addr,
            pc,
            lr
        );
        kill_user(sig as u32);
    }
}

/// Last step before every return to EL0: preemption, signals (fatal ones,
/// stops, handlers: `signals::deliver_on_return`), and a frame that can only
/// return to EL0 with interrupts unmasked. `syscall` is the first argument
/// of the system call this return ends, if it is one.
fn exit_to_user(f: &mut super::switch::UserFrame, syscall: Option<u64>) {
    use super::switch;
    switch::irq_enable();
    #[cfg(feature = "alloc")]
    crate::sched::dispatch::preempt_user();
    #[cfg(feature = "alloc")]
    crate::process::signals::deliver_on_return(f, syscall);
    #[cfg(not(feature = "alloc"))]
    {
        let _ = syscall;
        crate::process::user_return_check();
    }
    switch::irq_save();
    f.spsr = switch::user_spsr(f.spsr);
    if !crate::syscall::userspace::is_user_address(f.elr as usize) {
        kill_user(11);
    }
}

/// End the current process with `signal` (a fault it did not handle).
fn kill_user(signal: u32) -> ! {
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        let _ = crate::syscall::process::exit_current(0, signal);
    }
    panic!(
        "AArch64 user fault (signal {}) outside a dispatched thread",
        signal
    );
}

extern "C" {
    static veridian_aarch64_vectors: u8;
}

/// The SGI the dispatcher sends to wake another CPU (`switch::kick`).
pub const SCHED_WAKE_SGI: u32 = 1;

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
        let Some(iar) = super::gic::acknowledge_raw() else {
            return;
        };
        // An SGI (SCHED_WAKE_SGI, a dispatcher kick) needs no work here:
        // taking it ends the target's wait, and it reaches its next
        // scheduling point.
        if iar & 0x3FF == VIRTUAL_TIMER_INTID {
            super::timer::handle_interrupt();
        }
        super::gic::end_of_interrupt_raw(iar);
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

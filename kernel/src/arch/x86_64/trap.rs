//! Exception and interrupt entry (N-166 to N-169, N-175).
//!
//! Every vector enters through a 16-byte stub in one table
//! (`veridian_trap_stubs`, vector `v` at offset `16 * v`). The stub pushes
//! an error code of 0 if the CPU did not push one, then the vector number,
//! and jumps to [`trap_common`], which:
//!
//! 1. executes `swapgs` only if the interrupted context was ring 3, judged from
//!    the saved CS, followed by `lfence` on both paths so a mispredicted branch
//!    cannot run kernel code on the user GS base (CVE-2019-1125);
//! 2. pushes the general registers, completing a [`TrapFrame`] on the stack;
//! 3. calls [`trap_dispatch`] with the frame, which may modify it;
//! 4. restores the registers and returns with `iretq`, swapping GS back only
//!    when returning to ring 3.
//!
//! GS convention: whenever ring 0 runs, GS_BASE holds this CPU's per-CPU
//! block. Every entry from ring 3 (here, `syscall_entry`) swaps it in and
//! every exit to ring 3 swaps it out, exactly once each.
//!
//! Stacks: a vector from ring 3 runs on TSS.RSP0, which is kept equal to the
//! syscall entry stack (`percpu::set_entry_stack`); a vector from ring 0
//! runs on the stack it interrupted. Only #DF, NMI and #MC use IST stacks,
//! because they can arrive when the current stack is unusable (N-169). The
//! handlers for those three never rely on the GS base, so they are correct
//! even when they interrupt the instructions between a `swapgs` and the
//! `iretq`/`sysretq` that follows it.
//!
//! The syscall entry builds the same frame (`vector` = [`SYSCALL_VECTOR`],
//! `error_code` = the system call number), so both paths share one register
//! layout and one user-register sanitiser ([`sanitize_user_frame`]).

use core::{
    arch::{global_asm, naked_asm},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use super::idt::{raw_serial_hex, raw_serial_str};

/// Registers saved on kernel entry, lowest address first.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    /// Vector number, or [`SYSCALL_VECTOR`].
    pub vector: u64,
    /// The CPU's error code (0 if none), or the system call number.
    pub error_code: u64,
    // Hardware interrupt frame (built by `syscall_entry` for a syscall).
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// Size of [`TrapFrame`] in bytes; entry assembly relies on it.
pub const TRAP_FRAME_SIZE: usize = 22 * 8;

const _: () = {
    assert!(core::mem::size_of::<TrapFrame>() == TRAP_FRAME_SIZE);
    assert!(core::mem::offset_of!(TrapFrame, rax) == 14 * 8);
    assert!(core::mem::offset_of!(TrapFrame, vector) == 15 * 8);
    assert!(core::mem::offset_of!(TrapFrame, rip) == 17 * 8);
    assert!(core::mem::offset_of!(TrapFrame, cs) == 18 * 8);
    assert!(core::mem::offset_of!(TrapFrame, rflags) == 19 * 8);
    assert!(core::mem::offset_of!(TrapFrame, rsp) == 20 * 8);
    assert!(core::mem::offset_of!(TrapFrame, ss) == 21 * 8);
};

/// `TrapFrame::vector` of a frame built by `syscall_entry`.
pub const SYSCALL_VECTOR: u64 = u64::MAX;

/// User code and stack selectors (RPL 3).
pub const USER_CS: u64 = 0x33;
pub const USER_SS: u64 = 0x2B;

/// One past the highest user address.
pub const USER_END: u64 = 0x0000_8000_0000_0000;

/// RFLAGS bits user code may control: CF PF AF ZF SF TF DF OF AC ID (Linux
/// FIX_EFLAGS without RF and NT). IOPL and every system flag are excluded.
pub const USER_RFLAGS_MASK: u64 = 0x0004_0DD5 | (1 << 21);
/// Bits always set in user RFLAGS: reserved bit 1 and IF.
pub const USER_RFLAGS_FIXED: u64 = 0x202;

/// User RFLAGS from a value that may have come from user memory (sigreturn,
/// ptrace, a saved context): only the user-controllable bits survive, and
/// IF is forced on (N-170).
pub const fn sanitize_user_rflags(rflags: u64) -> u64 {
    (rflags & USER_RFLAGS_MASK) | USER_RFLAGS_FIXED
}

/// Whether `rip` may be loaded as a user instruction pointer: canonical and
/// below the kernel half.
pub const fn is_user_address(addr: u64) -> bool {
    addr < USER_END
}

/// Make a frame that is about to return to ring 3 safe to return with: the
/// user selectors, sanitised RFLAGS and a user RIP and RSP. A frame that
/// fails the address check has been corrupted by kernel code (user code
/// cannot load a non-user RIP); its RIP is replaced with 0 so the process
/// faults in user mode instead of the return faulting in ring 0. Returns
/// whether the frame was already valid.
pub fn sanitize_user_frame(f: &mut TrapFrame) -> bool {
    let mut ok = true;
    f.cs = USER_CS;
    f.ss = USER_SS;
    let flags = sanitize_user_rflags(f.rflags);
    ok &= flags == f.rflags;
    f.rflags = flags;
    if !is_user_address(f.rip) {
        f.rip = 0;
        ok = false;
    }
    if !is_user_address(f.rsp) {
        f.rsp = 0;
        ok = false;
    }
    ok
}

// The stub table. AT&T syntax so that `$symbol` is unambiguously an
// immediate inside `.rept`.
global_asm!(
    ".pushsection .text.veridian_trap_stubs,\"ax\",@progbits",
    ".p2align 4",
    ".globl veridian_trap_stubs",
    "veridian_trap_stubs:",
    ".macro VSTUB_NOERR vec",
    ".p2align 4",
    "pushq $0",
    "pushq $\\vec",
    "jmp {common}",
    ".endm",
    ".macro VSTUB_ERR vec",
    ".p2align 4",
    "pushq $\\vec",
    "jmp {common}",
    ".endm",
    // Exceptions. The CPU pushes an error code for 8, 10-14, 17, 21, 29, 30.
    "VSTUB_NOERR 0",
    "VSTUB_NOERR 1",
    "VSTUB_NOERR 2",
    "VSTUB_NOERR 3",
    "VSTUB_NOERR 4",
    "VSTUB_NOERR 5",
    "VSTUB_NOERR 6",
    "VSTUB_NOERR 7",
    "VSTUB_ERR 8",
    "VSTUB_NOERR 9",
    "VSTUB_ERR 10",
    "VSTUB_ERR 11",
    "VSTUB_ERR 12",
    "VSTUB_ERR 13",
    "VSTUB_ERR 14",
    "VSTUB_NOERR 15",
    "VSTUB_NOERR 16",
    "VSTUB_ERR 17",
    "VSTUB_NOERR 18",
    "VSTUB_NOERR 19",
    "VSTUB_NOERR 20",
    "VSTUB_ERR 21",
    "VSTUB_NOERR 22",
    "VSTUB_NOERR 23",
    "VSTUB_NOERR 24",
    "VSTUB_NOERR 25",
    "VSTUB_NOERR 26",
    "VSTUB_NOERR 27",
    "VSTUB_NOERR 28",
    "VSTUB_ERR 29",
    "VSTUB_ERR 30",
    "VSTUB_NOERR 31",
    // Interrupts 32-255.
    "vstub_vec = 32",
    ".rept 224",
    ".p2align 4",
    "pushq $0",
    "pushq $vstub_vec",
    "jmp {common}",
    "vstub_vec = vstub_vec + 1",
    ".endr",
    ".purgem VSTUB_NOERR",
    ".purgem VSTUB_ERR",
    ".popsection",
    common = sym trap_common,
    options(att_syntax),
);

extern "C" {
    static veridian_trap_stubs: u8;
}

/// Stride of the stub table.
const STUB_STRIDE: u64 = 16;

/// Vectors for which the CPU pushes an error code.
const fn has_error_code(vector: usize) -> bool {
    matches!(vector, 8 | 10..=14 | 17 | 21 | 29 | 30)
}

/// Common entry: see the module documentation.
#[unsafe(naked)]
unsafe extern "C" fn trap_common() {
    naked_asm!(
        // [rsp] vector, [rsp+8] error code, [rsp+16] rip, [rsp+24] cs
        "test byte ptr [rsp + 24], 3",
        "jz 2f",
        "swapgs",
        "2:",
        "lfence",
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rbp",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "cld",
        // The CPU aligned RSP to 16 before pushing its frame; with the 17
        // words pushed since, RSP is 16-byte aligned here, as a call needs.
        "mov rdi, rsp",
        "call {dispatch}",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        "add rsp, 16",
        // [rsp] rip, [rsp+8] cs
        "test byte ptr [rsp + 8], 3",
        "jz 3f",
        // No interrupt may arrive between the swapgs and the iretq: it
        // would see a ring-0 CS with the user GS base.
        "cli",
        "swapgs",
        "3:",
        "iretq",
        dispatch = sym trap_dispatch,
    );
}

/// Interrupts that reached `unexpected_vector`.
static UNEXPECTED_VECTORS: AtomicU64 = AtomicU64::new(0);

/// Rust side of every vector.
extern "C" fn trap_dispatch(f: &mut TrapFrame) {
    let user = f.cs & 3 == 3;
    match f.vector {
        2 => nmi(f),
        8 => double_fault(f),
        14 => page_fault(f, user),
        18 => machine_check(f),
        0..=31 => exception(f, user),
        32 => pic_timer(),
        33 => keyboard(),
        34..=47 => pic_other(f.vector),
        // The tick is charged as user time if it interrupted ring 3.
        48 => apic_timer(f.cs & 3 == 3),
        49 => {
            crate::mm::tlb::service_pending();
            super::apic::send_eoi();
        }
        50 => {
            // A remote CPU queued work here; waking from HLT is the point.
            crate::arch::percpu::note_ipi();
            super::apic::send_eoi();
        }
        // The local APIC's spurious vector: no EOI (SDM 3A 11.9).
        0xFF => {}
        _ => {
            UNEXPECTED_VECTORS.fetch_add(1, Ordering::Relaxed);
            super::apic::send_eoi();
        }
    }
    if user && !matches!(f.vector, 2 | 18) {
        exit_to_user(f);
    }
}

/// Last step before any return to ring 3 from an interrupt or exception:
/// a thread whose process received a fatal signal exits instead. Signal
/// delivery and rescheduling hook in here too (sprint D3).
fn exit_to_user(f: &mut TrapFrame) {
    // Timer preemption of user code (stage D3): an interrupt from ring 3
    // whose tick used up the task's slice switches here.
    #[cfg(feature = "alloc")]
    crate::sched::dispatch::preempt_user();
    crate::process::user_return_check();
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        crate::process::signals::deliver_on_return(f);
    }
    sanitize_user_frame(f);
}

/// A fault signal for a dispatched thread whose handler is installed and
/// not blocked is queued for that thread and delivered on the way back
/// (SIGSEGV handlers, SIGFPE handlers...). Returns false when the fault
/// must kill the process instead (default action, ignored, or blocked:
/// Linux forces the default for a blocked or ignored synchronous signal).
fn queue_fault_signal(sig: u32) -> bool {
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        if let (Some(process), Some(thread)) = (
            crate::process::current_process(),
            crate::process::current_thread(),
        ) {
            let sig = sig as usize;
            let bit = crate::process::signals::sig_bit(sig);
            let handler = process.get_signal_handler(sig).unwrap_or(0);
            let blocked = thread.sigmask.load(Ordering::Acquire) & bit != 0;
            if handler > 1 && !blocked {
                thread.sigpending.fetch_or(bit, Ordering::AcqRel);
                return true;
            }
        }
    }
    let _ = sig;
    false
}

/// Mark the faulting user process as killed by `signal` and return to the
/// context that launched it. Does not return.
///
/// `thread_only` keeps the other threads of the process alive (the page
/// fault path's existing behaviour for clone threads).
fn kill_user(signal: u32, thread_only: bool) -> ! {
    // A thread running as its own task leaves through the normal exit
    // path (stage D2), which tears its process down. The trap frame on its
    // kernel stack is simply abandoned.
    #[cfg(feature = "alloc")]
    if crate::sched::dispatch::current_owner().is_some() {
        let _ = crate::syscall::process::exit_current(0, signal);
    }
    if thread_only {
        if let Some(thread) = crate::process::current_thread() {
            thread.set_state(crate::process::thread::ThreadState::Zombie);
        } else if let Some(process) = crate::process::current_process() {
            process.set_term_signal(signal);
            process.set_state(crate::process::pcb::ProcessState::Zombie);
        }
    } else if let Some(process) = crate::process::current_process() {
        process.set_term_signal(signal);
        process.set_state(crate::process::pcb::ProcessState::Zombie);
    }
    if super::usermode::has_boot_return_context() {
        // SAFETY: ring 0 with interrupts disabled (interrupt gate), GS_BASE
        // holds the per-CPU block (trap_common swapped it in), and the boot
        // return context was just checked to exist.
        unsafe { super::usermode::boot_return_to_kernel() };
    }
    // A user context always has a launcher today (nested execution).
    fatal(b"user fault without a launch context", None);
}

/// Log a fatal kernel fault and stop this CPU. Uses only port I/O: the
/// interrupted code may hold any lock.
fn fatal(what: &[u8], f: Option<&TrapFrame>) -> ! {
    let cr2: u64;
    let cr3: u64;
    // SAFETY: reading control registers in ring 0 has no side effects.
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    }
    // SAFETY: COM1 is the kernel console on the supported platforms.
    unsafe {
        raw_serial_str(b"FATAL: ");
        raw_serial_str(what);
        if let Some(f) = f {
            raw_serial_str(b" vec=0x");
            raw_serial_hex(f.vector);
            raw_serial_str(b" err=0x");
            raw_serial_hex(f.error_code);
            raw_serial_str(b" rip=0x");
            raw_serial_hex(f.rip);
            raw_serial_str(b" cs=0x");
            raw_serial_hex(f.cs);
            raw_serial_str(b" rflags=0x");
            raw_serial_hex(f.rflags);
            raw_serial_str(b" rsp=0x");
            raw_serial_hex(f.rsp);
        }
        raw_serial_str(b" cr2=0x");
        raw_serial_hex(cr2);
        raw_serial_str(b" cr3=0x");
        raw_serial_hex(cr3);
        raw_serial_str(b"\n");
    }
    loop {
        x86_64::instructions::interrupts::disable();
        x86_64::instructions::hlt();
    }
}

/// Names of the exception vectors, for diagnostics.
fn exception_name(vector: u64) -> &'static [u8] {
    match vector {
        0 => b"#DE divide error",
        1 => b"#DB debug",
        3 => b"#BP breakpoint",
        4 => b"#OF overflow",
        5 => b"#BR bound range",
        6 => b"#UD invalid opcode",
        7 => b"#NM device not available",
        10 => b"#TS invalid TSS",
        11 => b"#NP segment not present",
        12 => b"#SS stack fault",
        13 => b"#GP general protection",
        16 => b"#MF x87 floating point",
        17 => b"#AC alignment check",
        19 => b"#XM SIMD floating point",
        20 => b"#VE virtualization",
        21 => b"#CP control protection",
        29 => b"#VC VMM communication",
        30 => b"#SX security",
        _ => b"reserved exception",
    }
}

/// Signal a user process receives for an exception (Linux `traps.c`).
fn exception_signal(vector: u64) -> u32 {
    const SIGILL: u32 = 4;
    const SIGTRAP: u32 = 5;
    const SIGBUS: u32 = 7;
    const SIGFPE: u32 = 8;
    const SIGSEGV: u32 = 11;
    match vector {
        0 | 16 | 19 => SIGFPE,
        1 | 3 => SIGTRAP,
        6 | 7 => SIGILL,
        11 | 12 | 17 => SIGBUS,
        _ => SIGSEGV,
    }
}

/// Every exception except #PF, NMI, #DF and #MC.
fn exception(f: &mut TrapFrame, user: bool) {
    if user {
        // SAFETY: COM1 is the kernel console on the supported platforms.
        unsafe {
            raw_serial_str(b"[TRAP] user ");
            raw_serial_str(exception_name(f.vector));
            raw_serial_str(b" rip=0x");
            raw_serial_hex(f.rip);
            raw_serial_str(b" err=0x");
            raw_serial_hex(f.error_code);
            raw_serial_str(b"\n");
        }
        if queue_fault_signal(exception_signal(f.vector)) {
            return;
        }
        kill_user(exception_signal(f.vector), false);
    }
    match f.vector {
        // A kernel breakpoint or debug trap (debugger use) resumes.
        1 | 3 => {
            // SAFETY: COM1 is the kernel console on the supported platforms.
            unsafe {
                raw_serial_str(b"[TRAP] kernel ");
                raw_serial_str(exception_name(f.vector));
                raw_serial_str(b" rip=0x");
                raw_serial_hex(f.rip);
                raw_serial_str(b"\n");
            }
        }
        _ => fatal(exception_name(f.vector), Some(f)),
    }
}

/// NMI (IST). Uses neither locks nor the GS base: it can interrupt any
/// instruction, including the entry and exit windows where GS holds the
/// user value.
fn nmi(_f: &mut TrapFrame) {
    // System control port B: bit 7 = memory parity / SERR, bit 6 = I/O
    // channel check (the legacy NMI reasons).
    // SAFETY: reading port 0x61 has no side effects.
    let reason: u8 = unsafe { x86_64::instructions::port::Port::new(0x61).read() };
    // SAFETY: COM1 is the kernel console on the supported platforms.
    unsafe {
        raw_serial_str(b"[NMI] reason=0x");
        raw_serial_hex(u64::from(reason));
        raw_serial_str(b"\n");
    }
}

/// Double fault (IST): the kernel cannot continue.
fn double_fault(f: &mut TrapFrame) -> ! {
    let cr2: u64;
    // SAFETY: reading CR2 has no side effects.
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags))
    };
    if crate::mm::kstack::is_guard_fault(cr2) || crate::mm::kstack::is_guard_fault(f.rsp) {
        fatal(b"#DF: kernel stack overflow (guard page)", Some(f));
    }
    fatal(b"#DF double fault", Some(f));
}

/// Machine check (IST). Logs the valid error banks and stops: nothing is
/// recoverable without a recovery policy.
fn machine_check(f: &mut TrapFrame) -> ! {
    if mca_supported() {
        // SAFETY: MCA is reported by CPUID, so these MSRs exist.
        unsafe {
            let msr = |n: u32| x86_64::registers::model_specific::Msr::new(n).read();
            let cap = msr(0x179);
            raw_serial_str(b"[MCE] MCG_STATUS=0x");
            raw_serial_hex(msr(0x17A));
            raw_serial_str(b"\n");
            for bank in 0..(cap & 0xFF) as u32 {
                let status = msr(0x401 + 4 * bank);
                if status >> 63 != 0 {
                    raw_serial_str(b"[MCE] bank ");
                    raw_serial_hex(u64::from(bank));
                    raw_serial_str(b" STATUS=0x");
                    raw_serial_hex(status);
                    raw_serial_str(b"\n");
                }
            }
        }
    }
    fatal(b"#MC machine check", Some(f));
}

/// Whether the CPU has the machine-check exception and architecture
/// (CPUID.01H:EDX bits 7 and 14).
fn mca_supported() -> bool {
    // SAFETY: CPUID leaf 1 is unprivileged and side-effect free.
    let edx = unsafe { core::arch::x86_64::__cpuid(1).edx };
    edx & (1 << 7) != 0 && edx & (1 << 14) != 0
}

/// Enable delivery of machine checks as #MC (CR4.MCE). Without it the CPU
/// shuts down on a machine check (HX-22).
pub fn enable_machine_check() {
    // SAFETY: CPUID leaf 1 is unprivileged and side-effect free.
    if unsafe { core::arch::x86_64::__cpuid(1).edx } & (1 << 7) == 0 {
        return;
    }
    use x86_64::registers::control::{Cr4, Cr4Flags};
    // SAFETY: setting CR4.MCE only changes how a machine check is reported;
    // the #MC handler is installed.
    unsafe { Cr4::update(|f| f.insert(Cr4Flags::MACHINE_CHECK_EXCEPTION)) };
}

/// Page fault.
fn page_fault(f: &mut TrapFrame, user: bool) {
    // SAFETY: reading CR2 has no side effects. Read before anything that
    // could fault again.
    let cr2: u64 = unsafe {
        let v: u64;
        core::arch::asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags));
        v
    };
    let ec = f.error_code;
    let rip = f.rip;

    #[cfg(feature = "trace")]
    // SAFETY: COM1 is the kernel console on the supported platforms.
    unsafe {
        raw_serial_str(b"PF! cr2=0x");
        raw_serial_hex(cr2);
        raw_serial_str(b" ec=0x");
        raw_serial_hex(ec);
        raw_serial_str(b" rip=0x");
        raw_serial_hex(rip);
        raw_serial_str(b"\n");
    }
    crate::perf::count_page_fault();

    if !user && cr2 < USER_END {
        // A kernel access to user memory. Inside the fault-tolerant user
        // copy, populate the page if it is merely not present yet (W-3), or
        // resume at the copy's fixup so the syscall returns EFAULT and
        // unwinds normally, releasing its locks (MEM-SEC-01, SYS-SEC-01,
        // W-12).
        #[cfg(target_os = "none")]
        if let Some(fixup) = super::usercopy::fixup_for(rip) {
            let info = crate::mm::page_fault::from_x86_64(ec, cr2, rip);
            if cr2 >= 0x1000 && crate::mm::page_fault::resolve_user_copy_fault(&info) {
                return; // retry the copy instruction
            }
            f.rip = fixup;
            return;
        }
        // Any other kernel access to user memory bypassed the accessors;
        // abandon the syscall through the launch context.
        // SAFETY: COM1 is the kernel console on the supported platforms.
        unsafe {
            raw_serial_str(b"KERN_PF_USER_ADDR cr2=0x");
            raw_serial_hex(cr2);
            raw_serial_str(b" rip=0x");
            raw_serial_hex(rip);
            raw_serial_str(b" sc=0x");
            raw_serial_hex(crate::syscall::LAST_SYSCALL_NUM.load(Ordering::Relaxed));
            raw_serial_str(b"\n");
        }
        if super::usermode::has_boot_return_context() {
            // SAFETY: as in `kill_user`.
            unsafe { super::usermode::boot_return_to_kernel() };
        }
        fatal(b"kernel page fault at a user address", Some(f));
    }

    if user {
        // Demand paging, copy-on-write and stack growth; NULL never maps.
        if cr2 >= 0x1000 {
            let info = crate::mm::page_fault::from_x86_64(ec, cr2, rip);
            if crate::mm::page_fault::handle_page_fault(info).is_ok() {
                return;
            }
        }
        // SAFETY: COM1 is the kernel console on the supported platforms.
        unsafe {
            raw_serial_str(b"SEGFAULT addr=0x");
            raw_serial_hex(cr2);
            raw_serial_str(b" rip=0x");
            raw_serial_hex(rip);
            raw_serial_str(b" ec=0x");
            raw_serial_hex(ec);
            raw_serial_str(b"\n");
        }
        if queue_fault_signal(11) {
            return;
        }
        kill_user(11, true);
    }

    if crate::mm::kstack::is_guard_fault(cr2) {
        fatal(b"kernel stack overflow (guard page)", Some(f));
    }
    fatal(b"kernel page fault", Some(f));
}

/// Legacy PIC timer (IRQ 0).
fn pic_timer() {
    crate::perf::count_interrupt();
    // Skip the tick if the scheduler lock is held: the holder finishes its
    // decision, and the next tick catches up.
    if let Some(mut sched) = crate::sched::scheduler::current_scheduler().try_lock() {
        sched.tick();
    }
    pic_eoi(32);
}

/// PS/2 keyboard (IRQ 1). Must not print or take a console lock.
fn keyboard() {
    crate::perf::count_interrupt();
    // SAFETY: port 0x60 is the 8042 data port; reading it takes the byte.
    let scancode: u8 = unsafe { x86_64::instructions::port::Port::new(0x60).read() };
    crate::drivers::keyboard::handle_scancode(scancode);
    pic_eoi(33);
}

/// Local APIC timer, which interrupted user mode if `user`.
fn apic_timer(user: bool) {
    crate::perf::count_interrupt();
    super::timer::tick(user);
    super::apic::send_eoi();
}

/// Any other legacy PIC vector. IRQ 7 and IRQ 15 can be spurious (the
/// in-service bit is clear); those get no EOI from the PIC that raised them
/// (IRQ 15 still needs one for the cascade on the master).
fn pic_other(vector: u64) {
    let irq = (vector - 32) as u8;
    if irq == 7 || irq == 15 {
        let (cmd, bit) = if irq == 7 { (0x20u16, 7) } else { (0xA0u16, 7) };
        // SAFETY: OCW3 "read ISR" then reading the command port only
        // queries the PIC.
        let isr: u8 = unsafe {
            let mut port = x86_64::instructions::port::Port::<u8>::new(cmd);
            port.write(0x0B);
            port.read()
        };
        if isr & (1 << bit) == 0 {
            if irq == 15 {
                // SAFETY: EOI to the master for the cascade line.
                unsafe { x86_64::instructions::port::Port::<u8>::new(0x20).write(0x20) };
            }
            return;
        }
    }
    UNEXPECTED_VECTORS.fetch_add(1, Ordering::Relaxed);
    pic_eoi(vector);
}

/// End-of-interrupt to the 8259 PIC(s) for `vector` (32-47).
fn pic_eoi(vector: u64) {
    // SAFETY: writing the non-specific EOI command to the PIC command ports
    // only acknowledges the interrupt in service.
    unsafe {
        if vector >= 40 {
            x86_64::instructions::port::Port::<u8>::new(0xA0).write(0x20);
        }
        x86_64::instructions::port::Port::<u8>::new(0x20).write(0x20);
    }
}

/// Number of interrupts that arrived on vectors nothing handles.
pub fn unexpected_vectors() -> u64 {
    UNEXPECTED_VECTORS.load(Ordering::Relaxed)
}

/// One IDT gate (SDM 3A 6.14.1).
#[repr(C)]
#[derive(Clone, Copy)]
struct Gate {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl Gate {
    const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        attributes: 0,
        offset_mid: 0,
        offset_high: 0,
        reserved: 0,
    };

    /// A present 64-bit interrupt gate (IF cleared on entry).
    fn new(handler: u64, selector: u16, ist: u8, dpl: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector,
            ist,
            attributes: 0x8E | (dpl << 5),
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }
}

#[repr(C, align(16))]
struct Idt(core::cell::UnsafeCell<[Gate; 256]>);

// SAFETY: written once, by the boot CPU in `init` before any CPU loads it
// (guarded by IDT_BUILT); read only by the CPUs' interrupt logic after that.
unsafe impl Sync for Idt {}

static IDT: Idt = Idt(core::cell::UnsafeCell::new([Gate::MISSING; 256]));
static IDT_BUILT: AtomicBool = AtomicBool::new(false);

/// Address of vector `v`'s stub.
fn stub(v: usize) -> u64 {
    // Only the address of the table symbol is taken.
    core::ptr::addr_of!(veridian_trap_stubs) as u64 + v as u64 * STUB_STRIDE
}

/// Check that every stub sits at its expected address and pushes its own
/// vector number (`push imm8` = 0x6A, `push imm32` = 0x68), so a change in
/// stub size cannot silently shift every vector by one.
fn verify_stubs() -> bool {
    (0..256).all(|v| {
        // SAFETY: the stub table is 256 * 16 bytes of kernel text.
        let code = unsafe { core::slice::from_raw_parts(stub(v) as *const u8, 16) };
        let vec_push = if has_error_code(v) {
            code
        } else if code[0] == 0x6A && code[1] == 0 {
            &code[2..]
        } else {
            return false;
        };
        match vec_push[0] {
            0x6A => vec_push[1] as usize == v,
            0x68 => {
                u32::from_le_bytes([vec_push[1], vec_push[2], vec_push[3], vec_push[4]]) as usize
                    == v
            }
            _ => false,
        }
    })
}

/// Build the IDT (first call) and load it on this CPU.
pub fn init() {
    if !IDT_BUILT.load(Ordering::Acquire) {
        assert!(verify_stubs(), "trap stub table layout");
        let cs = super::gdt::selectors().code_selector.0;
        // SAFETY: the first call runs on the boot CPU before any other CPU
        // starts or loads the IDT, so nothing reads the table yet.
        let gates = unsafe { &mut *IDT.0.get() };
        for (v, gate) in gates.iter_mut().enumerate() {
            // IST numbers in the gate are 1-based; 0 means "no IST".
            let ist = match v {
                2 => super::gdt::NMI_IST_INDEX + 1,
                8 => super::gdt::DOUBLE_FAULT_IST_INDEX + 1,
                18 => super::gdt::MACHINE_CHECK_IST_INDEX + 1,
                _ => 0,
            } as u8;
            // int3 and into may be executed by user code (debuggers,
            // overflow checks) and must reach their handlers, not #GP.
            let dpl = if v == 3 || v == 4 { 3 } else { 0 };
            *gate = Gate::new(stub(v), cs, ist, dpl);
        }
        IDT_BUILT.store(true, Ordering::Release);
    }
    let ptr = x86_64::structures::DescriptorTablePointer {
        limit: (core::mem::size_of::<[Gate; 256]>() - 1) as u16,
        base: x86_64::VirtAddr::from_ptr(IDT.0.get()),
    };
    // SAFETY: the table is complete, static and lives forever.
    unsafe { x86_64::instructions::tables::lidt(&ptr) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rflags_sanitiser_drops_iopl_and_system_flags() {
        // IOPL 3, NT, RF, VM, VIF/VIP, all set by a forged frame.
        let forged = 0x3000 | (1 << 14) | (1 << 16) | (1 << 17) | (3 << 19) | 0xFFF;
        let clean = sanitize_user_rflags(forged);
        assert_eq!(clean & 0x3000, 0, "IOPL");
        assert_eq!(clean & (1 << 14), 0, "NT");
        assert_eq!(clean & (1 << 16), 0, "RF");
        assert_eq!(clean & (1 << 17), 0, "VM");
        assert_eq!(clean & (3 << 19), 0, "VIF/VIP");
        assert_ne!(clean & 0x200, 0, "IF forced on");
        assert_ne!(clean & 0x2, 0, "reserved bit 1");
        // User arithmetic flags and DF/TF/AC/ID survive.
        let user = 0x0001 | 0x0004 | 0x0010 | 0x0040 | 0x0080 | 0x0100 | 0x0400 | 0x0800;
        assert_eq!(
            sanitize_user_rflags(user | (1 << 18) | (1 << 21)) & !0x202,
            user | (1 << 18) | (1 << 21)
        );
    }

    #[test]
    fn frame_sanitiser_rejects_kernel_and_noncanonical_addresses() {
        let mut f = TrapFrame {
            rip: 0x40_1000,
            rsp: 0x7FFF_FFFF_E000,
            cs: 0x08,
            ss: 0x10,
            rflags: 0x3202,
            ..Default::default()
        };
        assert!(!sanitize_user_frame(&mut f));
        assert_eq!((f.cs, f.ss), (USER_CS, USER_SS));
        assert_eq!(f.rflags, 0x202);
        assert_eq!(f.rip, 0x40_1000);

        let mut f = TrapFrame {
            rip: 0xFFFF_8000_0000_1000,
            rsp: 0x0000_8000_0000_0000,
            cs: USER_CS,
            ss: USER_SS,
            rflags: 0x202,
            ..Default::default()
        };
        assert!(!sanitize_user_frame(&mut f));
        assert_eq!((f.rip, f.rsp), (0, 0));

        let mut f = TrapFrame {
            rip: 0x40_1000,
            rsp: 0x7FFF_FFFF_E000,
            cs: USER_CS,
            ss: USER_SS,
            rflags: 0x246,
            ..Default::default()
        };
        assert!(sanitize_user_frame(&mut f), "a valid frame is unchanged");
    }

    #[test]
    fn error_code_vectors_match_the_sdm() {
        let with: [usize; 10] = [8, 10, 11, 12, 13, 14, 17, 21, 29, 30];
        for v in 0..32 {
            assert_eq!(has_error_code(v), with.contains(&v), "vector {v}");
        }
    }

    #[test]
    fn exception_signals_follow_linux() {
        assert_eq!(exception_signal(0), 8); // #DE -> SIGFPE
        assert_eq!(exception_signal(6), 4); // #UD -> SIGILL
        assert_eq!(exception_signal(13), 11); // #GP -> SIGSEGV
        assert_eq!(exception_signal(17), 7); // #AC -> SIGBUS
        assert_eq!(exception_signal(3), 5); // #BP -> SIGTRAP
        assert_eq!(exception_signal(19), 8); // #XM -> SIGFPE
    }
}

//! x86_64 system call entry point and SYSCALL/SYSRET MSR configuration.
//!
//! This module configures the CPU's SYSCALL/SYSRET mechanism for user-kernel
//! transitions. The key components are:
//! - `syscall_entry`: naked assembly handler invoked by the SYSCALL instruction
//! - `PerCpuData`: per-CPU storage for kernel/user RSP, accessed via GS segment
//! - `init_syscall`: MSR configuration (EFER, STAR, LSTAR, SFMASK) and the boot
//!   CPU's per-CPU block

#![allow(function_casts_as_integer)]

use crate::{arch::percpu::this_arch_cpu_ptr, syscall::syscall_handler};

/// Saved user registers of a system call: the same [`TrapFrame`] layout the
/// exception entry builds (`vector` = `SYSCALL_VECTOR`, `error_code` = the
/// system call number). `rcx` and `r11` hold the user RIP and RFLAGS as the
/// `syscall` instruction left them; `rip`, `rflags` and `rsp` are what the
/// return path loads, so code that redirects the return (exec, signal
/// delivery, sigreturn) changes those.
pub type SyscallFrame = super::trap::TrapFrame;

/// Get a reference to the saved syscall register frame of the syscall this
/// CPU is executing.
///
/// Only valid during syscall handler execution. Returns `None` if called
/// outside of a syscall context. The pointer is per CPU (`gs:[0x10]`), so
/// with more than one CPU a syscall never reads another CPU's frame (N-35).
///
/// # Safety
/// The returned reference points to the kernel stack. It is valid only while
/// the syscall handler is executing (before registers are popped on return).
pub fn get_syscall_frame() -> Option<&'static SyscallFrame> {
    let cpu = this_arch_cpu_ptr();
    if !crate::arch::percpu::is_arch_cpu_block(cpu as u64) {
        return None;
    }
    // SAFETY: `cpu` is this CPU's block (checked); only this CPU writes it.
    let ptr = unsafe { (*cpu).syscall_frame };
    if ptr == 0 {
        return None;
    }
    // SAFETY: syscall_entry stores the kernel stack address after all
    // registers are pushed, and clears it before they are popped, so a
    // non-zero value is a live SyscallFrame for the duration of the
    // handler. The layout matches the push order in the assembly.
    Some(unsafe { &*(ptr as *const SyscallFrame) })
}

/// Per-CPU data accessed via GS during syscall entry/exit: the architecture
/// per-CPU block (`kernel_rsp` at `gs:[0x0]`, `user_rsp` at `gs:[0x8]`,
/// `syscall_frame` at `gs:[0x10]`).
pub type PerCpuData = crate::arch::percpu::ArchCpu;

/// Get a mutable pointer to the calling CPU's per-CPU data.
pub fn per_cpu_data_ptr() -> *mut PerCpuData {
    this_arch_cpu_ptr()
}

/// Decide how a system call returns, after sanitising its frame (N-170).
/// Returns 1 if `sysretq` can return it, 0 if it must use `iretq`.
///
/// `sysretq` loads RIP from RCX and RFLAGS from R11, so it can only return a
/// frame whose `rcx`/`r11` still equal `rip`/`rflags` (not one rewritten by
/// exec or sigreturn). It is also kept away from the top user page: on
/// Intel CPUs a `sysretq` with a non-canonical RCX raises #GP in ring 0 with
/// the user stack loaded (CVE-2012-0217), and Linux likewise refuses the
/// last page below the canonical boundary (N-171).
extern "C" fn syscall_exit_prepare(frame: &mut SyscallFrame) -> u64 {
    super::trap::sanitize_user_frame(frame);
    let sysret_ok = frame.rcx == frame.rip
        && frame.r11 == frame.rflags
        && frame.rip < super::trap::USER_END - 4096;
    sysret_ok as u64
}

/// x86_64 SYSCALL instruction entry point
///
/// Builds a [`TrapFrame`](super::trap::TrapFrame) on this CPU's entry stack
/// (`gs:[0x0]`), calls the system call handler, stores its result in the
/// frame, and returns through `sysretq` when the frame allows it and through
/// `iretq` otherwise (see `syscall_exit_prepare`).
///
/// # Safety
/// This function must only be called by the CPU's SYSCALL instruction.
/// It expects specific register states as defined by the x86_64 ABI.
#[no_mangle]
#[unsafe(naked)]
pub unsafe extern "C" fn syscall_entry() {
    core::arch::naked_asm!(
        // Interrupts are off (SFMASK clears IF) until the handler enables
        // them, so nothing can arrive while GS and RSP are being switched.
        "swapgs",
        "mov gs:[0x8], rsp",          // user RSP (scratch)
        "mov rsp, gs:[0x0]",          // this CPU's entry stack (16-byte aligned)

        // Hardware-style frame: ss, rsp, rflags, cs, rip; then error_code
        // (the syscall number) and vector.
        "push {user_ss}",
        "push qword ptr gs:[0x8]",
        "push r11",                   // user RFLAGS
        "push {user_cs}",
        "push rcx",                   // user RIP
        "push rax",                   // error_code = syscall number
        "push -1",                    // vector = SYSCALL_VECTOR
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
        "mov gs:[0x10], rsp",         // this CPU's frame pointer (N-35)

        // User SSE registers (xmm0-xmm15). The kernel itself is built
        // soft-float, but a parent's registers must survive a nested child
        // run inside its syscall until user vector state is switched per
        // task with XSAVE (N-41).
        "sub rsp, 256",
        "movdqu [rsp],      xmm0",
        "movdqu [rsp+0x10], xmm1",
        "movdqu [rsp+0x20], xmm2",
        "movdqu [rsp+0x30], xmm3",
        "movdqu [rsp+0x40], xmm4",
        "movdqu [rsp+0x50], xmm5",
        "movdqu [rsp+0x60], xmm6",
        "movdqu [rsp+0x70], xmm7",
        "movdqu [rsp+0x80], xmm8",
        "movdqu [rsp+0x90], xmm9",
        "movdqu [rsp+0xa0], xmm10",
        "movdqu [rsp+0xb0], xmm11",
        "movdqu [rsp+0xc0], xmm12",
        "movdqu [rsp+0xd0], xmm13",
        "movdqu [rsp+0xe0], xmm14",
        "movdqu [rsp+0xf0], xmm15",

        // SYSCALL ABI (rax = number, rdi rsi rdx r10 r8 = args) to the C
        // ABI (rdi = number, rsi rdx rcx r8 r9 = args).
        "xchg rdi, rax",
        "xchg rsi, rax",
        "xchg rdx, rax",
        "mov rcx, rax",
        "mov r9, r8",
        "mov r8, r10",
        "cld",
        "call {handler}",

        // The handler may have enabled interrupts (waits); nothing may
        // arrive from here to the return, or it would see a ring-0 CS
        // with the user GS base after the swapgs.
        "cli",
        "mov [rsp + 256 + {rax_off}], rax",   // result into frame.rax
        "mov qword ptr gs:[0x10], 0",
        "lea rdi, [rsp + 256]",
        "call {prepare}",
        "test eax, eax",                      // flags survive until the jz

        "movdqu xmm0,  [rsp]",
        "movdqu xmm1,  [rsp+0x10]",
        "movdqu xmm2,  [rsp+0x20]",
        "movdqu xmm3,  [rsp+0x30]",
        "movdqu xmm4,  [rsp+0x40]",
        "movdqu xmm5,  [rsp+0x50]",
        "movdqu xmm6,  [rsp+0x60]",
        "movdqu xmm7,  [rsp+0x70]",
        "movdqu xmm8,  [rsp+0x80]",
        "movdqu xmm9,  [rsp+0x90]",
        "movdqu xmm10, [rsp+0xa0]",
        "movdqu xmm11, [rsp+0xb0]",
        "movdqu xmm12, [rsp+0xc0]",
        "movdqu xmm13, [rsp+0xd0]",
        "movdqu xmm14, [rsp+0xe0]",
        "movdqu xmm15, [rsp+0xf0]",
        "lea rsp, [rsp + 256]",
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
        "jz 2f",

        // sysretq: rcx == rip and r11 == rflags (checked above).
        "mov rsp, [rsp + 16 + 24]",   // user RSP from frame.rsp
        "swapgs",
        "sysretq",

        // iretq: the frame's rip/cs/rflags/rsp/ss.
        "2:",
        "add rsp, 16",                // vector, error_code
        "swapgs",
        "iretq",

        handler = sym syscall_handler,
        prepare = sym syscall_exit_prepare,
        user_ss = const super::trap::USER_SS,
        user_cs = const super::trap::USER_CS,
        rax_off = const core::mem::offset_of!(super::trap::TrapFrame, rax),
    );
}

/// Initialize SYSCALL/SYSRET support.
///
/// Configures the following MSRs:
/// - **EFER**: Enable SYSCALL/SYSRET extensions
/// - **LSTAR**: Set syscall entry point to `syscall_entry`
/// - **STAR**: Set segment selectors for SYSCALL (kernel) and SYSRET (user)
/// - **SFMASK**: Mask IF flag so syscall entry runs with interrupts disabled
/// - **GS_BASE**: the boot CPU's per-CPU block
///
/// Must be called after `gdt::init()` and before any user-mode transitions.
pub fn init_syscall() {
    use x86_64::registers::{
        model_specific::{Efer, EferFlags, LStar, SFMask, Star},
        rflags::RFlags,
    };

    use super::gdt;

    let sels = gdt::selectors();

    // SAFETY: Writing MSRs to configure SYSCALL/SYSRET is required during
    // kernel init for system call support. EFER, LSTAR, STAR, SFMASK, and
    // KernelGsBase are x86_64 model-specific registers that control the
    // SYSCALL instruction behavior. This is called with interrupts disabled
    // during single-threaded init.
    unsafe {
        // Enable SYSCALL/SYSRET
        Efer::update(|flags| {
            flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });
    }

    // Set up SYSCALL entry point
    LStar::write(x86_64::VirtAddr::new(syscall_entry as usize as u64));

    // Set up segment selectors for SYSCALL/SYSRET transitions.
    //
    // GDT layout after gdt::init():
    //   0x08: Kernel CS (Ring 0)
    //   0x10: Kernel DS (Ring 0)
    //   0x18: TSS (occupies 2 entries)
    //   0x28: User Data (Ring 3, selector 0x2B with RPL)
    //   0x30: User Code (Ring 3, selector 0x33 with RPL)
    //
    // Star::write validates:
    //   cs_sysret(0x33) - 16 = 0x23 == ss_sysret(0x2B) - 8 = 0x23  (match)
    //   cs_syscall(0x08) == ss_syscall(0x10) - 8 = 0x08              (match)
    //   ss_sysret RPL = 3 (Ring3)                                     (correct)
    //   ss_syscall RPL = 0 (Ring0)                                    (correct)
    //
    // Internally writes STAR[63:48] = ss_sysret - 8 = 0x23, which means:
    //   SYSRET: CS = 0x23+16 = 0x33 (user code), SS = 0x23+8 = 0x2B (user data)
    Star::write(
        sels.user_code_selector, // User CS for SYSRET (0x33)
        sels.user_data_selector, // User SS for SYSRET (0x2B)
        sels.code_selector,      // Kernel CS for SYSCALL (0x08)
        sels.data_selector,      // Kernel SS for SYSCALL (0x10)
    )
    .expect("failed to configure STAR MSR segment selectors");

    // SFMASK: flags cleared on SYSCALL entry. IF, so we enter with
    // interrupts disabled until we are on the kernel stack. DF, because the
    // Rust ABI and every `rep movs`/`rep stos` in the kernel assume it is
    // clear: a user `std; syscall` otherwise made the user-copy routine copy
    // downwards, outside the range it validated (review of the v0.26.0
    // stack, PR #8). TF, AC and NT for the same reason Linux masks them:
    // user single-stepping, alignment checks and nested-task state must not
    // carry into kernel code.
    SFMask::write(
        RFlags::INTERRUPT_FLAG
            | RFlags::DIRECTION_FLAG
            | RFlags::TRAP_FLAG
            | RFlags::ALIGNMENT_CHECK
            | RFlags::NESTED_TASK,
    );

    // Per-CPU block: GS_BASE while ring 0 runs; syscall_entry's swapgs
    // brings it in from KernelGsBase on entry from ring 3.

    // The boot CPU is logical CPU 0; its hardware id is the initial APIC ID.
    // SAFETY: CPUID leaf 1 is unprivileged and side-effect free.
    let apic_id = unsafe { core::arch::x86_64::__cpuid(1).ebx >> 24 };
    // SAFETY: runs once on the boot CPU, before any syscall or other use of
    // its per-CPU block.
    unsafe { crate::arch::percpu::install(0, apic_id) };
}

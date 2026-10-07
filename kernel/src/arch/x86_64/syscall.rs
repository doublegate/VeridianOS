//! x86_64 system call entry point and SYSCALL/SYSRET MSR configuration.
//!
//! This module configures the CPU's SYSCALL/SYSRET mechanism for user-kernel
//! transitions. The key components are:
//! - `syscall_entry`: naked assembly handler invoked by the SYSCALL instruction
//! - `PerCpuData`: per-CPU storage for kernel/user RSP, accessed via GS segment
//! - `init_syscall`: MSR configuration (EFER, STAR, LSTAR, SFMASK,
//!   KernelGsBase)

#![allow(function_casts_as_integer)]

use crate::{arch::percpu::this_arch_cpu_ptr, syscall::syscall_handler};

/// Saved user register frame from SYSCALL entry.
///
/// This struct matches the exact push order in `syscall_entry` assembly.
/// After all pushes, RSP points to this layout (lowest address = first field).
/// The struct is used by `fork_process()` to capture the live register state
/// of the parent at the moment of the fork() syscall, so the child gets a
/// copy of the parent's actual CPU registers rather than the stale
/// ThreadContext from process creation time.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SyscallFrame {
    pub r9: u64,  // arg6 (pushed last)
    pub r8: u64,  // arg5
    pub r10: u64, // arg4
    pub rdx: u64, // arg3
    pub rsi: u64, // arg2
    pub rdi: u64, // arg1
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub r11: u64, // User RFLAGS (clobbered by SYSCALL)
    pub rcx: u64, // User RIP (clobbered by SYSCALL)
}

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

/// Get the user RSP saved by syscall_entry into per-CPU data.
///
/// Only valid during syscall handler execution.
pub fn get_saved_user_rsp() -> u64 {
    // SAFETY: this CPU's block; syscall_entry stores user_rsp (gs:[0x8])
    // before switching to the kernel stack.
    unsafe { (*this_arch_cpu_ptr()).user_rsp }
}

/// Per-CPU data accessed via GS during syscall entry/exit: the architecture
/// per-CPU block (`kernel_rsp` at `gs:[0x0]`, `user_rsp` at `gs:[0x8]`,
/// `syscall_frame` at `gs:[0x10]`).
pub type PerCpuData = crate::arch::percpu::ArchCpu;

// CR3 switching removed: Process page tables now contain complete kernel
// mapping (L4 entries 256-511 copied from boot tables), so syscalls run
// with user CR3 active. This eliminates the GP fault on CR3 restore that
// occurred when switching back to incompatible user page tables.

/// Get a mutable pointer to the calling CPU's per-CPU data.
///
/// Used to update `kernel_rsp` on context switch. The returned pointer is
/// valid for the lifetime of the kernel.
pub fn per_cpu_data_ptr() -> *mut PerCpuData {
    this_arch_cpu_ptr()
}

/// x86_64 SYSCALL instruction entry point
///
/// This function handles the transition from user mode to kernel mode
/// when a SYSCALL instruction is executed. It saves the user context,
/// switches to the kernel stack, and calls the system call handler.
///
/// # Safety
/// This function must only be called by the CPU's SYSCALL instruction.
/// It expects specific register states as defined by the x86_64 ABI.
#[no_mangle]
#[unsafe(naked)]
pub unsafe extern "C" fn syscall_entry() {
    core::arch::naked_asm!(
        // Save user context on kernel stack
        "swapgs",                    // Switch to kernel GS
        "mov gs:[0x8], rsp",        // Save user RSP in per-CPU data (offset 0x8)
        "mov rsp, gs:[0x0]",        // Load kernel RSP from per-CPU data (offset 0x0)

        // CR3 switching removed: Process page tables contain complete kernel
        // mapping, so we can access kernel data structures directly without
        // switching to boot page tables.

        // Save all user registers.
        // rcx and r11 are clobbered by SYSCALL (RIP / RFLAGS), saved first.
        // Callee-saved: rbp, rbx, r12-r15. Caller-saved / args: rdi, rsi,
        // rdx, r10, r8, r9. All must be preserved so the user sees correct
        // values after SYSRET (except rax which holds the return value).
        "push rcx",                  // User RIP
        "push r11",                  // User RFLAGS
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "push rdi",                  // arg1 (will be clobbered by ABI shuffle)
        "push rsi",                  // arg2
        "push rdx",                  // arg3
        "push r10",                  // arg4
        "push r8",                   // arg5
        "push r9",                   // arg6

        // Save frame pointer for fork() register capture.
        // RSP now points to the complete SyscallFrame on the kernel stack.
        // fork_process() reads this to give the child a copy of the parent's
        // live registers instead of the stale ThreadContext from exec/load.
        "mov gs:[0x10], rsp",        // this CPU's frame pointer (N-35)

        // Save user SSE registers (xmm0-xmm15).
        // The kernel is compiled with +sse2 and LLVM may use XMM registers
        // in any Rust function (memcpy, memset, optimizations). Without
        // saving them here, the kernel clobbers user SSE state.
        "sub rsp, 256",              // 16 registers * 16 bytes
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

        // Rearrange registers from SYSCALL ABI to C calling convention.
        //
        // SYSCALL ABI:  rax=number, rdi=arg1, rsi=arg2, rdx=arg3, r10=arg4, r8=arg5
        // C convention: rdi=param1, rsi=param2, rdx=param3, rcx=param4, r8=param5, r9=param6
        //
        // We need: rdi=rax, rsi=rdi, rdx=rsi, rcx=rdx, r8=r10, r9=r8
        // Use xchg chain through rax as accumulator to rotate the values.
        "xchg rdi, rax",             // rdi = syscall_num (rax), rax = arg1 (old rdi)
        "xchg rsi, rax",             // rsi = arg1 (rax), rax = arg2 (old rsi)
        "xchg rdx, rax",             // rdx = arg2 (rax), rax = arg3 (old rdx)
        "mov rcx, rax",              // rcx = arg3 (old rdx)
        "mov r9, r8",                // r9 = arg5 (must precede r8 overwrite)
        "mov r8, r10",               // r8 = arg4

        "call {handler}",

        // Clear frame pointer now that handler has returned.
        // This prevents stale pointer use outside syscall context.
        "mov qword ptr gs:[0x10], 0",

        // Restore user SSE registers (xmm0-xmm15)
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
        "add rsp, 256",

        // Restore user registers (reverse order of saves).
        // rax holds the syscall return value and is NOT restored.
        "pop r9",
        "pop r8",
        "pop r10",
        "pop rdx",
        "pop rsi",
        "pop rdi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "pop r11",                   // User RFLAGS
        "pop rcx",                   // User RIP

        // Restore user stack and return (no CR3 switching)
        "mov rsp, gs:[0x8]",        // Restore user RSP
        "swapgs",                    // Switch back to user GS
        "sysretq",

        handler = sym syscall_handler,
    );
}

/// Initialize SYSCALL/SYSRET support.
///
/// Configures the following MSRs:
/// - **EFER**: Enable SYSCALL/SYSRET extensions
/// - **LSTAR**: Set syscall entry point to `syscall_entry`
/// - **STAR**: Set segment selectors for SYSCALL (kernel) and SYSRET (user)
/// - **SFMASK**: Mask IF flag so syscall entry runs with interrupts disabled
/// - **KernelGsBase**: Point to `PerCpuData` for swapgs in syscall_entry
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

    // Set up per-CPU data for swapgs.
    // KernelGsBase is swapped with GsBase on the `swapgs` instruction.
    // After swapgs in syscall_entry, GS points to our PerCpuData so the
    // assembly can read kernel_rsp from gs:[0x0] and save user_rsp to gs:[0x8].
    //
    // CR3 initialization removed: Process page tables now contain complete
    // kernel mappings (L4 entries 256-511), so syscalls run with user CR3
    // and can directly access kernel data structures.

    // The boot CPU is logical CPU 0; its hardware id is the initial APIC ID.
    // SAFETY: CPUID leaf 1 is unprivileged and side-effect free.
    let apic_id = unsafe { core::arch::x86_64::__cpuid(1).ebx >> 24 };
    // SAFETY: runs once on the boot CPU, before any syscall or other use of
    // its per-CPU block.
    unsafe { crate::arch::percpu::install(0, apic_id) };
}

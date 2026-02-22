//! x86_64 system call entry point and SYSCALL/SYSRET MSR configuration.
//!
//! This module configures the CPU's SYSCALL/SYSRET mechanism for user-kernel
//! transitions. The key components are:
//! - `syscall_entry`: naked assembly handler invoked by the SYSCALL instruction
//! - `PerCpuData`: per-CPU storage for kernel/user RSP, accessed via GS segment
//! - `init_syscall`: MSR configuration (EFER, STAR, LSTAR, SFMASK,
//!   KernelGsBase)

#![allow(function_casts_as_integer)]

use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::syscall::syscall_handler;

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

/// Kernel stack pointer after all user registers are saved in syscall_entry.
/// Points to a valid `SyscallFrame` during syscall handler execution.
/// Set to 0 outside of syscall context.
static SYSCALL_FRAME_PTR: AtomicU64 = AtomicU64::new(0);

/// Get a reference to the saved syscall register frame.
///
/// Only valid during syscall handler execution. Returns `None` if called
/// outside of a syscall context.
///
/// # Safety
/// The returned reference points to the kernel stack. It is valid only while
/// the syscall handler is executing (before registers are popped on return).
pub fn get_syscall_frame() -> Option<&'static SyscallFrame> {
    let ptr = SYSCALL_FRAME_PTR.load(Ordering::Acquire);
    if ptr == 0 {
        return None;
    }
    // SAFETY: SYSCALL_FRAME_PTR is set by syscall_entry to point to the
    // kernel stack after all registers are pushed. The pointer is valid
    // for the duration of the syscall handler. The SyscallFrame layout
    // matches the exact push order in the assembly.
    Some(unsafe { &*(ptr as *const SyscallFrame) })
}

/// Get the user RSP saved by syscall_entry into per-CPU data.
///
/// Only valid during syscall handler execution.
pub fn get_saved_user_rsp() -> u64 {
    // SAFETY: PER_CPU_AREA.user_rsp is set by syscall_entry (mov gs:[0x8], rsp)
    // before switching to the kernel stack. It is valid during syscall handling.
    unsafe { (*PER_CPU_AREA.0.get()).user_rsp }
}

/// Per-CPU data accessed via GS segment register during syscall entry/exit.
///
/// The `syscall_entry` naked asm reads `kernel_rsp` from `gs:[0x0]` and saves
/// `user_rsp` to `gs:[0x8]`. This struct must be `#[repr(C)]` to guarantee
/// field layout matches the assembly offsets.
#[repr(C)]
pub struct PerCpuData {
    /// Kernel stack pointer (offset 0x0) -- loaded into RSP on syscall entry
    pub kernel_rsp: u64,
    /// User stack pointer (offset 0x8) -- saved from RSP on syscall entry
    pub user_rsp: u64,
}

#[repr(transparent)]
pub(super) struct PerCpuDataCell(UnsafeCell<PerCpuData>);

// SAFETY: Per-CPU data is only accessed via GS register from the current CPU
// during syscall entry/exit. On a single-CPU system (our current QEMU config),
// there are no concurrent accesses. The naked asm in syscall_entry uses
// `mov gs:[offset]` which does not go through Rust's aliasing rules.
unsafe impl Sync for PerCpuDataCell {}

pub(super) static PER_CPU_AREA: PerCpuDataCell = PerCpuDataCell(UnsafeCell::new(PerCpuData {
    kernel_rsp: 0,
    user_rsp: 0,
}));

// CR3 switching removed: Process page tables now contain complete kernel
// mapping (L4 entries 256-511 copied from boot tables), so syscalls run
// with user CR3 active. This eliminates the GP fault on CR3 restore that
// occurred when switching back to incompatible user page tables.

/// Get a mutable pointer to the per-CPU data.
///
/// Used to update `kernel_rsp` on context switch and to set up KernelGsBase
/// during init. The returned pointer is valid for the lifetime of the kernel.
pub fn per_cpu_data_ptr() -> *mut PerCpuData {
    PER_CPU_AREA.0.get()
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
        "mov [{frame_ptr}], rsp",

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
        "mov qword ptr [{frame_ptr}], 0",

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
        frame_ptr = sym SYSCALL_FRAME_PTR,
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
        model_specific::{Efer, EferFlags, KernelGsBase, LStar, SFMask, Star},
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

    // SFMASK: mask the IF flag during SYSCALL so we enter with interrupts
    // disabled. This prevents interrupt handlers from firing before we have
    // switched to the kernel stack.
    SFMask::write(RFlags::INTERRUPT_FLAG);

    // Set up per-CPU data for swapgs.
    // KernelGsBase is swapped with GsBase on the `swapgs` instruction.
    // After swapgs in syscall_entry, GS points to our PerCpuData so the
    // assembly can read kernel_rsp from gs:[0x0] and save user_rsp to gs:[0x8].
    //
    // CR3 initialization removed: Process page tables now contain complete
    // kernel mappings (L4 entries 256-511), so syscalls run with user CR3
    // and can directly access kernel data structures.

    let per_cpu_addr = per_cpu_data_ptr() as u64;
    KernelGsBase::write(x86_64::VirtAddr::new(per_cpu_addr));
}

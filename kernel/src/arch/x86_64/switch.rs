//! Task switching on kernel stacks (ADR 0006).
//!
//! Every task owns a kernel stack. A switch pushes the callee-saved
//! registers on the current stack, stores the stack pointer in the outgoing
//! task, loads the incoming task's stack pointer and pops its registers; the
//! `ret` then continues wherever that task last switched out. All other
//! state (a user task's registers, a syscall in progress) is already on the
//! task's own stack.
//!
//! A new task's stack is seeded with a switch frame whose return address is
//! a start trampoline:
//!
//! ```text
//!   kernel thread:  [r15 r14 r13 r12 rbx rbp] [kthread_trampoline]
//!                    r12 = entry, r13 = argument
//!   user thread:    [r15 r14 r13 r12 rbx rbp] [user_trampoline] [UserFrame]
//! ```

use core::arch::{asm, naked_asm};

use super::trap::{is_user_address, sanitize_user_rflags, USER_CS, USER_SS};

/// Registers restored when a new user task first enters ring 3, in the order
/// `user_trampoline` pops them, followed by the `iretq` frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct UserFrame {
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
    // iretq frame
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

const SWITCH_REGS: usize = 6;

impl UserFrame {
    /// The entry frame for a user thread whose registers are `ctx`.
    /// RIP and RSP must be canonical user addresses (else `None`: a forged
    /// `iretq` frame would fault in ring 0); RFLAGS is sanitised.
    pub fn from_context(ctx: &super::context::X86_64Context) -> Option<Self> {
        if !is_user_address(ctx.rip) || !is_user_address(ctx.rsp) {
            return None;
        }
        Some(Self {
            r15: ctx.r15,
            r14: ctx.r14,
            r13: ctx.r13,
            r12: ctx.r12,
            r11: ctx.r11,
            r10: ctx.r10,
            r9: ctx.r9,
            r8: ctx.r8,
            rbp: ctx.rbp,
            rdi: ctx.rdi,
            rsi: ctx.rsi,
            rdx: ctx.rdx,
            rcx: ctx.rcx,
            rbx: ctx.rbx,
            rax: ctx.rax,
            rip: ctx.rip,
            cs: USER_CS,
            rflags: sanitize_user_rflags(ctx.rflags),
            rsp: ctx.rsp,
            ss: USER_SS,
        })
    }
}

/// Save the callee-saved registers and stack pointer of the running task
/// into `*prev_sp`, and resume the task whose saved stack pointer is
/// `next_sp`. Returns when the outgoing task is switched back in.
///
/// # Safety
/// Interrupts must be disabled. `prev_sp` must be writable; `next_sp` must be
/// a stack pointer saved by this function or produced by a `seed_*`
/// function, on a stack that stays mapped while the task runs.
#[unsafe(naked)]
pub unsafe extern "C" fn switch_stacks(prev_sp: *mut usize, next_sp: usize) {
    naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    );
}

/// First code of a new kernel thread: finish the switch, enable interrupts
/// (`schedule` switched with them off), then run `entry(arg)` (r12, r13)
/// and exit if it returns.
#[unsafe(naked)]
unsafe extern "C" fn kthread_trampoline() {
    naked_asm!(
        // rsp is 16-byte aligned here (seeded so), as a call requires.
        "call {finish}",
        "sti",
        "mov rdi, r13",
        "call r12",
        "call {exit}",
        "ud2",
        finish = sym crate::sched::dispatch::finish_switch,
        exit = sym crate::sched::dispatch::exit_kernel_thread,
    );
}

/// First code of a new user thread: finish the switch, then pop the
/// `UserFrame` registers and enter ring 3.
#[unsafe(naked)]
unsafe extern "C" fn user_trampoline() {
    naked_asm!(
        // rsp points at the UserFrame, 16-byte aligned.
        "call {finish}",
        // A fatal signal sent before the thread first ran ends it here.
        "call {check}",
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
        // Kernel GS_BASE holds the per-CPU block; give the user its own.
        "swapgs",
        "iretq",
        finish = sym crate::sched::dispatch::finish_switch,
        check = sym crate::process::user_return_check,
    );
}

/// Push `v` onto a downward-growing stack at `*sp`.
///
/// # Safety
/// `*sp - 8` must be writable.
unsafe fn push(sp: &mut usize, v: u64) {
    *sp -= 8;
    // SAFETY: forwarded from the caller.
    unsafe { (*sp as *mut u64).write(v) };
}

/// Seed a kernel stack (top `stack_top`, 16-byte aligned) so that switching
/// to it runs `entry(arg)`. Returns the saved stack pointer.
///
/// # Safety
/// The top 64 bytes below `stack_top` must be writable and unused.
pub unsafe fn seed_kernel_thread(
    stack_top: usize,
    entry: extern "C" fn(usize),
    arg: usize,
) -> usize {
    let mut sp = stack_top & !0xF;
    // SAFETY: within the caller's writable stack top.
    unsafe {
        push(&mut sp, 0); // keeps rsp 16-aligned after the ret below
        push(&mut sp, kthread_trampoline as *const () as u64);
        push(&mut sp, 0); // rbp
        push(&mut sp, 0); // rbx
        push(&mut sp, entry as *const () as u64); // r12
        push(&mut sp, arg as u64); // r13
        push(&mut sp, 0); // r14
        push(&mut sp, 0); // r15
    }
    debug_assert_eq!((stack_top & !0xF) - sp, (SWITCH_REGS + 2) * 8);
    sp
}

/// Seed a kernel stack so that switching to it enters ring 3 with `frame`.
/// Returns the saved stack pointer.
///
/// # Safety
/// The top `size_of::<UserFrame>() + 64` bytes below `stack_top` must be
/// writable and unused.
pub unsafe fn seed_user_thread(stack_top: usize, frame: &UserFrame) -> usize {
    let top = stack_top & !0xF;
    let frame_at = top - core::mem::size_of::<UserFrame>();
    debug_assert_eq!(frame_at % 16, 0);
    // SAFETY: within the caller's writable stack top.
    unsafe {
        (frame_at as *mut UserFrame).write(*frame);
        let mut sp = frame_at;
        push(&mut sp, user_trampoline as *const () as u64);
        for _ in 0..SWITCH_REGS {
            push(&mut sp, 0);
        }
        sp
    }
}

/// FS base (user TLS pointer).
pub fn read_fs_base() -> u64 {
    // SAFETY: IA32_FS_BASE is always readable in ring 0.
    unsafe { x86_64::registers::model_specific::Msr::new(0xC000_0100).read() }
}

/// Set the FS base. `base` must be canonical (checked by callers that take it
/// from user space).
pub fn write_fs_base(base: u64) {
    // SAFETY: writing a canonical value to IA32_FS_BASE has no other effect.
    unsafe { x86_64::registers::model_specific::Msr::new(0xC000_0100).write(base) }
}

/// Load a page table root if it differs from the active one.
///
/// # Safety
/// `root` must be a valid PML4 that maps the kernel half.
pub unsafe fn switch_address_space(root: u64) {
    let cur: u64;
    // SAFETY: reading CR3 has no side effects.
    unsafe { asm!("mov {}, cr3", out(reg) cur, options(nomem, nostack, preserves_flags)) };
    if root != 0 && root != cur & !0xFFF {
        // SAFETY: forwarded from the caller.
        unsafe { asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags)) };
    }
}

/// Extended (FPU/SSE/AVX) state save area, XSAVE format.
pub mod xsave {
    use alloc::alloc::{alloc_zeroed, dealloc, Layout};
    use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

    /// Bytes needed for the enabled XCR0 features (CPUID 0Dh/0 EBX), or 512
    /// for FXSAVE when XSAVE is unavailable.
    static SIZE: AtomicU32 = AtomicU32::new(0);
    static XCR0: AtomicU64 = AtomicU64::new(0);

    /// Record the area size. Call after `context::init_fpu` (XCR0 set).
    pub fn init() {
        if super::super::context::has_xsave() {
            // SAFETY: CPUID leaf 0Dh is unprivileged and side-effect free.
            let r = unsafe { core::arch::x86_64::__cpuid_count(0x0D, 0) };
            SIZE.store(r.ebx.max(576), Ordering::Release);
            // SAFETY: XGETBV is available once CR4.OSXSAVE is set (init_fpu).
            let xcr0 = unsafe { core::arch::x86_64::_xgetbv(0) };
            XCR0.store(xcr0, Ordering::Release);
        } else {
            SIZE.store(512, Ordering::Release);
        }
    }

    pub fn size() -> usize {
        SIZE.load(Ordering::Acquire) as usize
    }

    /// A 64-byte-aligned save area holding a task's user vector state.
    pub struct Area {
        ptr: *mut u8,
        layout: Layout,
    }

    // SAFETY: the area is plain memory owned by one task; it is only touched
    // by the CPU running (or switching) that task.
    unsafe impl Send for Area {}
    // SAFETY: as above; no shared mutation.
    unsafe impl Sync for Area {}

    impl Area {
        /// A fresh area holding the initial state (x87 default control word,
        /// MXCSR 0x1F80 with all exceptions masked, vector registers zero).
        pub fn new() -> Option<Self> {
            let size = size().max(512);
            let layout = Layout::from_size_align(size, 64).ok()?;
            // SAFETY: non-zero size.
            let ptr = unsafe { alloc_zeroed(layout) };
            if ptr.is_null() {
                return None;
            }
            // SAFETY: legacy region: FCW at 0, MXCSR at 24 (within 512 bytes).
            unsafe {
                (ptr as *mut u16).write(0x037F);
                (ptr.add(24) as *mut u32).write(0x1F80);
            }
            // XSTATE_BV (header at 512) stays 0 except x87/SSE, so XRSTOR
            // loads the init state for the rest; legacy fields come from the
            // area because bits 0-1 are set.
            if size >= 576 {
                // SAFETY: header lies within the area.
                unsafe { (ptr.add(512) as *mut u64).write(0b11) };
            }
            Some(Self { ptr, layout })
        }

        /// Save the CPU's user extended state here.
        pub fn save(&mut self) {
            let mask = XCR0.load(Ordering::Relaxed);
            // SAFETY: 64-byte aligned area of the CPUID-reported size.
            unsafe {
                if mask != 0 {
                    core::arch::asm!("xsave64 [{}]", in(reg) self.ptr,
                        in("eax") mask as u32, in("edx") (mask >> 32) as u32,
                        options(nostack, preserves_flags));
                } else {
                    core::arch::asm!("fxsave64 [{}]", in(reg) self.ptr, options(nostack, preserves_flags));
                }
            }
        }

        /// Load this area into the CPU.
        pub fn restore(&self) {
            let mask = XCR0.load(Ordering::Relaxed);
            // SAFETY: the area holds a state produced by `new` or `save`.
            unsafe {
                if mask != 0 {
                    core::arch::asm!("xrstor64 [{}]", in(reg) self.ptr,
                        in("eax") mask as u32, in("edx") (mask >> 32) as u32,
                        options(nostack, preserves_flags));
                } else {
                    core::arch::asm!("fxrstor64 [{}]", in(reg) self.ptr, options(nostack, preserves_flags));
                }
            }
        }

        /// Copy another task's state (fork, clone).
        pub fn copy_from(&mut self, other: &Area) {
            // SAFETY: both areas have the same (global) size.
            unsafe { core::ptr::copy_nonoverlapping(other.ptr, self.ptr, self.layout.size()) };
        }
    }

    impl Drop for Area {
        fn drop(&mut self) {
            // SAFETY: allocated in `new` with this layout.
            unsafe { dealloc(self.ptr, self.layout) };
        }
    }
}

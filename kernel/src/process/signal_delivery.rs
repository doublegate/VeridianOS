//! Signal delivery to user-space signal handlers
//!
//! When a signal becomes pending for a process and the signal has a registered
//! handler (not SIG_DFL or SIG_IGN), the kernel must arrange for the handler to
//! execute in user space. This module implements the signal frame construction
//! and restoration mechanism:
//!
//! 1. **Delivery** (`deliver_signal`): Saves the current thread context into a
//!    `SignalFrame` on the user stack, sets up a trampoline return address that
//!    will invoke `sigreturn`, and redirects execution to the signal handler.
//!
//! 2. **Restoration** (`restore_signal_frame`): Called from `sys_sigreturn` to
//!    read the saved `SignalFrame` from the user stack, restore registers, and
//!    resume execution at the point where the signal interrupted the thread.
//!
//! The implementation currently targets x86_64 as the primary platform, with
//! stub implementations for AArch64 and RISC-V.

#[allow(unused_imports)]
use crate::{error::KernelError, println, process::pcb::Process, process::thread::Thread};

/// Syscall number for SIG_RETURN (must match Syscall::SigReturn = 123).
const SYS_SIGRETURN: u64 = 123;

/// Signal handler value indicating default action.
const SIG_DFL: u64 = 0;
/// Signal handler value indicating the signal should be ignored.
const SIG_IGN: u64 = 1;

// ============================================================================
// Signal frame (x86_64)
// ============================================================================

/// Saved thread context pushed onto the user stack during signal delivery.
///
/// This is a C-compatible structure so that the signal handler trampoline can
/// pass a pointer to it back to `sys_sigreturn`. The layout must remain stable
/// across kernel versions for ABI compatibility.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SignalFrame {
    // -- Trampoline return address (at lowest address, i.e. where RSP points) --
    /// Address of the sigreturn trampoline code placed just after this struct.
    pub trampoline_ret_addr: u64,

    // -- Signal information --
    /// Signal number that caused this delivery.
    pub signum: u64,

    // -- Saved signal mask --
    /// The process signal mask at the time of delivery (restored on sigreturn).
    pub saved_mask: u64,

    // -- Saved general-purpose registers --
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,

    // -- Saved instruction pointer and flags --
    pub rip: u64,
    pub rflags: u64,
}

/// Size of the signal frame in bytes.
const SIGNAL_FRAME_SIZE: usize = core::mem::size_of::<SignalFrame>();

/// x86_64 sigreturn trampoline machine code.
///
/// This small code sequence is written onto the user stack just above the
/// signal frame. When the signal handler returns, it executes this trampoline
/// which calls `syscall(SYS_SIGRETURN, frame_ptr)`.
///
/// Assembly:
/// ```text
///   lea rdi, [rsp]      ; frame_ptr = current RSP (points to SignalFrame)
///   mov rax, 123         ; SYS_SIGRETURN
///   syscall
///   ud2                  ; should never reach here
/// ```
///
/// Encoded bytes (15 bytes):
///   48 8d 3c 24          lea rdi, [rsp]
///   48 c7 c0 7b 00 00 00 mov rax, 123
///   0f 05                syscall
///   0f 0b                ud2
#[cfg(target_arch = "x86_64")]
const SIGRETURN_TRAMPOLINE: [u8; 15] = [
    0x48, 0x8d, 0x3c, 0x24, // lea rdi, [rsp]
    0x48, 0xc7, 0xc0, 0x7b, 0x00, 0x00, 0x00, // mov rax, 123
    0x0f, 0x05, // syscall
    0x0f, 0x0b, // ud2
];

/// Size of the trampoline code in bytes.
#[cfg(target_arch = "x86_64")]
const TRAMPOLINE_SIZE: usize = SIGRETURN_TRAMPOLINE.len();

// ============================================================================
// Physical memory write/read helpers (same pattern as creation.rs)
// ============================================================================

/// Write a value to a user-space address via the physical memory window.
///
/// # Safety
///
/// `vaddr` must be a valid mapped address in the process's VAS with write
/// permissions. The caller must ensure no concurrent access to this memory.
#[cfg(feature = "alloc")]
unsafe fn write_to_user_stack(
    memory_space: &crate::mm::VirtualAddressSpace,
    vaddr: usize,
    value: usize,
) {
    use crate::mm::VirtualAddress;

    let pt_root = memory_space.get_page_table();
    if pt_root == 0 {
        return;
    }

    let mapper = unsafe { crate::mm::vas::create_mapper_from_root_pub(pt_root) };
    if let Ok((frame, _flags)) = mapper.translate_page(VirtualAddress(vaddr as u64)) {
        let page_offset = vaddr & 0xFFF;
        let phys_addr = (frame.as_u64() << 12) + page_offset as u64;
        // SAFETY: phys_addr is converted to a kernel-accessible virtual
        // address via phys_to_virt_addr (required on x86_64 where physical
        // memory is mapped at a dynamic offset, not identity-mapped).
        unsafe {
            let virt = crate::mm::phys_to_virt_addr(phys_addr);
            core::ptr::write(virt as *mut usize, value);
        }
    }
}

/// Write a byte slice to a user-space address via the physical memory window.
///
/// # Safety
///
/// Same requirements as `write_to_user_stack`. The range
/// `[vaddr, vaddr+data.len())` must be within a single mapped page.
#[cfg(feature = "alloc")]
unsafe fn write_bytes_to_user_stack(
    memory_space: &crate::mm::VirtualAddressSpace,
    vaddr: usize,
    data: &[u8],
) {
    use crate::mm::VirtualAddress;

    let pt_root = memory_space.get_page_table();
    if pt_root == 0 {
        return;
    }

    let mapper = unsafe { crate::mm::vas::create_mapper_from_root_pub(pt_root) };
    if let Ok((frame, _flags)) = mapper.translate_page(VirtualAddress(vaddr as u64)) {
        let page_offset = vaddr & 0xFFF;
        let phys_addr = (frame.as_u64() << 12) + page_offset as u64;
        // SAFETY: phys_addr is converted to a kernel-accessible virtual
        // address via phys_to_virt_addr. The destination has at least
        // data.len() bytes available within the page.
        unsafe {
            let virt = crate::mm::phys_to_virt_addr(phys_addr);
            core::ptr::copy_nonoverlapping(data.as_ptr(), virt as *mut u8, data.len());
        }
    }
}

/// Read a `usize` value from a user-space address via the physical memory
/// window.
///
/// # Safety
///
/// `vaddr` must be a valid mapped address in the process's VAS. The caller
/// must ensure no concurrent writes to this memory.
#[cfg(feature = "alloc")]
unsafe fn read_from_user_stack(
    memory_space: &crate::mm::VirtualAddressSpace,
    vaddr: usize,
) -> Option<usize> {
    use crate::mm::VirtualAddress;

    let pt_root = memory_space.get_page_table();
    if pt_root == 0 {
        return None;
    }

    let mapper = unsafe { crate::mm::vas::create_mapper_from_root_pub(pt_root) };
    if let Ok((frame, _flags)) = mapper.translate_page(VirtualAddress(vaddr as u64)) {
        let page_offset = vaddr & 0xFFF;
        let phys_addr = (frame.as_u64() << 12) + page_offset as u64;
        // SAFETY: phys_addr is converted to a kernel-accessible virtual
        // address via phys_to_virt_addr for reading.
        Some(unsafe {
            let virt = crate::mm::phys_to_virt_addr(phys_addr);
            core::ptr::read(virt as *const usize)
        })
    } else {
        None
    }
}

/// Read a byte slice from a user-space address via the physical memory window.
///
/// # Safety
///
/// `vaddr` must be a valid mapped address. The range `[vaddr, vaddr+len)` must
/// be within a single mapped page.
#[cfg(feature = "alloc")]
unsafe fn read_bytes_from_user_stack(
    memory_space: &crate::mm::VirtualAddressSpace,
    vaddr: usize,
    buf: &mut [u8],
) -> bool {
    use crate::mm::VirtualAddress;

    let pt_root = memory_space.get_page_table();
    if pt_root == 0 {
        return false;
    }

    let mapper = unsafe { crate::mm::vas::create_mapper_from_root_pub(pt_root) };
    if let Ok((frame, _flags)) = mapper.translate_page(VirtualAddress(vaddr as u64)) {
        let page_offset = vaddr & 0xFFF;
        let phys_addr = (frame.as_u64() << 12) + page_offset as u64;
        // SAFETY: phys_addr is converted to a kernel-accessible virtual
        // address via phys_to_virt_addr. We copy exactly buf.len() bytes.
        unsafe {
            let virt = crate::mm::phys_to_virt_addr(phys_addr);
            core::ptr::copy_nonoverlapping(virt as *const u8, buf.as_mut_ptr(), buf.len());
        }
        true
    } else {
        false
    }
}

// ============================================================================
// Signal delivery (x86_64)
// ============================================================================

/// Deliver a signal to a user-space handler by constructing a signal frame on
/// the user stack.
///
/// This function:
/// 1. Looks up the signal handler for `signum` in the process's handler table.
/// 2. If the handler is SIG_DFL (0) or SIG_IGN (1), handles the signal
///    in-kernel (terminate or ignore) and returns without modifying the thread.
/// 3. For a real handler address, saves the current thread context into a
///    `SignalFrame` on the user stack.
/// 4. Writes a sigreturn trampoline just above the frame.
/// 5. Sets the thread's RIP to the handler, RSP to the signal frame, and RDI to
///    the signal number (first argument per System V AMD64 ABI).
///
/// On success, the next time this thread returns to user space it will execute
/// the signal handler. When the handler returns, the trampoline calls
/// `sigreturn` which restores the original context.
///
/// # Arguments
/// - `process`: The process receiving the signal.
/// - `thread`: The thread whose context will be modified.
/// - `signum`: Signal number (1-31).
///
/// # Returns
/// - `Ok(true)` if a signal frame was constructed and the handler will run.
/// - `Ok(false)` if the signal was handled in-kernel (default/ignore).
/// - `Err(...)` on failure (invalid signal, no mapped stack, etc.).
#[cfg(feature = "alloc")]
pub fn deliver_signal(
    process: &Process,
    thread: &Thread,
    signum: usize,
) -> Result<bool, KernelError> {
    if signum == 0 || signum > 31 {
        return Err(KernelError::InvalidArgument {
            name: "signum",
            value: "signal number out of range (1-31)",
        });
    }

    // Look up the handler for this signal
    let handler = process.get_signal_handler(signum).unwrap_or(0);

    // Handle SIG_DFL and SIG_IGN in-kernel
    if handler == SIG_DFL {
        // Default action: for most signals, terminate the process.
        // SIGCHLD (17), SIGURG (23), SIGWINCH (28) are ignored by default.
        // For now, log and return false (caller decides termination).
        println!(
            "[SIGNAL] Signal {} for process {}: default action",
            signum, process.pid.0
        );
        return Ok(false);
    }

    if handler == SIG_IGN {
        // Explicitly ignored -- clear the pending bit and return.
        process.clear_pending_signal(signum);
        return Ok(false);
    }

    // We have a real handler address -- deliver via signal frame.
    deliver_signal_to_handler(process, thread, signum, handler)
}

/// Internal: construct the signal frame for a real handler address.
#[cfg(feature = "alloc")]
fn deliver_signal_to_handler(
    process: &Process,
    thread: &Thread,
    signum: usize,
    handler: u64,
) -> Result<bool, KernelError> {
    // Architecture-specific delivery
    #[cfg(target_arch = "x86_64")]
    {
        deliver_signal_x86_64(process, thread, signum, handler)
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Stub: AArch64 signal delivery not yet implemented
        let _ = (process, thread, signum, handler);
        println!("[SIGNAL] AArch64 signal delivery not yet implemented");
        Ok(false)
    }

    #[cfg(target_arch = "riscv64")]
    {
        // Stub: RISC-V signal delivery not yet implemented
        let _ = (process, thread, signum, handler);
        println!("[SIGNAL] RISC-V signal delivery not yet implemented");
        Ok(false)
    }
}

/// x86_64 signal delivery implementation.
///
/// Stack layout after delivery (growing downward):
///
/// ```text
/// [original RSP]                 <- where the thread was before signal
///   ...
/// [trampoline code, 14 bytes]    <- trampoline_addr
///   [padding for 16-byte align]
/// [SignalFrame]                   <- new RSP
///   .trampoline_ret_addr = trampoline_addr
///   .signum
///   .saved_mask
///   .rax..r15
///   .rip
///   .rflags
/// ```
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
fn deliver_signal_x86_64(
    process: &Process,
    thread: &Thread,
    signum: usize,
    handler: u64,
) -> Result<bool, KernelError> {
    use core::sync::atomic::Ordering;

    let memory_space = process.memory_space.lock();
    let mut ctx = thread.context.lock();

    // Save current register state
    let saved_rip = ctx.rip;
    let saved_rsp = ctx.rsp;
    let saved_rflags = ctx.rflags;
    let saved_rax = ctx.rax;
    let saved_rbx = ctx.rbx;
    let saved_rcx = ctx.rcx;
    let saved_rdx = ctx.rdx;
    let saved_rsi = ctx.rsi;
    let saved_rdi = ctx.rdi;
    let saved_rbp = ctx.rbp;
    let saved_r8 = ctx.r8;
    let saved_r9 = ctx.r9;
    let saved_r10 = ctx.r10;
    let saved_r11 = ctx.r11;
    let saved_r12 = ctx.r12;
    let saved_r13 = ctx.r13;
    let saved_r14 = ctx.r14;
    let saved_r15 = ctx.r15;

    // Save and update signal mask: block the delivered signal during handler
    let saved_mask = process.signal_mask.load(Ordering::Acquire);
    let blocked_during_handler = saved_mask | (1u64 << signum);
    // Cannot mask SIGKILL (9) or SIGSTOP (19)
    let sanitized = blocked_during_handler & !((1u64 << 9) | (1u64 << 19));
    process.signal_mask.store(sanitized, Ordering::Release);

    // Clear the pending signal bit (we are delivering it now)
    process.clear_pending_signal(signum);

    // Calculate stack positions for the signal frame and trampoline.
    // User stack grows downward from saved_rsp.
    let mut sp = saved_rsp as usize;

    // 1. Write trampoline code at top (below current SP)
    sp -= TRAMPOLINE_SIZE;
    // Align trampoline start to 2-byte boundary (for code fetch efficiency)
    sp &= !1;
    let trampoline_addr = sp;

    // SAFETY: sp is within the user stack region and is mapped in the
    // process's page tables. We write the trampoline machine code bytes.
    unsafe {
        write_bytes_to_user_stack(&memory_space, trampoline_addr, &SIGRETURN_TRAMPOLINE);
    }

    // 2. Allocate space for SignalFrame below the trampoline
    sp -= SIGNAL_FRAME_SIZE;
    // Align to 16 bytes (x86_64 ABI requirement for stack alignment)
    sp &= !0xF;
    let frame_addr = sp;

    // 3. Build the signal frame
    let frame = SignalFrame {
        trampoline_ret_addr: trampoline_addr as u64,
        signum: signum as u64,
        saved_mask,
        rax: saved_rax,
        rbx: saved_rbx,
        rcx: saved_rcx,
        rdx: saved_rdx,
        rsi: saved_rsi,
        rdi: saved_rdi,
        rbp: saved_rbp,
        rsp: saved_rsp,
        r8: saved_r8,
        r9: saved_r9,
        r10: saved_r10,
        r11: saved_r11,
        r12: saved_r12,
        r13: saved_r13,
        r14: saved_r14,
        r15: saved_r15,
        rip: saved_rip,
        rflags: saved_rflags,
    };

    // 4. Write the signal frame to the user stack
    // SAFETY: frame_addr is within the user stack, aligned to 16 bytes, and
    // mapped in the process's page tables. We write the entire SignalFrame
    // as a byte slice.
    unsafe {
        let frame_bytes = core::slice::from_raw_parts(
            &frame as *const SignalFrame as *const u8,
            SIGNAL_FRAME_SIZE,
        );
        write_bytes_to_user_stack(&memory_space, frame_addr, frame_bytes);
    }

    // 5. Set up the thread context to execute the signal handler.
    //    - RIP = handler address
    //    - RSP = frame_addr (handler's stack; return addr is at [RSP] which is
    //      trampoline_ret_addr in the SignalFrame)
    //    - RDI = signum (first argument, System V AMD64 ABI)
    ctx.rip = handler;
    ctx.rsp = frame_addr as u64;
    ctx.rdi = signum as u64;

    // Clear direction flag and ensure interrupts are enabled in user mode
    ctx.rflags = (ctx.rflags & !0x400) | 0x200; // clear DF, set IF

    println!(
        "[SIGNAL] Delivered signal {} to process {} handler {:#x}, frame at {:#x}",
        signum, process.pid.0, handler, frame_addr
    );

    Ok(true)
}

// ============================================================================
// Signal frame restoration (sigreturn)
// ============================================================================

/// Restore the original thread context from a signal frame on the user stack.
///
/// Called by `sys_sigreturn` after the signal handler returns. Reads the
/// `SignalFrame` from the user stack and restores all saved registers and the
/// signal mask.
///
/// # Arguments
/// - `process`: The process whose signal mask will be restored.
/// - `thread`: The thread whose context will be restored.
/// - `frame_ptr`: User-space pointer to the `SignalFrame` (passed by the
///   trampoline via RDI).
///
/// # Returns
/// - `Ok(())` on success (thread context is restored, execution will resume at
///   the interrupted instruction).
/// - `Err(...)` if the frame cannot be read.
#[cfg(feature = "alloc")]
pub fn restore_signal_frame(
    process: &Process,
    thread: &Thread,
    frame_ptr: usize,
) -> Result<(), KernelError> {
    #[cfg(target_arch = "x86_64")]
    {
        restore_signal_frame_x86_64(process, thread, frame_ptr)
    }

    #[cfg(target_arch = "aarch64")]
    {
        let _ = (process, thread, frame_ptr);
        Err(KernelError::NotImplemented {
            feature: "signal frame restore (aarch64)",
        })
    }

    #[cfg(target_arch = "riscv64")]
    {
        let _ = (process, thread, frame_ptr);
        Err(KernelError::NotImplemented {
            feature: "signal frame restore (riscv64)",
        })
    }
}

/// x86_64 signal frame restoration.
#[cfg(all(feature = "alloc", target_arch = "x86_64"))]
fn restore_signal_frame_x86_64(
    process: &Process,
    thread: &Thread,
    frame_ptr: usize,
) -> Result<(), KernelError> {
    use core::sync::atomic::Ordering;

    let memory_space = process.memory_space.lock();

    // Read the signal frame from the user stack
    let mut frame_bytes = [0u8; SIGNAL_FRAME_SIZE];
    // SAFETY: frame_ptr was passed from the trampoline and points to a
    // SignalFrame we previously wrote. We read it back via the physical
    // memory window.
    let ok = unsafe { read_bytes_from_user_stack(&memory_space, frame_ptr, &mut frame_bytes) };

    if !ok {
        return Err(KernelError::InvalidArgument {
            name: "frame_ptr",
            value: "could not read signal frame from user stack",
        });
    }

    // SAFETY: frame_bytes contains SIGNAL_FRAME_SIZE bytes that we read from
    // the user stack. We reinterpret them as a SignalFrame. The struct is
    // repr(C) and all fields are plain u64 values, so any bit pattern is
    // valid. We copy the struct by value.
    let frame: SignalFrame = unsafe { core::ptr::read(frame_bytes.as_ptr() as *const SignalFrame) };

    // Restore the thread context
    {
        let mut ctx = thread.context.lock();
        ctx.rax = frame.rax;
        ctx.rbx = frame.rbx;
        ctx.rcx = frame.rcx;
        ctx.rdx = frame.rdx;
        ctx.rsi = frame.rsi;
        ctx.rdi = frame.rdi;
        ctx.rbp = frame.rbp;
        ctx.rsp = frame.rsp;
        ctx.r8 = frame.r8;
        ctx.r9 = frame.r9;
        ctx.r10 = frame.r10;
        ctx.r11 = frame.r11;
        ctx.r12 = frame.r12;
        ctx.r13 = frame.r13;
        ctx.r14 = frame.r14;
        ctx.r15 = frame.r15;
        ctx.rip = frame.rip;
        ctx.rflags = frame.rflags;
    }

    // Restore the signal mask
    // Cannot unmask SIGKILL (9) or SIGSTOP (19)
    let restored_mask = frame.saved_mask & !((1u64 << 9) | (1u64 << 19));
    process.signal_mask.store(restored_mask, Ordering::Release);

    println!(
        "[SIGNAL] Restored signal frame for process {}, resuming at {:#x}",
        process.pid.0, frame.rip
    );

    Ok(())
}

// ============================================================================
// Pending signal check (called from syscall return path)
// ============================================================================

/// Check for and deliver any pending signals on the current process/thread.
///
/// This function should be called on the syscall return path (or on return
/// from interrupt) to deliver signals at a safe point. It dequeues the
/// lowest-numbered pending unblocked signal and, if a user-space handler is
/// registered, constructs a signal frame so the handler executes on return
/// to user mode.
///
/// # Returns
/// - `Ok(true)` if a signal was delivered (thread context was modified).
/// - `Ok(false)` if no deliverable signal was pending.
/// - `Err(...)` on failure.
#[cfg(feature = "alloc")]
pub fn check_pending_signals() -> Result<bool, KernelError> {
    let process =
        crate::process::current_process().ok_or(KernelError::ProcessNotFound { pid: 0 })?;
    let thread = crate::process::current_thread().ok_or(KernelError::ThreadNotFound { tid: 0 })?;

    // Get next pending unblocked signal
    if let Some(signum) = process.get_next_pending_signal() {
        deliver_signal(process, thread, signum)
    } else {
        Ok(false)
    }
}

#[cfg(not(feature = "alloc"))]
pub fn check_pending_signals() -> Result<bool, KernelError> {
    Ok(false)
}

#[cfg(not(feature = "alloc"))]
pub fn deliver_signal(
    _process: &Process,
    _thread: &Thread,
    _signum: usize,
) -> Result<bool, KernelError> {
    Err(KernelError::NotImplemented {
        feature: "signal delivery (requires alloc)",
    })
}

#[cfg(not(feature = "alloc"))]
pub fn restore_signal_frame(
    _process: &Process,
    _thread: &Thread,
    _frame_ptr: usize,
) -> Result<(), KernelError> {
    Err(KernelError::NotImplemented {
        feature: "signal frame restore (requires alloc)",
    })
}

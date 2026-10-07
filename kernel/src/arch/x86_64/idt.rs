//! Interrupt Descriptor Table
//!
//! Sets up handlers for CPU exceptions (breakpoint, page fault, GPF,
//! double fault) and hardware interrupts (timer). Fatal exception
//! handlers log diagnostic information and halt the CPU instead of
//! panicking, which avoids triggering a double fault from within an
//! interrupt context.

use lazy_static::lazy_static;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

/// Make the per-CPU block the active GS base, which is what
/// `boot_return_to_kernel` expects (it was written for the state inside a
/// syscall, and executes `swapgs` itself). An exception from user mode
/// arrives with the user's GS active and needs a `swapgs`; a kernel-mode
/// fault inside a syscall already has the per-CPU block and must not swap,
/// or the next syscall reads its kernel stack through the user's GS (review
/// of the v0.26.0 stack, PR #14).
///
/// # Safety
///
/// Ring 0 only, with interrupts disabled, immediately before
/// `boot_return_to_kernel`.
unsafe fn gs_to_syscall_state() {
    let gs_base = x86_64::registers::model_specific::GsBase::read().as_u64();
    if !crate::arch::percpu::is_arch_cpu_block(gs_base) {
        // SAFETY: the caller's contract; swapgs only exchanges the GS base
        // MSRs.
        unsafe { core::arch::asm!("swapgs", options(nomem, nostack)) };
    }
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        // Set IST on ALL exception vectors that can fire from Ring 3 to
        // prevent DF escalation when TSS.RSP0 is stale/unmapped. Any
        // exception without IST falls back to RSP0 for the privilege
        // switch, which fails if the process CR3 doesn't map RSP0.
        //
        // SAFETY: GENERAL_IST_INDEX (2) is a valid IST index configured
        // in the TSS. Shared across these low-frequency exception handlers.
        unsafe {
            idt.divide_error
                .set_handler_fn(divide_error_handler)
                .set_stack_index(crate::arch::x86_64::gdt::GENERAL_IST_INDEX);
            idt.invalid_opcode
                .set_handler_fn(invalid_opcode_handler)
                .set_stack_index(crate::arch::x86_64::gdt::GENERAL_IST_INDEX);
            idt.segment_not_present
                .set_handler_fn(segment_not_present_handler)
                .set_stack_index(crate::arch::x86_64::gdt::GENERAL_IST_INDEX);
            idt.stack_segment_fault
                .set_handler_fn(stack_segment_fault_handler)
                .set_stack_index(crate::arch::x86_64::gdt::GENERAL_IST_INDEX);
            idt.alignment_check
                .set_handler_fn(alignment_check_handler)
                .set_stack_index(crate::arch::x86_64::gdt::GENERAL_IST_INDEX);
            idt.security_exception
                .set_handler_fn(security_exception_handler)
                .set_stack_index(crate::arch::x86_64::gdt::GENERAL_IST_INDEX);
        }
        // SAFETY: PAGE_FAULT_IST_INDEX (1) is a valid IST index configured in
        // the TSS during GDT initialization. Using a dedicated IST stack for
        // page faults is REQUIRED for Ring 3 fault handling: when a page fault
        // occurs in user mode, the CPU normally loads RSP from TSS.RSP0 to
        // switch to the kernel stack. If TSS.RSP0 is stale or the address is
        // not mapped in the current process's CR3, the CPU cannot push the
        // exception frame and escalates to a Double Fault. IST bypasses
        // TSS.RSP0 entirely, loading the stack pointer from the TSS IST entry
        // unconditionally.
        unsafe {
            idt.page_fault
                .set_handler_fn(page_fault_handler)
                .set_stack_index(crate::arch::x86_64::gdt::PAGE_FAULT_IST_INDEX);
        }
        // SAFETY: GENERAL_IST_INDEX (2) is a valid IST index configured in
        // the TSS during GDT initialization. Using a dedicated IST stack for
        // GPF is REQUIRED for Ring 3 fault handling: when a GPF occurs in
        // user mode, the CPU normally loads RSP from TSS.RSP0. If TSS.RSP0
        // is stale or unmapped in the current process's CR3, the CPU cannot
        // push the exception frame and escalates to a Double Fault.
        unsafe {
            idt.general_protection_fault
                .set_handler_fn(general_protection_fault_handler)
                .set_stack_index(crate::arch::x86_64::gdt::GENERAL_IST_INDEX);
        }
        // SAFETY: DOUBLE_FAULT_IST_INDEX is a valid IST index that was set up
        // during GDT initialization. Using a dedicated interrupt stack prevents
        // a triple fault when the kernel stack is corrupted.
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(crate::arch::x86_64::gdt::DOUBLE_FAULT_IST_INDEX);
        }
        // Hardware interrupt handlers all use IST to bypass TSS.RSP0 for
        // Ring 3 delivery. Without IST, a timer/keyboard/IPI interrupt firing
        // while user code is running loads RSP from TSS.RSP0. If RSP0 is stale
        // or unmapped in the process's page tables, the CPU cannot push the
        // interrupt frame and escalates to a Double Fault.
        //
        // SAFETY: HARDWARE_IRQ_IST_INDEX (3) is a valid IST index configured
        // in the TSS during GDT initialization.
        unsafe {
            idt[32]
                .set_handler_fn(timer_interrupt_handler)
                .set_stack_index(crate::arch::x86_64::gdt::HARDWARE_IRQ_IST_INDEX);
            idt[33]
                .set_handler_fn(keyboard_interrupt_handler)
                .set_stack_index(crate::arch::x86_64::gdt::HARDWARE_IRQ_IST_INDEX);
            idt[48]
                .set_handler_fn(apic_timer_interrupt_handler)
                .set_stack_index(crate::arch::x86_64::gdt::HARDWARE_IRQ_IST_INDEX);
            idt[49]
                .set_handler_fn(tlb_shootdown_handler)
                .set_stack_index(crate::arch::x86_64::gdt::HARDWARE_IRQ_IST_INDEX);
            idt[50]
                .set_handler_fn(sched_wake_handler)
                .set_stack_index(crate::arch::x86_64::gdt::HARDWARE_IRQ_IST_INDEX);
        }
        idt
    };
}

pub fn init() {
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    // Raw serial output ONLY -- println! uses spinlocks which deadlock in
    // interrupt context, causing re-entrant DF cascades.
    // SAFETY: Writing to COM1 data register at I/O port 0x3F8 is safe
    // for diagnostics.
    unsafe {
        raw_serial_str(b"FATAL:DF rip=0x");
        // Print the instruction pointer from the exception frame
        let rip = stack_frame.instruction_pointer.as_u64();
        raw_serial_hex(rip);
        raw_serial_str(b" rsp=0x");
        let rsp = stack_frame.stack_pointer.as_u64();
        raw_serial_hex(rsp);
        // Read CR2 (faulting address from the page fault that triggered DF)
        let cr2: u64;
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack));
        raw_serial_str(b" cr2=0x");
        raw_serial_hex(cr2);
        // Read CS from the stack frame to determine privilege level
        let cs = stack_frame.code_segment.0;
        raw_serial_str(b" cs=0x");
        raw_serial_hex(cs as u64);
        raw_serial_str(b"\n");
        // Print current CR3 and TSS.RSP0 for diagnosis
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        raw_serial_str(b"  CR3=0x");
        raw_serial_hex(cr3);
        raw_serial_str(b" TSS_RSP0=0x");
        raw_serial_hex(crate::arch::x86_64::gdt::get_kernel_stack());
        raw_serial_str(b"\n");
    }

    loop {
        x86_64::instructions::hlt();
    }
}

/// Resolve a page fault through `mm::page_fault` on a 16-byte-aligned
/// stack: the full user-fault path (demand paging, CoW, stack growth, then
/// SIGSEGV), or for `kernel_copy` the narrow, signal-free
/// `resolve_user_copy_fault`.
///
/// Exceptions that push an error code enter the handler with RSP 8 bytes
/// off the alignment compiled code assumes, so deep Rust code reached from
/// the handler can fault on an aligned SSE store (`movaps`) -- observed as a
/// GP inside `handle_page_fault` when called for a kernel-mode user-copy
/// fault. The trampoline realigns RSP before the call and restores it.
fn resolve_fault_aligned(ec: u64, cr2: u64, rip: u64, kernel_copy: bool) -> bool {
    extern "C" fn resolve_user(ec: u64, cr2: u64, rip: u64) -> u64 {
        let info = crate::mm::page_fault::from_x86_64(ec, cr2, rip);
        crate::mm::page_fault::handle_page_fault(info).is_ok() as u64
    }
    extern "C" fn resolve_copy(ec: u64, cr2: u64, rip: u64) -> u64 {
        let info = crate::mm::page_fault::from_x86_64(ec, cr2, rip);
        crate::mm::page_fault::resolve_user_copy_fault(&info) as u64
    }
    let resolve: extern "C" fn(u64, u64, u64) -> u64 = if kernel_copy {
        resolve_copy
    } else {
        resolve_user
    };
    let resolved: u64;
    // SAFETY: r12 is callee-saved under the C ABI, so it survives the call
    // and restores the original RSP; `and rsp, -16` only moves RSP down into
    // the same stack. All other caller-saved state is declared clobbered.
    unsafe {
        core::arch::asm!(
            "mov r12, rsp",
            "and rsp, -16",
            "call {f}",
            "mov rsp, r12",
            f = in(reg) resolve,
            in("rdi") ec,
            in("rsi") cr2,
            in("rdx") rip,
            lateout("rax") resolved,
            out("r12") _,
            clobber_abi("C"),
        );
    }
    resolved != 0
}

extern "x86-interrupt" fn page_fault_handler(
    // Mutated only by the bare-metal user-copy fixup.
    #[cfg_attr(not(target_os = "none"), allow(unused_mut))] mut stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    // FIRST: Emit a raw serial byte to prove we reached the handler.
    // This comes before ANY other operation to diagnose IST issues.
    // SAFETY: Port I/O write to COM1 for diagnostic.
    unsafe {
        core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'!', options(nomem, nostack));
    }

    // SAFETY: Read CR2 FIRST — before any Rust function call that might
    // trigger a secondary page fault and overwrite it.
    let cr2_val: u64 = unsafe {
        let val: u64;
        core::arch::asm!("mov {}, cr2", out(reg) val, options(nomem, nostack));
        val
    };

    let ec = error_code.bits();
    let rip_val = stack_frame.instruction_pointer.as_u64();
    let was_user = ec & 4 != 0; // U/S bit

    // Raw serial diagnostic BEFORE any Rust function calls (count_page_fault,
    // trace!, current_process). Those calls use global state that may not be
    // accessible from the IST/TSS stack and can trigger secondary faults
    // escalating to Double Fault.
    // SAFETY: Port I/O writes to COM1 (0x3F8) for diagnostic serial output.
    unsafe {
        raw_serial_str(b"PF! cr2=0x");
        raw_serial_hex(cr2_val);
        raw_serial_str(b" ec=0x");
        raw_serial_hex(ec);
        raw_serial_str(b" rip=0x");
        raw_serial_hex(rip_val);
        raw_serial_str(b"\n");
    }

    // Now safe to call Rust functions — we already have CR2 and diagnostics.
    crate::perf::count_page_fault();

    // --- Kernel-mode fault inside the fault-tolerant user copy ---
    //
    // A syscall touching user memory through usercopy::copy_user (the
    // validated accessors in syscall::userspace) faulted. Resolve it like a
    // user fault if it is a not-yet-populated page (W-3); otherwise resume
    // at the fixup so the copy returns EFAULT and the syscall unwinds
    // normally, releasing its locks (MEM-SEC-01, SYS-SEC-01, W-12).
    #[cfg(target_os = "none")]
    if !was_user && cr2_val < 0x0000_8000_0000_0000 {
        if let Some(fixup) = crate::arch::x86_64::usercopy::fixup_for(rip_val) {
            if cr2_val >= 0x1000 && resolve_fault_aligned(ec, cr2_val, rip_val, true) {
                return; // page now present: retry the copy instruction
            }
            // Redirect the saved RIP to the fixup stub of the routine that
            // faulted; the fixup only sets rax and returns to copy_user's
            // caller, whose stack frame is intact. A single 8-byte store:
            // Volatile::update copies the frame through SSE registers, which
            // faults on this handler's 8-byte-misaligned stack.
            // SAFETY: the x86-interrupt ABI passes `stack_frame` in place
            // (it is the hardware frame); instruction_pointer is its first
            // field, a u64 VirtAddr.
            unsafe {
                core::ptr::write_volatile(core::ptr::addr_of_mut!(stack_frame) as *mut u64, fixup);
            }
            return;
        }
    }

    // --- Kernel-mode fault at user address (fast path) ---
    //
    // Any other kernel-mode access to user memory (code that bypasses the
    // fault-tolerant accessors) is still abandoned via the boot context.
    //
    // Handle this BEFORE demand paging. A kernel-mode fault (U/S bit clear)
    // at a user-space address means a syscall handler tried to access
    // unmapped user memory. We MUST NOT call current_process(),
    // current_thread(), or handle_page_fault() here because:
    //   - current_process() calls SCHEDULER.lock() which may already be held,
    //     causing spinlock deadlock on the IST stack -> GP fault.
    //   - handle_page_fault() calls current_process() + memory_space.lock() with
    //     the same deadlock risk.
    //
    // Instead, print a diagnostic and return to boot context directly.
    // The thread that caused the fault will not be marked Zombie (no safe
    // way to do so without locks), but boot_return_to_kernel() restores
    // the kernel to a known-good state.
    if !was_user && cr2_val < 0x0000_8000_0000_0000 {
        // SAFETY: Port I/O writes to COM1 for diagnostics.
        unsafe {
            raw_serial_str(b"KERN_PF_USER_ADDR cr2=0x");
            raw_serial_hex(cr2_val);
            raw_serial_str(b" rip=0x");
            raw_serial_hex(rip_val);
            // Print last syscall info (atomics, no locks needed)
            raw_serial_str(b" sc=0x");
            raw_serial_hex(
                crate::syscall::LAST_SYSCALL_NUM.load(core::sync::atomic::Ordering::Relaxed),
            );
            raw_serial_str(b" a1=0x");
            raw_serial_hex(
                crate::syscall::LAST_SYSCALL_ARG1.load(core::sync::atomic::Ordering::Relaxed),
            );
            raw_serial_str(b" a2=0x");
            raw_serial_hex(
                crate::syscall::LAST_SYSCALL_ARG2.load(core::sync::atomic::Ordering::Relaxed),
            );
            raw_serial_str(b"\n");
        }
        if crate::arch::x86_64::usermode::has_boot_return_context() {
            // SAFETY: GS is put in the state boot_return_to_kernel expects
            // (per-CPU block active). A kernel-mode fault inside a syscall
            // already has it, so an unconditional swapgs here inverted it.
            unsafe {
                gs_to_syscall_state();
                crate::arch::x86_64::usermode::boot_return_to_kernel();
            }
        }
        // No boot context -- halt (should not happen during normal syscall path).
        loop {
            x86_64::instructions::hlt();
        }
    }

    // --- User-mode demand paging ---
    //
    // Only attempt demand paging for USER-MODE faults (U/S bit set in ec).
    // Skip NULL dereferences (addr < PAGE_SIZE) since no valid mapping can
    // exist there.
    //
    // The demand paging code uses try_lock() on process.memory_space to avoid
    // deadlock from IST interrupt context.
    if was_user && cr2_val >= 0x1000 && resolve_fault_aligned(ec, cr2_val, rip_val, false) {
        // Fault resolved (demand page, CoW, or stack growth) -- resume.
        return;
    }

    // --- Unresolvable fault ---
    //
    // Print diagnostics via raw serial, then halt or kill the process.
    // SAFETY: Writing to COM1 data register at I/O port 0x3F8 for diagnostics.
    unsafe {
        raw_serial_str(b"PF@0x");
        raw_serial_hex(cr2_val);
        raw_serial_str(b" ec=0x");
        raw_serial_hex(ec);
        raw_serial_str(b" rip=0x");
        raw_serial_hex(rip_val);
        raw_serial_str(b"\n");
    }

    if was_user {
        // User-mode fault: unresolvable. Kill the process directly.
        // Cannot call sys_exit() from interrupt context (it uses println!
        // and locks which risk deadlock). Instead, mark the process as
        // Zombie and call boot_return_to_kernel directly.
        // SAFETY: Port I/O writes to COM1 (0x3F8) for diagnostic serial output.
        unsafe {
            raw_serial_str(b"SEGFAULT addr=0x");
            raw_serial_hex(cr2_val);
            raw_serial_str(b" rip=0x");
            raw_serial_hex(rip_val);
            raw_serial_str(b"\n");
        }

        // Dump user stack to identify the call chain at crash time.
        // Since we don't switch CR3, user pages are mapped.
        // SAFETY: User stack is mapped (no CR3 switch). RSP is bounds-checked
        // before dereferencing. Port I/O to COM1 for serial output.
        unsafe {
            let user_rsp = stack_frame.stack_pointer.as_u64();
            raw_serial_str(b"  RSP=0x");
            raw_serial_hex(user_rsp);
            raw_serial_str(b"\n");
            // Dump first 12 qwords from the user stack
            if user_rsp > 0x1000 && user_rsp < 0x0000_8000_0000_0000 {
                for i in 0u64..12 {
                    let addr = user_rsp + i * 8;
                    let val = *(addr as *const u64);
                    raw_serial_str(b"  [RSP+0x");
                    raw_serial_hex(i * 8);
                    raw_serial_str(b"]=0x");
                    raw_serial_hex(val);
                    raw_serial_str(b"\n");
                }
            }
        }

        // Mark the faulting thread (or process) as Zombie before returning
        // to boot context. Only use atomic state operations.
        // Do NOT iterate threads BTreeMap or look up parent via
        // get_process() -- those BTreeMap operations GP fault from
        // interrupt context on the TSS stack.
        // For CLONE_THREAD children, only mark the thread as Zombie
        // so the parent and other threads survive.
        if let Some(thread) = crate::process::current_thread() {
            thread.set_state(crate::process::thread::ThreadState::Zombie);
        } else if let Some(process) = crate::process::current_process() {
            process.set_term_signal(11); // killed by SIGSEGV (N-99)
            process.set_state(crate::process::pcb::ProcessState::Zombie);
        }

        // Return to boot context.
        // The page fault handler runs in interrupt context (no swapgs on
        // entry). boot_return_to_kernel expects the swapgs state from
        // syscall_entry. Do swapgs first to balance boot_return's swapgs.
        if crate::arch::x86_64::usermode::has_boot_return_context() {
            // SAFETY: GS is put in the state boot_return_to_kernel expects;
            // boot_return context was verified by has_boot_return_context().
            unsafe {
                gs_to_syscall_state();
                crate::arch::x86_64::usermode::boot_return_to_kernel();
            }
        }
        // No boot context -- halt.
        loop {
            x86_64::instructions::hlt();
        }
    }

    // Kernel-mode fault at kernel address -- unrecoverable.
    // Use raw serial ONLY to avoid println! triggering secondary faults.
    // SAFETY: Port I/O writes to COM1 for diagnostics.
    unsafe {
        raw_serial_str(b"FATAL: kernel page fault at 0x");
        raw_serial_hex(cr2_val);
        raw_serial_str(b" ec=0x");
        raw_serial_hex(ec);
        raw_serial_str(b" rip=0x");
        raw_serial_hex(rip_val);
        raw_serial_str(b"\n");
    }
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    // All output via raw serial to avoid spinlock deadlocks.
    // SAFETY: Writing to COM1 data register at I/O port 0x3F8 is safe for
    // diagnostics. We bypass the serial spinlock because we may have
    // interrupted code that holds it.
    unsafe {
        raw_serial_str(b"FATAL:GP err=0x");
        raw_serial_hex(error_code);
        raw_serial_str(b"\n");

        // Read the saved interrupt frame directly from the stack.
        // The x86_64 CPU pushes [SS, RSP, RFLAGS, CS, RIP] and the error
        // code. The x86-interrupt calling convention passes us a reference
        // to the saved frame. We cast through the InterruptStackFrame
        // (which is a repr(C) wrapper around the saved values) to get at
        // the raw u64 fields. The frame layout (from low address):
        //   [0]: RIP, [1]: CS, [2]: RFLAGS, [3]: RSP, [4]: SS
        let frame_base = &stack_frame as *const _ as *const u64;
        raw_serial_str(b"RIP=0x");
        raw_serial_hex(core::ptr::read_volatile(frame_base));
        raw_serial_str(b" CS=0x");
        raw_serial_hex(core::ptr::read_volatile(frame_base.add(1)));
        raw_serial_str(b"\n");
        raw_serial_str(b"RFLAGS=0x");
        raw_serial_hex(core::ptr::read_volatile(frame_base.add(2)));
        raw_serial_str(b"\nRSP=0x");
        raw_serial_hex(core::ptr::read_volatile(frame_base.add(3)));
        raw_serial_str(b" SS=0x");
        raw_serial_hex(core::ptr::read_volatile(frame_base.add(4)));
        raw_serial_str(b"\n");

        // Also print CR2 and CR3 for diagnosis (CR2 may hold a stale
        // page-fault address that helps identify cascading faults).
        let cr2: u64;
        let cr3: u64;
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack));
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        raw_serial_str(b"CR2=0x");
        raw_serial_hex(cr2);
        raw_serial_str(b" CR3=0x");
        raw_serial_hex(cr3);
        raw_serial_str(b"\n");
    }

    // Check if the fault was from user mode (CS RPL = 3).
    let cs = stack_frame.code_segment.0;
    let was_user = (cs & 3) == 3;
    if was_user {
        // User-mode GPF: kill the process and return to boot context.
        // SAFETY: Port I/O to COM1 for diagnostic output.
        unsafe {
            raw_serial_str(b"[GP_KILL] user-mode GPF, terminating process\n");
        }

        if let Some(process) = crate::process::current_process() {
            process.set_term_signal(11); // killed by SIGSEGV (N-99)
            process.set_state(crate::process::pcb::ProcessState::Zombie);
        }

        if crate::arch::x86_64::usermode::has_boot_return_context() {
            // SAFETY: GS is put in the state boot_return_to_kernel expects.
            unsafe {
                raw_serial_str(b"[GP_KILL] boot_return\n");
                gs_to_syscall_state();
                crate::arch::x86_64::usermode::boot_return_to_kernel();
            }
        }
    }

    loop {
        x86_64::instructions::hlt();
    }
}

/// Write a byte string to COM1 serial, bypassing all locks.
///
/// # Safety
/// Port 0x3F8 must be a valid COM1 data register.
pub(crate) unsafe fn raw_serial_str(s: &[u8]) {
    for &b in s {
        // SAFETY: forwarded from this function's contract: port 0x3F8 is the
        // COM1 data register, so the OUT only transmits a byte.
        unsafe {
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
        }
    }
}

/// Write a u64 as hex to COM1 serial, bypassing all locks.
///
/// # Safety
/// Port 0x3F8 must be a valid COM1 data register.
pub(crate) unsafe fn raw_serial_hex(val: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    // Print 16 hex digits (skip leading zeros after first nonzero)
    let mut started = false;
    for i in (0..16).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as usize;
        if nibble != 0 || started || i == 0 {
            started = true;
            let b = HEX[nibble];
            // SAFETY: forwarded from this function's contract: port 0x3F8 is
            // the COM1 data register, so the OUT only transmits a byte.
            unsafe {
                core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
            }
        }
    }
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    crate::perf::count_interrupt();

    // Notify the scheduler of a timer tick for preemptive scheduling.
    // Use try_lock to avoid deadlock: if the scheduler lock is already held
    // (e.g., we interrupted mid-schedule), skip the tick — the holder will
    // complete its scheduling decision and release the lock.
    if let Some(mut sched) = crate::sched::scheduler::current_scheduler().try_lock() {
        sched.tick();
    }

    // SAFETY: Writing the EOI (End of Interrupt) byte (0x20) to the master
    // PIC command port (0x20) is required to acknowledge the timer interrupt.
    // Failing to send EOI would mask all further IRQs at this priority level.
    unsafe {
        use x86_64::instructions::port::Port;
        let mut pic_command: Port<u8> = Port::new(0x20);
        pic_command.write(0x20); // EOI command
    }
}

extern "x86-interrupt" fn apic_timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    crate::perf::count_interrupt();

    // Increment the global tick counter (atomic, always safe from interrupt
    // context).
    super::timer::tick();

    // Send APIC End-Of-Interrupt (NOT PIC EOI -- APIC timer uses its own EOI path).
    crate::arch::x86_64::apic::send_eoi();
}

extern "x86-interrupt" fn tlb_shootdown_handler(_stack_frame: InterruptStackFrame) {
    // Flush the local CPU's TLB in response to a remote CPU modifying shared
    // page tables. On single-CPU systems this handler is never invoked, but
    // it is registered for correctness when SMP is enabled.
    crate::mm::tlb::service_pending();
    crate::arch::x86_64::apic::send_eoi();
}

extern "x86-interrupt" fn sched_wake_handler(_stack_frame: InterruptStackFrame) {
    // Wake handler: a remote CPU placed a task on our run queue and sent this
    // IPI to break us out of HLT. No action needed beyond EOI -- the scheduler
    // will pick up the new task on the next scheduling decision.
    crate::arch::percpu::note_ipi();
    crate::arch::x86_64::apic::send_eoi();
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    crate::perf::count_interrupt();

    // Read scancode from PS/2 data port (0x60) and forward to keyboard driver.
    // This handler must NOT call println! or acquire any spinlock used by
    // the serial/fbcon output path.
    // SAFETY: Port 0x60 is the PS/2 keyboard data port. Reading it clears
    // the keyboard controller's output buffer.
    let scancode: u8 = unsafe {
        use x86_64::instructions::port::Port;
        Port::<u8>::new(0x60).read()
    };
    crate::drivers::keyboard::handle_scancode(scancode);
    // SAFETY: EOI to PIC1 (port 0x20) acknowledges the keyboard interrupt.
    unsafe {
        use x86_64::instructions::port::Port;
        Port::<u8>::new(0x20).write(0x20);
    }
}

// --- Exception handlers for all remaining vectors that can fire from Ring 3
// --- These use raw serial output only (no spinlocks) and kill the user process
// before returning to boot context, same pattern as the GPF handler.

/// Generic exception kill: print vector name, kill process, boot_return.
///
/// # Safety
/// Must only be called from exception handlers with valid stack frames.
unsafe fn exception_kill_user(name: &[u8], stack_frame: &InterruptStackFrame) {
    // SAFETY: port 0x3F8 is the COM1 data register on the x86_64 platforms
    // this kernel targets (QEMU q35/pc and PC hardware), which is the
    // raw_serial_* contract.
    unsafe {
        raw_serial_str(b"FATAL:");
        raw_serial_str(name);
        raw_serial_str(b" rip=0x");
        raw_serial_hex(stack_frame.instruction_pointer.as_u64());
        raw_serial_str(b" cs=0x");
        raw_serial_hex(stack_frame.code_segment.0 as u64);
        raw_serial_str(b" rsp=0x");
        raw_serial_hex(stack_frame.stack_pointer.as_u64());
        raw_serial_str(b"\n");
    }

    let cs = stack_frame.code_segment.0;
    if (cs & 3) == 3 {
        // User-mode fault: kill process and return to boot context.
        if let Some(process) = crate::process::current_process() {
            process.set_term_signal(11); // killed by SIGSEGV (N-99)
            process.set_state(crate::process::pcb::ProcessState::Zombie);
        }
        if crate::arch::x86_64::usermode::has_boot_return_context() {
            // SAFETY: forwarded from this function's contract: we are in an
            // exception handler (Ring 0, interrupts disabled) on a valid
            // stack frame. has_boot_return_context() just confirmed that
            // BOOT_RETURN_RSP/CR3 are valid, and gs_to_syscall_state() runs
            // immediately before boot_return_to_kernel(), as both require.
            unsafe {
                raw_serial_str(b"[EXC_KILL] boot_return\n");
                gs_to_syscall_state();
                crate::arch::x86_64::usermode::boot_return_to_kernel();
            }
        }
    }
}

extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    // SAFETY: called from this vector\'s own handler with the CPU-pushed
    // stack frame, which is exception_kill_user\'s contract.
    unsafe { exception_kill_user(b"#DE", &stack_frame) };
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    // SAFETY: called from this vector\'s own handler with the CPU-pushed
    // stack frame, which is exception_kill_user\'s contract.
    unsafe { exception_kill_user(b"#UD", &stack_frame) };
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn segment_not_present_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
    // SAFETY: called from this vector\'s own handler with the CPU-pushed
    // stack frame, which is exception_kill_user\'s contract.
    unsafe { exception_kill_user(b"#NP", &stack_frame) };
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn stack_segment_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
    // SAFETY: called from this vector\'s own handler with the CPU-pushed
    // stack frame, which is exception_kill_user\'s contract.
    unsafe { exception_kill_user(b"#SS", &stack_frame) };
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn alignment_check_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
    // SAFETY: called from this vector\'s own handler with the CPU-pushed
    // stack frame, which is exception_kill_user\'s contract.
    unsafe { exception_kill_user(b"#AC", &stack_frame) };
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn security_exception_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
    // SAFETY: called from this vector\'s own handler with the CPU-pushed
    // stack frame, which is exception_kill_user\'s contract.
    unsafe { exception_kill_user(b"#SX", &stack_frame) };
    loop {
        x86_64::instructions::hlt();
    }
}

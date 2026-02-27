//! Interrupt Descriptor Table
//!
//! Sets up handlers for CPU exceptions (breakpoint, page fault, GPF,
//! double fault) and hardware interrupts (timer). Fatal exception
//! handlers log diagnostic information and halt the CPU instead of
//! panicking, which avoids triggering a double fault from within an
//! interrupt context.

use lazy_static::lazy_static;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault.set_handler_fn(general_protection_fault_handler);
        // SAFETY: DOUBLE_FAULT_IST_INDEX is a valid IST index that was set up
        // during GDT initialization. Using a dedicated interrupt stack prevents
        // a triple fault when the kernel stack is corrupted.
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(crate::arch::x86_64::gdt::DOUBLE_FAULT_IST_INDEX);
        }
        // Add timer interrupt handler (IRQ0 = interrupt 32)
        idt[32].set_handler_fn(timer_interrupt_handler);
        // Add keyboard interrupt handler (IRQ1 = interrupt 33)
        idt[33].set_handler_fn(keyboard_interrupt_handler);
        // Add APIC timer interrupt handler (vector 48, separate from PIC timer at 32)
        idt[48].set_handler_fn(apic_timer_interrupt_handler);
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
        raw_serial_str(b"\n");
    }

    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    // SAFETY: Read CR2 (faulting address) before any code that might trigger
    // another page fault, which would overwrite CR2.
    let cr2_val: u64 = unsafe {
        let val: u64;
        core::arch::asm!("mov {}, cr2", out(reg) val, options(nomem, nostack));
        val
    };

    let ec = error_code.bits();
    let rip_val = stack_frame.instruction_pointer.as_u64();
    let was_user = ec & 4 != 0; // U/S bit

    // Early diagnostic: raw serial before ANY other work to catch cascading faults
    unsafe {
        raw_serial_str(b"PF! cr2=0x");
        raw_serial_hex(cr2_val);
        raw_serial_str(b" ec=0x");
        raw_serial_hex(ec);
        raw_serial_str(b" rip=0x");
        raw_serial_hex(rip_val);
        // Print current PID to identify which process faulted
        let pf_pid = crate::process::current_process()
            .map(|p| p.pid.0)
            .unwrap_or(0xDEAD);
        raw_serial_str(b" pid=0x");
        raw_serial_hex(pf_pid);
        raw_serial_str(b"\n");
    }

    // Attempt to resolve via demand paging framework.
    // Skip demand paging for NULL dereferences (addr < PAGE_SIZE) since no
    // valid mapping can exist there, and the demand paging code may GP fault
    // while iterating the VAS mappings from interrupt context.
    if cr2_val >= 0x1000 {
        let info = crate::mm::page_fault::from_x86_64(ec, cr2_val, rip_val);
        if let Ok(()) = crate::mm::page_fault::handle_page_fault(info) {
            // Fault resolved (demand page, CoW, or stack growth) — resume.
            return;
        }
    }

    // Unresolvable fault — print diagnostics via raw serial, then halt or
    // kill the process.
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
        unsafe {
            raw_serial_str(b"SEGFAULT pid=0x");
            raw_serial_hex(
                crate::process::current_process()
                    .map(|p| p.pid.0)
                    .unwrap_or(0xDEAD),
            );
            raw_serial_str(b" addr=0x");
            raw_serial_hex(cr2_val);
            raw_serial_str(b" rip=0x");
            raw_serial_hex(rip_val);
            raw_serial_str(b"\n");
        }

        // Dump user stack to identify the call chain at crash time.
        // Since we don't switch CR3, user pages are mapped.
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

        // Mark process as Zombie before returning to boot context.
        // Only use atomic state operations (set_exit_code, set_state).
        // Do NOT iterate threads BTreeMap or look up parent via
        // get_process() — those BTreeMap operations GP fault from
        // interrupt context on the TSS stack.
        if let Some(process) = crate::process::current_process() {
            process.set_exit_code(128 + 11); // SIGSEGV
            process.set_state(crate::process::pcb::ProcessState::Zombie);
        }

        // Return to boot context.
        // The page fault handler runs in interrupt context (no swapgs on
        // entry). boot_return_to_kernel expects the swapgs state from
        // syscall_entry. Do swapgs first to balance boot_return's swapgs.
        if crate::arch::x86_64::usermode::has_boot_return_context() {
            unsafe {
                raw_serial_str(b"[PF_KILL] boot_return\n");
                core::arch::asm!("swapgs", options(nomem, nostack));
                crate::arch::x86_64::usermode::boot_return_to_kernel();
            }
        }
        // No boot context — halt.
        loop {
            x86_64::instructions::hlt();
        }
    } else {
        // Kernel fault — unrecoverable. Print and halt.
        println!(
            "FATAL: kernel page fault at {:#x} ec={:#x} rip={:#x}",
            cr2_val, ec, rip_val
        );
        println!("{:#?}", stack_frame);
        loop {
            x86_64::instructions::hlt();
        }
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
        core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
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
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
        }
    }
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
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
    // Increment the global tick counter (atomic, always safe from interrupt
    // context).
    super::timer::tick();

    // Send APIC End-Of-Interrupt (NOT PIC EOI -- APIC timer uses its own EOI path).
    crate::arch::x86_64::apic::send_eoi();
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
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

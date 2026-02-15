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
    // Raw serial output first (bypasses spinlock to avoid deadlock in interrupt
    // context) SAFETY: Writing to COM1 data register at I/O port 0x3F8 is safe
    // for diagnostics.
    unsafe {
        for &b in b"FATAL:DF\n" {
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
        }
    }
    println!("FATAL: DOUBLE FAULT");
    println!("{:#?}", stack_frame);

    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    // Raw serial diagnostic (bypasses spinlock, safe in any context)
    // SAFETY: Writing to COM1 data register at I/O port 0x3F8 for diagnostics.
    unsafe {
        // Print "PF:" header
        for &b in b"PF@" {
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
        }
        // Print CR2 as hex (faulting address)
        let cr2_val: u64;
        core::arch::asm!("mov {}, cr2", out(reg) cr2_val, options(nomem, nostack));
        for shift in (0..16).rev() {
            let nibble = ((cr2_val >> (shift * 4)) & 0xF) as u8;
            let ch = if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            };
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") ch, options(nomem, nostack));
        }
        // Print error code bits
        let ec = error_code.bits();
        for &b in b" ec=" {
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
        }
        for shift in (0..4).rev() {
            let nibble = ((ec >> (shift * 4)) & 0xF) as u8;
            let ch = if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            };
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") ch, options(nomem, nostack));
        }
        // Print RIP from stack frame
        let rip_val = stack_frame.instruction_pointer.as_u64();
        for &b in b" rip=" {
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
        }
        for shift in (0..16).rev() {
            let nibble = ((rip_val >> (shift * 4)) & 0xF) as u8;
            let ch = if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            };
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") ch, options(nomem, nostack));
        }
        for &b in b"\n" {
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
        }
    }

    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    // Raw serial output first (bypasses spinlock)
    // SAFETY: Writing to COM1 data register at I/O port 0x3F8 for diagnostics.
    unsafe {
        for &b in b"FATAL:GP\n" {
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
        }
    }
    println!("FATAL: GENERAL PROTECTION FAULT");
    println!("Error Code: {:#x}", error_code);
    println!("{:#?}", stack_frame);

    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // SAFETY: Writing the EOI (End of Interrupt) byte (0x20) to the master
    // PIC command port (0x20) is required to acknowledge the timer interrupt.
    // Failing to send EOI would mask all further IRQs at this priority level.
    unsafe {
        use x86_64::instructions::port::Port;
        let mut pic_command: Port<u8> = Port::new(0x20);
        pic_command.write(0x20); // EOI command
    }
}

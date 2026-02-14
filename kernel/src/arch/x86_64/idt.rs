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
    // A double fault is unrecoverable. Log what we can and halt forever.
    // We intentionally avoid panic!() here because the panic handler itself
    // could trigger another exception on a corrupted stack, causing a
    // triple fault and immediate CPU reset with no diagnostic output.
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
    use x86_64::registers::control::Cr2;

    // Log full diagnostic information before halting. Using panic!() in an
    // interrupt handler risks a double fault if the panic machinery touches
    // the faulting page or overflows the interrupt stack.
    println!("FATAL: PAGE FAULT");
    println!("Accessed Address: {:?}", Cr2::read());
    println!("Error Code: {:?}", error_code);
    println!("{:#?}", stack_frame);

    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    // GPF is typically unrecoverable in kernel mode. Log and halt rather
    // than panic, which could cause a double fault in interrupt context.
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

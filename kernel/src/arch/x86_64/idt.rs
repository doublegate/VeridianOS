// Interrupt Descriptor Table

use lazy_static::lazy_static;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
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

#[allow(dead_code)]
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
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // Acknowledge the interrupt by sending End of Interrupt (EOI) to PIC
    unsafe {
        // Send EOI to the master PIC (0x20)
        use x86_64::instructions::port::Port;
        let mut pic_command: Port<u8> = Port::new(0x20);
        pic_command.write(0x20); // EOI command
    }
}

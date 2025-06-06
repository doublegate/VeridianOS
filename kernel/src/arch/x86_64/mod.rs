pub mod boot;
pub mod gdt;
pub mod idt;
pub mod serial;
pub mod vga;

pub fn init() {
    gdt::init();
    idt::init();
    unsafe { interrupts::enable() };
}

pub fn halt() -> ! {
    use x86_64::instructions::hlt;
    interrupts::disable();
    loop {
        hlt();
    }
}

pub fn enable_interrupts() {
    x86_64::instructions::interrupts::enable();
}

pub fn disable_interrupts() {
    x86_64::instructions::interrupts::disable();
}

pub fn idle() {
    x86_64::instructions::hlt();
}

mod interrupts {
    pub unsafe fn enable() {
        x86_64::instructions::interrupts::enable();
    }

    pub fn disable() {
        x86_64::instructions::interrupts::disable();
    }
}

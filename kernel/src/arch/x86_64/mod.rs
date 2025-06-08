pub mod boot;
pub mod gdt;
pub mod idt;
pub mod serial;
pub mod vga;

#[allow(dead_code)]
pub fn init() {
    gdt::init();
    idt::init();
    unsafe { interrupts::enable() };
}

#[allow(dead_code)]
pub fn halt() -> ! {
    use x86_64::instructions::hlt;
    interrupts::disable();
    loop {
        hlt();
    }
}

#[allow(dead_code)]
pub fn enable_interrupts() {
    x86_64::instructions::interrupts::enable();
}

#[allow(dead_code)]
pub fn disable_interrupts() {
    x86_64::instructions::interrupts::disable();
}

#[allow(dead_code)]
pub fn idle() {
    x86_64::instructions::hlt();
}

pub fn serial_init() -> uart_16550::SerialPort {
    let mut serial_port = unsafe { uart_16550::SerialPort::new(0x3F8) };
    serial_port.init();
    serial_port
}

mod interrupts {
    #[allow(dead_code)]
    pub unsafe fn enable() {
        x86_64::instructions::interrupts::enable();
    }

    #[allow(dead_code)]
    pub fn disable() {
        x86_64::instructions::interrupts::disable();
    }
}

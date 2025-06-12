pub mod boot;
pub mod context;
pub mod gdt;
pub mod idt;
pub mod mmu;
pub mod serial;
pub mod syscall;
pub mod timer;
pub mod vga;

#[allow(dead_code)]
pub fn init() {
    gdt::init();
    idt::init();
    mmu::init();
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

pub fn disable_interrupts() -> impl Drop {
    struct InterruptGuard {
        was_enabled: bool,
    }

    impl Drop for InterruptGuard {
        fn drop(&mut self) {
            if self.was_enabled {
                x86_64::instructions::interrupts::enable();
            }
        }
    }

    let was_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();
    InterruptGuard { was_enabled }
}

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

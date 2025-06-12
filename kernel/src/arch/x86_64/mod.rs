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
    println!("[ARCH] Starting GDT init...");
    gdt::init();
    println!("[ARCH] GDT initialized");

    println!("[ARCH] Starting IDT init...");
    idt::init();
    println!("[ARCH] IDT initialized");

    // Initialize PIC (8259) before enabling interrupts
    println!("[ARCH] Initializing PIC...");
    unsafe {
        use pic8259::ChainedPics;
        const PIC_1_OFFSET: u8 = 32;
        const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
        let mut pics = ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET);
        pics.initialize();
        // Disable all interrupts for now
        pics.write_masks(0xFF, 0xFF);
    }
    println!("[ARCH] PIC initialized with all interrupts masked");

    println!("[ARCH] Starting MMU init...");
    mmu::init();
    println!("[ARCH] MMU initialized");

    // Don't enable interrupts yet - they're all masked
    println!("[ARCH] Skipping interrupt enable for now");
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

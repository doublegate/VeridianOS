pub mod boot;
pub mod bootstrap;
pub mod context;
pub mod early_serial;
pub mod entry;
pub mod gdt;
pub mod idt;
pub mod mmu;
pub mod serial;
pub mod syscall;
pub mod timer;
pub mod vga;

#[allow(dead_code)]
pub fn init() {
    // Disable interrupts during initialization
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }
    
    println!("[ARCH] Starting GDT init...");
    gdt::init();
    println!("[ARCH] GDT initialized");

    println!("[ARCH] Starting IDT init...");
    idt::init();
    println!("[ARCH] IDT initialized");

    // Initialize PIC (8259) before enabling interrupts
    println!("[ARCH] Initializing PIC...");
    unsafe {
        use x86_64::instructions::port::Port;
        
        // Initialize PIC manually to ensure interrupts stay masked
        const PIC1_COMMAND: u16 = 0x20;
        const PIC1_DATA: u16 = 0x21;
        const PIC2_COMMAND: u16 = 0xA0;
        const PIC2_DATA: u16 = 0xA1;
        
        let mut pic1_cmd = Port::<u8>::new(PIC1_COMMAND);
        let mut pic1_data = Port::<u8>::new(PIC1_DATA);
        let mut pic2_cmd = Port::<u8>::new(PIC2_COMMAND);
        let mut pic2_data = Port::<u8>::new(PIC2_DATA);
        
        // Start initialization sequence
        pic1_cmd.write(0x11);
        pic2_cmd.write(0x11);
        
        // Set vector offsets
        pic1_data.write(32);
        pic2_data.write(40);
        
        // Set cascading
        pic1_data.write(4);
        pic2_data.write(2);
        
        // Set 8086 mode
        pic1_data.write(0x01);
        pic2_data.write(0x01);
        
        // Mask all interrupts
        pic1_data.write(0xFF);
        pic2_data.write(0xFF);
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

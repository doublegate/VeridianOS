//! x86_64 architecture support.
//!
//! Provides hardware initialization (GDT, IDT, PIC), interrupt control,
//! serial I/O (COM1 at 0x3F8), VGA text output, and I/O port primitives
//! for the x86_64 platform.

#![allow(clippy::missing_safety_doc)]

pub mod boot;
pub mod bootstrap;
pub mod context;
pub mod early_serial;
pub mod entry;
pub mod gdt;
pub mod idt;
pub mod mmu;
pub mod multiboot;
pub mod serial;
pub mod syscall;
pub mod timer;
pub mod vga;

/// Called from bootstrap on x86_64; appears unused on other architectures.
#[allow(dead_code)]
pub fn init() {
    // SAFETY: The cli instruction disables hardware interrupts. This is required
    // during initialization to prevent interrupt handlers from firing before the
    // IDT and PIC are properly configured. nomem/nostack confirm no memory access.
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
    // SAFETY: I/O port writes to the 8259 PIC (ports 0x20/0x21 for PIC1,
    // 0xA0/0xA1 for PIC2) are required to initialize the interrupt controller.
    // The initialization sequence (ICW1-ICW4) is well-defined by the 8259 spec.
    // All interrupts are masked (0xFF) at the end to prevent spurious IRQs.
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

/// Halt the CPU. Used by panic/shutdown paths.
#[allow(dead_code)]
pub fn halt() -> ! {
    use x86_64::instructions::hlt;
    interrupts::disable();
    loop {
        hlt();
    }
}

/// Enable hardware interrupts. Will be used once interrupt handlers are fully
/// configured.
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

/// Speculation barrier to mitigate Spectre-style attacks.
/// Uses LFENCE which serializes instruction execution on Intel/AMD.
#[inline(always)]
pub fn speculation_barrier() {
    // SAFETY: lfence is a serializing instruction that prevents speculative
    // execution of subsequent instructions until all prior instructions
    // complete. No side effects beyond pipeline serialization.
    unsafe {
        core::arch::asm!("lfence", options(nostack, nomem, preserves_flags));
    }
}

pub fn serial_init() -> uart_16550::SerialPort {
    // SAFETY: SerialPort::new(0x3F8) creates a serial port handle for COM1
    // at the standard I/O base address. The address is well-known and the
    // port is initialized immediately after construction.
    let mut serial_port = unsafe { uart_16550::SerialPort::new(0x3F8) };
    serial_port.init();
    serial_port
}

/// Basic I/O port functions -- used by PCI, console, and storage drivers.
#[allow(dead_code)]
pub unsafe fn outb(port: u16, value: u8) {
    x86_64::instructions::port::Port::new(port).write(value);
}

#[allow(dead_code)]
pub unsafe fn inb(port: u16) -> u8 {
    x86_64::instructions::port::Port::new(port).read()
}

#[allow(dead_code)]
pub unsafe fn outw(port: u16, value: u16) {
    x86_64::instructions::port::Port::new(port).write(value);
}

#[allow(dead_code)]
pub unsafe fn inw(port: u16) -> u16 {
    x86_64::instructions::port::Port::new(port).read()
}

#[allow(dead_code)]
pub unsafe fn outl(port: u16, value: u32) {
    x86_64::instructions::port::Port::new(port).write(value);
}

#[allow(dead_code)]
pub unsafe fn inl(port: u16) -> u32 {
    x86_64::instructions::port::Port::new(port).read()
}

/// Kernel heap start address (mapped by bootloader 0.9)
pub const HEAP_START: usize = 0x444444440000;

/// Flush TLB for a specific virtual address.
#[allow(dead_code)]
pub fn tlb_flush_address(addr: u64) {
    // SAFETY: `invlpg` invalidates the TLB entry for the page containing the
    // given virtual address. Privileged, no side effects beyond TLB.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) addr);
    }
}

/// Flush entire TLB.
#[allow(dead_code)]
pub fn tlb_flush_all() {
    // SAFETY: Reloading CR3 with its current value flushes all non-global TLB
    // entries. Privileged, no memory side effects.
    unsafe {
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3);
        core::arch::asm!("mov cr3, {}", in(reg) cr3);
    }
}

mod interrupts {
    /// Enable interrupts. Will be called once interrupt handlers are
    /// registered.
    #[allow(dead_code)]
    pub unsafe fn enable() {
        x86_64::instructions::interrupts::enable();
    }

    #[allow(dead_code)]
    pub fn disable() {
        x86_64::instructions::interrupts::disable();
    }
}

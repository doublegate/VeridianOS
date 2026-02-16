//! x86_64 architecture support.
//!
//! Provides hardware initialization (GDT, IDT, PIC, APIC), interrupt control,
//! serial I/O (COM1 at 0x3F8), VGA text output, and I/O port primitives
//! for the x86_64 platform.

pub mod apic;
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
pub mod usermode;
pub mod vga;

/// Called from bootstrap on x86_64 via `crate::arch::init()`.
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

    // Initialize SYSCALL/SYSRET support (must be after GDT init so that
    // STAR MSR references valid user-mode selectors in the loaded GDT)
    println!("[ARCH] Initializing SYSCALL/SYSRET...");
    syscall::init_syscall();
    println!("[ARCH] SYSCALL/SYSRET initialized");

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

    // Initialize Local APIC + I/O APIC (additive to PIC -- PIC remains as
    // fallback). APIC init is non-fatal: if it fails the kernel continues
    // with PIC-only interrupt routing.
    println!("[ARCH] Initializing APIC...");
    match apic::init() {
        Ok(()) => println!("[ARCH] APIC initialized"),
        Err(e) => println!("[ARCH] APIC init skipped: {}", e),
    }

    // Don't enable interrupts yet - they're all masked
    println!("[ARCH] Skipping interrupt enable for now");
}

/// Halt the CPU. Used by panic/shutdown paths via `crate::arch::halt()`.
pub fn halt() -> ! {
    use x86_64::instructions::hlt;
    interrupts::disable();
    loop {
        hlt();
    }
}

/// Enable hardware interrupts.
pub fn enable_interrupts() {
    x86_64::instructions::interrupts::enable();
}

/// Unmask the keyboard IRQ (IRQ1) on PIC1.
///
/// Reads the current PIC1 data mask, clears bit 1, and writes it back.
/// This allows the keyboard interrupt (vector 33) to fire.
pub fn enable_keyboard_irq() {
    // SAFETY: Reading and writing the PIC1 data port (0x21) to unmask
    // IRQ1 (keyboard). This is a standard PIC operation.
    unsafe {
        use x86_64::instructions::port::Port;
        let mut pic1_data = Port::<u8>::new(0x21);
        let mask = pic1_data.read();
        pic1_data.write(mask & !0x02); // Clear bit 1 (IRQ1 = keyboard)
    }
}

/// Unmask the timer IRQ (IRQ0) on PIC1.
pub fn enable_timer_irq() {
    // SAFETY: Reading and writing PIC1 data port to unmask IRQ0.
    unsafe {
        use x86_64::instructions::port::Port;
        let mut pic1_data = Port::<u8>::new(0x21);
        let mask = pic1_data.read();
        pic1_data.write(mask & !0x01); // Clear bit 0 (IRQ0 = timer)
    }
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

/// Write a byte to an x86_64 I/O port.
///
/// # Safety
/// The caller must ensure `port` is a valid I/O port address for the
/// intended device. Writing to an incorrect port can cause undefined
/// hardware behavior.
pub unsafe fn outb(port: u16, value: u8) {
    x86_64::instructions::port::Port::new(port).write(value);
}

/// Read a byte from an x86_64 I/O port.
///
/// # Safety
/// The caller must ensure `port` is a valid I/O port address for the
/// intended device. Reading from an incorrect port may return garbage
/// or trigger hardware side effects.
pub unsafe fn inb(port: u16) -> u8 {
    x86_64::instructions::port::Port::new(port).read()
}

/// Write a 16-bit word to an x86_64 I/O port.
///
/// # Safety
/// The caller must ensure `port` is a valid I/O port address for the
/// intended device and that the device expects a 16-bit write.
pub unsafe fn outw(port: u16, value: u16) {
    x86_64::instructions::port::Port::new(port).write(value);
}

/// Read a 16-bit word from an x86_64 I/O port.
///
/// # Safety
/// The caller must ensure `port` is a valid I/O port address for the
/// intended device and that the device produces valid 16-bit reads.
pub unsafe fn inw(port: u16) -> u16 {
    x86_64::instructions::port::Port::new(port).read()
}

/// Write a 32-bit dword to an x86_64 I/O port.
///
/// # Safety
/// The caller must ensure `port` is a valid I/O port address for the
/// intended device and that the device expects a 32-bit write.
pub unsafe fn outl(port: u16, value: u32) {
    x86_64::instructions::port::Port::new(port).write(value);
}

/// Read a 32-bit dword from an x86_64 I/O port.
///
/// # Safety
/// The caller must ensure `port` is a valid I/O port address for the
/// intended device and that the device produces valid 32-bit reads.
pub unsafe fn inl(port: u16) -> u32 {
    x86_64::instructions::port::Port::new(port).read()
}

/// Kernel heap start address (mapped by bootloader 0.9)
pub const HEAP_START: usize = 0x444444440000;

/// Flush TLB for a specific virtual address. Called via
/// `crate::arch::tlb_flush_address()`.
pub fn tlb_flush_address(addr: u64) {
    // SAFETY: `invlpg` invalidates the TLB entry for the page containing the
    // given virtual address. Privileged, no side effects beyond TLB.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) addr);
    }
}

/// Flush entire TLB. Called via `crate::arch::tlb_flush_all()`.
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
    /// Disable hardware interrupts. Called from `halt()`.
    pub fn disable() {
        x86_64::instructions::interrupts::disable();
    }
}

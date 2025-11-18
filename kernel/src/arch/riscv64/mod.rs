// RISC-V 64 architecture support

pub mod boot;
pub mod bootstrap;
pub mod entry;
pub mod serial;

// Re-export context and timer from parent riscv module
#[allow(unused_imports)]
pub use super::riscv::{context, timer};

#[allow(dead_code)]
pub fn init() {
    // Initialize SBI (Supervisor Binary Interface)
    super::riscv::sbi::init();

    // Enable supervisor-mode external, software, and timer interrupts
    unsafe {
        // Enable interrupts in sstatus
        enable_interrupts();

        // Enable specific interrupt sources in sie
        // SEIE (bit 9), STIE (bit 5), SSIE (bit 1)
        core::arch::asm!("csrs sie, {}", in(reg) (1 << 9) | (1 << 5) | (1 << 1));
    }

    println!("[RISCV64] Architecture initialization complete");
}

#[allow(dead_code)]
pub fn halt() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

#[allow(dead_code)]
pub fn enable_interrupts() {
    unsafe {
        core::arch::asm!("csrsi sstatus, 2");
    }
}

#[allow(dead_code)]
pub fn disable_interrupts() -> impl Drop {
    struct InterruptGuard {
        was_enabled: bool,
    }

    impl Drop for InterruptGuard {
        fn drop(&mut self) {
            if self.was_enabled {
                unsafe {
                    core::arch::asm!("csrsi sstatus, 2");
                }
            }
        }
    }

    let mut sstatus: usize;
    unsafe {
        core::arch::asm!("csrr {}, sstatus", out(reg) sstatus);
        core::arch::asm!("csrci sstatus, 2");
    }
    InterruptGuard {
        was_enabled: (sstatus & 0x2) != 0,
    }
}

#[allow(dead_code)]
pub fn idle() {
    unsafe { core::arch::asm!("wfi") };
}

pub fn serial_init() -> crate::serial::Uart16550Compat {
    // QEMU virt machine places 16550 UART at 0x10000000
    let mut uart = crate::serial::Uart16550Compat::new(0x1000_0000);
    uart.init();
    uart
}

/// I/O port functions (stubs for RISC-V - no I/O ports like x86)
#[allow(dead_code)]
pub unsafe fn outb(_port: u16, _value: u8) {
    // No-op: RISC-V doesn't have I/O ports
}

#[allow(dead_code)]
pub unsafe fn inb(_port: u16) -> u8 {
    // No-op: RISC-V doesn't have I/O ports
    0
}

#[allow(dead_code)]
pub unsafe fn outw(_port: u16, _value: u16) {
    // No-op: RISC-V doesn't have I/O ports
}

#[allow(dead_code)]
pub unsafe fn inw(_port: u16) -> u16 {
    // No-op: RISC-V doesn't have I/O ports
    0
}

#[allow(dead_code)]
pub unsafe fn outl(_port: u16, _value: u32) {
    // No-op: RISC-V doesn't have I/O ports
}

#[allow(dead_code)]
pub unsafe fn inl(_port: u16) -> u32 {
    // No-op: RISC-V doesn't have I/O ports
    0
}

// RISC-V 64 architecture support

pub mod boot;

// Re-export context and timer from parent riscv module
#[allow(unused_imports)]
pub use super::riscv::{context, timer};

#[allow(dead_code)]
pub fn init() {
    // TODO: Initialize RISC-V 64-specific features
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

// RISC-V 64 architecture support (stub)

pub mod boot;

#[allow(dead_code)]
pub fn init() {
    // TODO: Initialize RISC-V 64-specific features
}

#[allow(dead_code)]
pub fn halt() -> ! {
    loop {
        // TODO: Implement WFI (Wait For Interrupt)
        unsafe { core::arch::asm!("wfi") };
    }
}

#[allow(dead_code)]
pub fn enable_interrupts() {
    // TODO: Enable interrupts on RISC-V
}

#[allow(dead_code)]
pub fn disable_interrupts() {
    // TODO: Disable interrupts on RISC-V
}

#[allow(dead_code)]
pub fn idle() {
    // TODO: Implement idle for RISC-V
    unsafe { core::arch::asm!("wfi") };
}

pub fn serial_init() -> crate::serial::Uart16550Compat {
    // QEMU virt machine places 16550 UART at 0x10000000
    let mut uart = crate::serial::Uart16550Compat::new(0x1000_0000);
    uart.init();
    uart
}

// RISC-V 64 architecture support (stub)

pub fn init() {
    // TODO: Initialize RISC-V 64-specific features
}

pub fn halt() -> ! {
    loop {
        // TODO: Implement WFI (Wait For Interrupt)
        unsafe { core::arch::asm!("wfi") };
    }
}

pub fn enable_interrupts() {
    // TODO: Enable interrupts on RISC-V
}

pub fn disable_interrupts() {
    // TODO: Disable interrupts on RISC-V
}

pub fn idle() {
    // TODO: Implement idle for RISC-V
    unsafe { core::arch::asm!("wfi") };
}

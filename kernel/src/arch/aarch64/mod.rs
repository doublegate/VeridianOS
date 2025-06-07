// AArch64 architecture support (stub)

pub fn init() {
    // TODO: Initialize AArch64-specific features
}

pub fn halt() -> ! {
    loop {
        // TODO: Implement WFI (Wait For Interrupt)
        unsafe { core::arch::asm!("wfi") };
    }
}

#[allow(dead_code)]
pub fn enable_interrupts() {
    // TODO: Enable interrupts on AArch64
}

#[allow(dead_code)]
pub fn disable_interrupts() {
    // TODO: Disable interrupts on AArch64
}

pub fn idle() {
    // TODO: Implement idle for AArch64
    unsafe { core::arch::asm!("wfi") };
}

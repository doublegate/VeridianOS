// AArch64 architecture support

// Include the boot module
pub mod boot;

#[allow(dead_code)]
pub fn init() {
    // Architecture-specific initialization
    // This will be expanded later
}

pub fn halt() -> ! {
    loop {
        unsafe {
            core::arch::asm!("wfe");
        }
    }
}

#[allow(dead_code)]
pub fn idle() {
    unsafe {
        core::arch::asm!("wfe");
    }
}

// Simple serial initialization for compatibility
pub fn serial_init() -> crate::serial::Pl011Uart {
    crate::serial::Pl011Uart::new(0x0900_0000)
}
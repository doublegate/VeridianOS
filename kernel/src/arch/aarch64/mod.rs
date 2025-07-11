// AArch64 architecture support

// Include the boot module
pub mod boot;
pub mod context;
pub mod direct_uart;
pub mod manual_print;
pub mod safe_iter;
pub mod timer;

#[allow(dead_code)]
pub fn init() {
    // Architecture-specific initialization
    unsafe {
        let uart = 0x0900_0000 as *mut u8;
        core::ptr::write_volatile(uart, b'A');
        core::ptr::write_volatile(uart, b'R');
        core::ptr::write_volatile(uart, b'C');
        core::ptr::write_volatile(uart, b'H');
        core::ptr::write_volatile(uart, b'\n');
    }
    // This will be expanded later
}

#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn disable_interrupts() -> impl Drop {
    struct InterruptGuard {
        was_enabled: bool,
    }

    impl Drop for InterruptGuard {
        fn drop(&mut self) {
            if self.was_enabled {
                unsafe {
                    core::arch::asm!("msr daifclr, #2");
                }
            }
        }
    }

    let mut daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif);
        core::arch::asm!("msr daifset, #2");
    }
    InterruptGuard {
        was_enabled: (daif & 0x80) == 0,
    }
}

// Simple serial initialization for compatibility
#[allow(dead_code)]
pub fn serial_init() -> crate::serial::Pl011Uart {
    crate::serial::Pl011Uart::new(0x0900_0000)
}

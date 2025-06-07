#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "x86_64")]
pub use x86_64::*;

#[cfg(target_arch = "aarch64")]
pub mod aarch64;

#[cfg(target_arch = "aarch64")]
pub use aarch64::*;

#[cfg(target_arch = "riscv64")]
pub mod riscv64;

#[cfg(target_arch = "riscv64")]
pub use riscv64::*;

// Wrapper for serial_init to return common type
pub fn serial_init() -> crate::serial::SerialPort {
    #[cfg(target_arch = "x86_64")]
    {
        crate::serial::SerialPort::from_inner(x86_64::serial_init())
    }
    #[cfg(target_arch = "aarch64")]
    {
        crate::serial::SerialPort::from_inner(aarch64::serial_init())
    }
    #[cfg(target_arch = "riscv64")]
    {
        crate::serial::SerialPort::from_inner(riscv64::serial_init())
    }
}

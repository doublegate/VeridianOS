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

pub trait Architecture {
    fn init();
    fn halt() -> !;
    fn enable_interrupts();
    fn disable_interrupts();
    fn idle();
}
#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;

#[macro_use]
mod print;

mod arch;
mod cap;
mod ipc;
mod mm;
mod sched;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    #[cfg(target_arch = "x86_64")]
    println!("[KERNEL PANIC] {}", _info);
    
    arch::halt();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[KERNEL PANIC] {}", info);
    exit_qemu(QemuExitCode::Failed);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    println!("VeridianOS v{}", env!("CARGO_PKG_VERSION"));
    println!("Initializing microkernel...");

    // Initialize architecture-specific features
    arch::init();

    // Initialize memory management
    mm::init();

    // Initialize capability system
    cap::init();

    // Initialize scheduler
    sched::init();

    // Initialize IPC
    ipc::init();

    #[cfg(test)]
    test_main();

    println!("VeridianOS initialized successfully!");

    // Enter scheduler main loop
    sched::run();
}

#[cfg(test)]
fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

#[cfg(test)]
trait Testable {
    fn run(&self) -> ();
}

#[cfg(test)]
impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

#[cfg(test)]
pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
    unreachable!();
}

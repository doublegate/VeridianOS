// Boot entry point for x86_64

use bootloader::{entry_point, BootInfo};

entry_point!(kernel_main_entry);

fn kernel_main_entry(_boot_info: &'static BootInfo) -> ! {
    // Call the real kernel_main from main.rs
    extern "C" {
        fn kernel_main() -> !;
    }
    unsafe { kernel_main() }
}

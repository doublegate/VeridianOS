// Boot entry point for x86_64

use bootloader::{BootInfo, entry_point};

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    // Jump to the main kernel entry
    crate::_start()
}
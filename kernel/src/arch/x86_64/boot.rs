// Boot entry point for x86_64

use bootloader::{entry_point, BootInfo};

entry_point!(kernel_main);

fn kernel_main(_boot_info: &'static BootInfo) -> ! {
    // Jump to the main kernel entry
    crate::kernel_main()
}

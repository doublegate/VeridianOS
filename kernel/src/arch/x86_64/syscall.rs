//! x86_64 system call entry point

// TODO: Import syscall handler when syscall module is enabled
// use crate::syscall::syscall_handler;

/// x86_64 SYSCALL instruction entry point
///
/// This is a placeholder implementation. The actual implementation
/// requires naked function support which is currently unstable.
#[no_mangle]
pub unsafe extern "C" fn syscall_entry() {
    // Placeholder implementation
    // In a real implementation, this would:
    // 1. Save user context
    // 2. Switch to kernel stack
    // 3. Call syscall_handler
    // 4. Restore user context
    // 5. Return to user mode

    // For now, just call the handler directly
    // let _result = syscall_handler(0, 0, 0, 0, 0, 0);
}

/// Initialize SYSCALL/SYSRET support
#[allow(dead_code)]
pub fn init_syscall() {
    use x86_64::registers::{
        model_specific::{Efer, EferFlags, LStar, Star},
        segmentation::SegmentSelector,
    };

    unsafe {
        // Enable SYSCALL/SYSRET
        Efer::update(|flags| {
            flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });

        // Set up SYSCALL entry point
        LStar::write(x86_64::VirtAddr::new(syscall_entry as usize as u64));

        // Set up segment selectors
        // Star::write takes 4 arguments:
        // 1. User CS (for SYSRET)
        // 2. User SS (for SYSRET)
        // 3. Kernel CS (for SYSCALL)
        // 4. Kernel SS (for SYSCALL)
        Star::write(
            SegmentSelector(0x18), // User CS (ring 3)
            SegmentSelector(0x20), // User SS (ring 3)
            SegmentSelector(0x08), // Kernel CS (ring 0)
            SegmentSelector(0x10), // Kernel SS (ring 0)
        )
        .unwrap();
    }
}

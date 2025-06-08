//! x86_64 system call entry point

use crate::syscall::syscall_handler;

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
    let _result = syscall_handler(0, 0, 0, 0, 0, 0);
}

/// Initialize SYSCALL/SYSRET support
pub fn init_syscall() {
    use x86_64::registers::model_specific::{Efer, EferFlags, LStar, Star};
    
    unsafe {
        // Enable SYSCALL/SYSRET
        Efer::update(|flags| {
            flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });
        
        // Set up SYSCALL entry point
        LStar::write(x86_64::VirtAddr::new(syscall_entry as u64));
        
        // Set up segment selectors
        // STAR[63:48] = kernel CS
        // STAR[47:32] = kernel SS
        // Note: Star::write takes two u8 arguments
        Star::write(0x00, 0x08).unwrap();
    }
}
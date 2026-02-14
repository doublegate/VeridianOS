//! Global shell state management.
//!
//! Manages the singleton Shell instance using a pointer-based pattern that
//! works safely across all architectures (x86_64, AArch64, RISC-V).
//! Architecture-specific memory barriers ensure proper visibility of the
//! pointer write on weakly-ordered platforms.

use super::Shell;

/// Global shell instance using pointer pattern for all architectures.
/// This avoids static mut Option issues and provides consistent behavior.
static mut SHELL_PTR: *mut Shell = core::ptr::null_mut();

/// Initialize the shell.
///
/// Creates the global Shell singleton. Subsequent calls are no-ops with a
/// warning printed to the console.
pub fn init() {
    #[allow(unused_imports)]
    use crate::println;

    // SAFETY: This function is called once during single-threaded kernel init.
    // The volatile read/write and architecture-specific memory barriers ensure
    // the pointer is visible to all subsequent readers. The Box::leak ensures
    // the Shell lives for the lifetime of the kernel.
    unsafe {
        let current = core::ptr::read_volatile(&raw const SHELL_PTR);
        if !current.is_null() {
            println!("[SHELL] WARNING: Already initialized! Skipping re-initialization.");
            return;
        }

        let shell = Shell::new();
        let shell_box = alloc::boxed::Box::new(shell);
        let shell_ptr = alloc::boxed::Box::leak(shell_box) as *mut Shell;

        // Store the pointer using volatile write to prevent compiler optimization
        core::ptr::write_volatile(&raw mut SHELL_PTR, shell_ptr);

        // SAFETY: Memory barriers after assignment ensure the pointer write is
        // visible to all CPUs on weakly-ordered architectures.
        #[cfg(target_arch = "aarch64")]
        {
            core::arch::asm!("dsb sy", "isb", options(nostack, nomem, preserves_flags));
        }

        #[cfg(target_arch = "riscv64")]
        {
            core::arch::asm!("fence rw, rw", options(nostack, nomem, preserves_flags));
        }

        // Verify the write took effect
        let _verify = core::ptr::read_volatile(&raw const SHELL_PTR);
        #[cfg(target_arch = "aarch64")]
        {
            crate::arch::aarch64::direct_uart::direct_print_str("[SHELL] Shell module loaded\n");
        }
        #[cfg(not(target_arch = "aarch64"))]
        if _verify.is_null() {
            println!("[SHELL] ERROR: SHELL_PTR write failed!");
        } else {
            println!("[SHELL] Shell module loaded (ptr={:p})", _verify);
        }
    }
}

/// Get the global shell.
///
/// # Panics
///
/// Panics if the shell has not been initialized via [`init`].
/// Prefer [`try_get_shell`] in contexts where a panic is unacceptable.
pub fn get_shell() -> &'static Shell {
    // SAFETY: The pointer was set during init() with proper memory barriers.
    // read_volatile prevents the compiler from caching the initial null value
    // across calls. The pointer, once set, is never modified or freed.
    unsafe {
        let ptr = core::ptr::read_volatile(&raw const SHELL_PTR);
        if ptr.is_null() {
            panic!("Shell not initialized");
        }
        &*ptr
    }
}

/// Try to get the global shell (non-panicking).
///
/// Returns `None` if the shell has not been initialized via [`init`].
pub fn try_get_shell() -> Option<&'static Shell> {
    // SAFETY: Same as get_shell - read_volatile ensures we see the latest
    // pointer value. The returned reference is valid for the kernel lifetime.
    unsafe {
        let ptr = core::ptr::read_volatile(&raw const SHELL_PTR);
        if ptr.is_null() {
            None
        } else {
            Some(&*ptr)
        }
    }
}

/// Run shell as a process.
///
/// This function never returns.
pub fn run_shell() -> ! {
    let shell = get_shell();
    shell.run()
}

//! Global shell state management.
//!
//! Manages the singleton Shell instance using OnceLock for safe
//! initialization across all architectures (x86_64, AArch64, RISC-V).

use super::Shell;

/// Global shell instance using OnceLock for safe initialization.
static SHELL: crate::sync::once_lock::OnceLock<Shell> = crate::sync::once_lock::OnceLock::new();

/// Initialize the shell.
///
/// Creates the global Shell singleton. Subsequent calls are no-ops with a
/// warning printed to the console.
pub fn init() {
    #[allow(unused_imports)]
    use crate::println;

    match SHELL.set(Shell::new()) {
        Ok(()) => {
            #[cfg(target_arch = "aarch64")]
            {
                crate::arch::aarch64::direct_uart::direct_print_str(
                    "[SHELL] Shell module loaded\n",
                );
            }
            #[cfg(not(target_arch = "aarch64"))]
            println!("[SHELL] Shell module loaded");
        }
        Err(_) => {
            println!("[SHELL] WARNING: Already initialized! Skipping re-initialization.");
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
    SHELL.get().expect("Shell not initialized")
}

/// Try to get the global shell (non-panicking).
///
/// Returns `None` if the shell has not been initialized via [`init`].
pub fn try_get_shell() -> Option<&'static Shell> {
    SHELL.get()
}

/// Run shell as a process.
///
/// This function never returns.
pub fn run_shell() -> ! {
    let shell = get_shell();
    shell.run()
}

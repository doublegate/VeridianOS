//! RISC-V SBI (Supervisor Binary Interface) calls
//!
//! Provides wrappers for SBI calls to interact with machine mode firmware.

/// SBI extension IDs
const SBI_EXT_BASE: usize = 0x10;
const SBI_EXT_TIMER: usize = 0x54494D45; // "TIME"
const SBI_EXT_IPI: usize = 0x735049; // "sPI"
const SBI_EXT_RFENCE: usize = 0x52464E43; // "RFNC"
const SBI_EXT_HSM: usize = 0x48534D; // "HSM"
const SBI_EXT_SRST: usize = 0x53525354; // "SRST"

/// SBI function IDs for timer extension
const SBI_TIMER_SET_TIMER: usize = 0;

/// SBI return value
#[derive(Debug, Clone, Copy)]
pub struct SbiRet {
    pub error: isize,
    pub value: usize,
}

impl SbiRet {
    pub fn is_ok(&self) -> bool {
        self.error == 0
    }
}

/// Make an SBI call
#[inline(always)]
fn sbi_call(extension: usize, function: usize, arg0: usize, arg1: usize, arg2: usize) -> SbiRet {
    let error: isize;
    let value: usize;

    unsafe {
        core::arch::asm!(
            "ecall",
            in("a0") arg0,
            in("a1") arg1,
            in("a2") arg2,
            in("a6") function,
            in("a7") extension,
            lateout("a0") error,
            lateout("a1") value,
        );
    }

    SbiRet { error, value }
}

/// Set timer for next interrupt
///
/// Sets the timer to trigger an interrupt at the specified time value.
/// Time is measured in implementation-defined units (typically CPU cycles).
pub fn set_timer(stime_value: u64) -> SbiRet {
    // For 64-bit RISC-V, the time value fits in a single register
    sbi_call(
        SBI_EXT_TIMER,
        SBI_TIMER_SET_TIMER,
        stime_value as usize,
        0,
        0,
    )
}

/// Get SBI implementation ID
pub fn get_sbi_impl_id() -> SbiRet {
    sbi_call(SBI_EXT_BASE, 1, 0, 0, 0)
}

/// Get SBI implementation version
pub fn get_sbi_impl_version() -> SbiRet {
    sbi_call(SBI_EXT_BASE, 2, 0, 0, 0)
}

/// Check if an SBI extension is available
pub fn probe_extension(extension_id: usize) -> bool {
    let ret = sbi_call(SBI_EXT_BASE, 3, extension_id, 0, 0);
    ret.value != 0
}

/// Legacy console putchar (for early boot)
pub fn console_putchar(ch: u8) {
    sbi_call(0x01, 0, ch as usize, 0, 0);
}

/// Legacy console getchar (for early boot)
pub fn console_getchar() -> isize {
    sbi_call(0x02, 0, 0, 0, 0).error
}

/// Initialize SBI and print version info
pub fn init() {
    use crate::println;

    let impl_id = get_sbi_impl_id();
    let impl_version = get_sbi_impl_version();

    println!("[SBI] SBI implementation ID: {}", impl_id.value);
    println!(
        "[SBI] SBI implementation version: 0x{:x}",
        impl_version.value
    );

    // Check for required extensions
    let timer_available = probe_extension(SBI_EXT_TIMER);
    println!("[SBI] Timer extension available: {}", timer_available);

    let ipi_available = probe_extension(SBI_EXT_IPI);
    println!("[SBI] IPI extension available: {}", ipi_available);

    let rfence_available = probe_extension(SBI_EXT_RFENCE);
    println!(
        "[SBI] Remote fence extension available: {}",
        rfence_available
    );

    if !timer_available {
        println!("[SBI] WARNING: Timer extension not available!");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_sbi_probe() {
        // Timer extension should be available on most platforms
        let has_timer = probe_extension(SBI_EXT_TIMER);
        // Just verify the call doesn't crash
        let _ = has_timer;
    }
}

//! PSCI (Arm Power State Coordination Interface, DEN0022) calls.
//!
//! Only what starting secondary CPUs needs. The conduit (HVC or SMC) comes
//! from the device tree's `/psci` `method`; QEMU virt uses HVC unless
//! `virtualization=on` (then SMC). Calls follow SMCCC (DEN0028): function id
//! in w0, arguments in x1-x3, result in x0; x4-x17 are treated as
//! clobbered.

/// How PSCI calls reach the firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conduit {
    Hvc,
    Smc,
}

/// `CPU_ON`, SMC64 calling convention.
const PSCI_0_2_FN64_CPU_ON: u64 = 0xC400_0003;

/// The conduit named by a `/psci` `method` property value.
pub fn conduit_from_method(method: &[u8]) -> Option<Conduit> {
    match method.split(|&b| b == 0).next()? {
        b"hvc" => Some(Conduit::Hvc),
        b"smc" => Some(Conduit::Smc),
        _ => None,
    }
}

/// Human-readable PSCI return code.
pub fn error_name(code: i64) -> &'static str {
    match code {
        0 => "SUCCESS",
        -1 => "NOT_SUPPORTED",
        -2 => "INVALID_PARAMETERS",
        -3 => "DENIED",
        -4 => "ALREADY_ON",
        -5 => "ON_PENDING",
        -6 => "INTERNAL_FAILURE",
        -7 => "NOT_PRESENT",
        -8 => "DISABLED",
        -9 => "INVALID_ADDRESS",
        _ => "unknown PSCI error",
    }
}

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
fn call(conduit: Conduit, fid: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: u64;
    match conduit {
        // SAFETY: an SMCCC call into the PSCI firmware named by the device
        // tree. Arguments are in x0-x3; x4-x17 may be clobbered (SMCCC 1.0).
        Conduit::Hvc => unsafe {
            core::arch::asm!("hvc #0", inout("x0") fid => ret, inout("x1") a1 => _,
                inout("x2") a2 => _, inout("x3") a3 => _, out("x4") _, out("x5") _,
                out("x6") _, out("x7") _, out("x8") _, out("x9") _, out("x10") _,
                out("x11") _, out("x12") _, out("x13") _, out("x14") _, out("x15") _,
                out("x16") _, out("x17") _, options(nostack))
        },
        // SAFETY: as above, through the secure monitor.
        Conduit::Smc => unsafe {
            core::arch::asm!("smc #0", inout("x0") fid => ret, inout("x1") a1 => _,
                inout("x2") a2 => _, inout("x3") a3 => _, out("x4") _, out("x5") _,
                out("x6") _, out("x7") _, out("x8") _, out("x9") _, out("x10") _,
                out("x11") _, out("x12") _, out("x13") _, out("x14") _, out("x15") _,
                out("x16") _, out("x17") _, options(nostack))
        },
    }
    ret as i64
}

/// Power on the CPU with affinity `mpidr` at physical address `entry`, with
/// `x0 = context`. Asynchronous: success means the CPU will start.
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub fn cpu_on(conduit: Conduit, mpidr: u64, entry: u64, context: u64) -> i64 {
    call(conduit, PSCI_0_2_FN64_CPU_ON, mpidr, entry, context)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conduit_and_errors() {
        assert_eq!(conduit_from_method(b"hvc\0"), Some(Conduit::Hvc));
        assert_eq!(conduit_from_method(b"smc"), Some(Conduit::Smc));
        assert_eq!(conduit_from_method(b"spin-table\0"), None);
        assert_eq!(error_name(-4), "ALREADY_ON");
        assert_eq!(error_name(-42), "unknown PSCI error");
    }
}

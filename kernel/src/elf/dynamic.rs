//! The auxiliary vector a program starts with (ADR 0010).
//!
//! Every program gets one, as on Linux: musl's start code reads AT_RANDOM
//! (stack protector), AT_SECURE, AT_PHDR/AT_PHNUM (static TLS) and
//! AT_HWCAP, and its dynamic loader reads the program's headers and entry
//! point from it.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Auxiliary vector entry types (Linux `include/uapi/linux/auxvec.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum AuxType {
    /// End of vector.
    AtNull = 0,
    /// Program headers of the program.
    AtPhdr = 3,
    /// Size of one program header.
    AtPhent = 4,
    /// Number of program headers.
    AtPhnum = 5,
    /// Page size.
    AtPagesz = 6,
    /// Load bias of the interpreter (0 without one).
    AtBase = 7,
    /// Flags (always 0).
    AtFlags = 8,
    /// Entry point of the program.
    AtEntry = 9,
    /// Real user ID.
    AtUid = 11,
    /// Effective user ID.
    AtEuid = 12,
    /// Real group ID.
    AtGid = 13,
    /// Effective group ID.
    AtEgid = 14,
    /// String naming the platform ("x86_64").
    AtPlatform = 15,
    /// CPU features (x86: CPUID leaf 1 EDX).
    AtHwcap = 16,
    /// Clock ticks per second (times(2)).
    AtClktck = 17,
    /// Secure mode: the exec changed the effective IDs.
    AtSecure = 23,
    /// 16 random bytes.
    AtRandom = 25,
    /// More CPU features.
    AtHwcap2 = 26,
    /// File name of the program.
    AtExecfn = 31,
    /// Minimum signal stack size the kernel needs for a signal frame.
    AtMinsigstksz = 51,
}

/// One entry of the auxiliary vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuxVecEntry {
    pub type_id: AuxType,
    pub value: u64,
}

impl AuxVecEntry {
    pub fn new(type_id: AuxType, value: u64) -> Self {
        Self { type_id, value }
    }
}

/// What the vector says about the loaded program.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProgramAux {
    /// Address of its program header table, if a loaded segment holds it.
    pub phdr: Option<u64>,
    pub phent: u16,
    pub phnum: u16,
    /// Its entry point (biased), also when an interpreter runs first.
    pub entry: u64,
    /// Load bias of the interpreter, 0 without one.
    pub base: u64,
}

/// What it says about the process.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessAux {
    pub uid: u32,
    pub euid: u32,
    pub gid: u32,
    pub egid: u32,
    /// The exec changed the effective user or group ID.
    pub secure: bool,
}

/// Addresses of what the kernel put on the new stack.
#[derive(Debug, Clone, Copy, Default)]
pub struct StackAux {
    /// 16 random bytes.
    pub random: u64,
    /// The program's file name.
    pub execfn: u64,
    /// The platform string.
    pub platform: u64,
}

/// The platform string (AT_PLATFORM), as Linux reports it.
pub const PLATFORM: &str = if cfg!(target_arch = "x86_64") {
    "x86_64"
} else if cfg!(target_arch = "aarch64") {
    "aarch64"
} else {
    "riscv64"
};

/// Clock ticks per second (Linux's USER_HZ).
pub const CLOCK_TICKS: u64 = 100;

/// CPU feature words (AT_HWCAP, AT_HWCAP2).
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn hwcaps() -> (u64, u64) {
    // SAFETY: CPUID leaf 1 exists on every x86_64 processor and has no
    // side effects. AT_HWCAP2 bit 1 (FSGSBASE) is 0: the kernel does not
    // enable the user-mode FS/GS base instructions.
    let edx = unsafe { core::arch::x86_64::__cpuid(1).edx };
    (edx as u64, 0)
}

#[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
fn hwcaps() -> (u64, u64) {
    (0, 0)
}

/// The vector, in Linux's order (`create_elf_tables`), ending with AT_NULL.
/// AT_SYSINFO_EHDR is absent: there is no vDSO, so libc makes system calls.
#[cfg(feature = "alloc")]
pub fn aux_vector(
    program: &ProgramAux,
    process: &ProcessAux,
    stack: &StackAux,
) -> Vec<AuxVecEntry> {
    use AuxType::*;
    let (hwcap, hwcap2) = hwcaps();
    let mut aux = Vec::with_capacity(24);
    let mut push = |t, v| aux.push(AuxVecEntry::new(t, v));
    #[cfg(target_arch = "x86_64")]
    push(AtMinsigstksz, crate::process::signals::MIN_SIGNAL_FRAME);
    push(AtHwcap, hwcap);
    push(AtPagesz, crate::mm::PAGE_SIZE as u64);
    push(AtClktck, CLOCK_TICKS);
    // Without a loaded header table, no headers: a count with a null
    // address would send libc's scan to address 0.
    let phnum = program.phdr.map_or(0, |_| program.phnum as u64);
    push(AtPhdr, program.phdr.unwrap_or(0));
    push(AtPhent, program.phent as u64);
    push(AtPhnum, phnum);
    push(AtBase, program.base);
    push(AtFlags, 0);
    push(AtEntry, program.entry);
    push(AtUid, process.uid as u64);
    push(AtEuid, process.euid as u64);
    push(AtGid, process.gid as u64);
    push(AtEgid, process.egid as u64);
    push(AtSecure, process.secure as u64);
    push(AtRandom, stack.random);
    #[cfg(target_arch = "x86_64")]
    push(AtHwcap2, hwcap2);
    #[cfg(not(target_arch = "x86_64"))]
    let _ = hwcap2;
    push(AtExecfn, stack.execfn);
    push(AtPlatform, stack.platform);
    push(AtNull, 0);
    aux
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(aux: &[AuxVecEntry], t: AuxType) -> Option<u64> {
        aux.iter().find(|e| e.type_id == t).map(|e| e.value)
    }

    #[test]
    fn aux_type_numbers_are_linux_ones() {
        assert_eq!(AuxType::AtBase as u64, 7);
        assert_eq!(AuxType::AtEntry as u64, 9);
        assert_eq!(AuxType::AtPlatform as u64, 15);
        assert_eq!(AuxType::AtSecure as u64, 23);
        assert_eq!(AuxType::AtRandom as u64, 25);
        assert_eq!(AuxType::AtHwcap2 as u64, 26);
        assert_eq!(AuxType::AtExecfn as u64, 31);
        assert_eq!(AuxType::AtMinsigstksz as u64, 51);
    }

    #[test]
    fn vector_describes_program_process_and_stack() {
        let aux = aux_vector(
            &ProgramAux {
                phdr: Some(0x5555_5555_4040),
                phent: 56,
                phnum: 11,
                entry: 0x5555_5555_5000,
                base: 0x4000_0000_0000,
            },
            &ProcessAux {
                uid: 1000,
                euid: 0,
                gid: 100,
                egid: 100,
                secure: true,
            },
            &StackAux {
                random: 0x7FFF_FFFF_E000,
                execfn: 0x7FFF_FFFF_EFF0,
                platform: 0x7FFF_FFFF_EF00,
            },
        );
        assert_eq!(value(&aux, AuxType::AtPhdr), Some(0x5555_5555_4040));
        assert_eq!(value(&aux, AuxType::AtPhent), Some(56));
        assert_eq!(value(&aux, AuxType::AtPhnum), Some(11));
        assert_eq!(value(&aux, AuxType::AtPagesz), Some(4096));
        assert_eq!(value(&aux, AuxType::AtBase), Some(0x4000_0000_0000));
        assert_eq!(value(&aux, AuxType::AtEntry), Some(0x5555_5555_5000));
        assert_eq!(value(&aux, AuxType::AtUid), Some(1000));
        assert_eq!(value(&aux, AuxType::AtEuid), Some(0));
        assert_eq!(value(&aux, AuxType::AtSecure), Some(1));
        assert_eq!(value(&aux, AuxType::AtRandom), Some(0x7FFF_FFFF_E000));
        assert_eq!(value(&aux, AuxType::AtExecfn), Some(0x7FFF_FFFF_EFF0));
        assert_eq!(value(&aux, AuxType::AtPlatform), Some(0x7FFF_FFFF_EF00));
        assert_eq!(value(&aux, AuxType::AtClktck), Some(100));
        assert_eq!(
            value(&aux, AuxType::AtMinsigstksz),
            Some(crate::process::signals::MIN_SIGNAL_FRAME)
        );
        // Linux's order: AT_PHDR before AT_BASE before AT_ENTRY before
        // AT_RANDOM; AT_NULL last and only there.
        let pos = |t| aux.iter().position(|e| e.type_id == t).unwrap();
        assert!(pos(AuxType::AtPhdr) < pos(AuxType::AtBase));
        assert!(pos(AuxType::AtBase) < pos(AuxType::AtEntry));
        assert!(pos(AuxType::AtEntry) < pos(AuxType::AtRandom));
        assert_eq!(aux.last(), Some(&AuxVecEntry::new(AuxType::AtNull, 0)));
        assert_eq!(
            aux.iter().filter(|e| e.type_id == AuxType::AtNull).count(),
            1
        );
    }

    /// Without a loaded header table, AT_PHNUM is 0 rather than a count of
    /// headers at address 0.
    #[test]
    fn no_header_count_without_header_address() {
        let aux = aux_vector(
            &ProgramAux {
                phdr: None,
                phent: 56,
                phnum: 5,
                ..Default::default()
            },
            &ProcessAux::default(),
            &StackAux::default(),
        );
        assert_eq!(value(&aux, AuxType::AtPhdr), Some(0));
        assert_eq!(value(&aux, AuxType::AtPhnum), Some(0));
    }
}

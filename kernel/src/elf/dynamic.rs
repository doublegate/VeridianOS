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

/// AArch64: Linux's arm64 AT_HWCAP bits from ID_AA64PFR0_EL1 and
/// ID_AA64ISAR0_EL1 (`arch/arm64/kernel/cpufeature.c`): FP (0) and ASIMD
/// (1) when implemented (field not 0xF), AES (3), PMULL (4), SHA1 (5),
/// SHA2 (6), CRC32 (7), LSE ATOMICS (8), FPHP (9) and ASIMDHP (10). EVTSTRM
/// and CPUID are not set: the kernel enables no timer event stream and does
/// not emulate EL0 reads of the ID registers.
pub fn arm64_hwcap(pfr0: u64, isar0: u64) -> u64 {
    let field = |r: u64, at: u32| (r >> at) & 0xF;
    let mut hwcap = 0;
    let (fp, simd) = (field(pfr0, 16), field(pfr0, 20));
    if fp != 0xF {
        hwcap |= 1 << 0;
        if fp == 1 {
            hwcap |= 1 << 9;
        }
    }
    if simd != 0xF {
        hwcap |= 1 << 1;
        if simd == 1 {
            hwcap |= 1 << 10;
        }
    }
    let aes = field(isar0, 4);
    if aes >= 1 {
        hwcap |= 1 << 3;
    }
    if aes >= 2 {
        hwcap |= 1 << 4;
    }
    for (at, bit) in [(8, 5), (12, 6), (16, 7)] {
        if field(isar0, at) >= 1 {
            hwcap |= 1 << bit;
        }
    }
    if field(isar0, 20) >= 2 {
        hwcap |= 1 << 8;
    }
    hwcap
}

/// AArch64: Linux's arm64 AT_HWCAP2 bits (`arch/arm64/kernel/cpufeature.c`)
/// for instructions EL0 can use with no kernel support, from
/// ID_AA64ISAR0/1/2_EL1: DCPODP (0), FLAGM2 (7), FRINT (8), I8MM (13), BF16
/// (14), DGH (15), RNG (16), EBF16 (32), CSSC (34), RPRFM (35), HBC (44),
/// LRCPC3 (46). Not reported, as the kernel does not provide what they
/// need: BTI (no PROT_BTI or guarded pages), MOPS (exceptions on migration),
/// WFxT, ECV, AFP, RPRES, and the SVE, SME and MTE families.
pub fn arm64_hwcap2(isar0: u64, isar1: u64, isar2: u64) -> u64 {
    let field = |r: u64, at: u32| (r >> at) & 0xF;
    // (register, field position, minimum value, AT_HWCAP2 bit)
    let caps = [
        (isar1, 0, 2, 0),   // DPB2: DC CVADP
        (isar0, 52, 2, 7),  // TS: FLAGM2
        (isar1, 32, 1, 8),  // FRINTTS
        (isar1, 52, 1, 13), // I8MM
        (isar1, 44, 1, 14), // BF16
        (isar1, 48, 1, 15), // DGH
        (isar0, 60, 1, 16), // RNDR
        (isar1, 44, 2, 32), // EBF16
        (isar2, 52, 1, 34), // CSSC
        (isar2, 48, 1, 35), // RPRFM
        (isar2, 20, 1, 44), // BC: HBC
        (isar1, 20, 3, 46), // LRCPC3
    ];
    caps.iter()
        .filter(|&&(reg, at, min, _)| field(reg, at) >= min)
        .fold(0, |acc, &(_, _, _, bit)| acc | 1 << bit)
}

/// RISC-V: Linux's AT_HWCAP, one bit per single-letter extension (bit
/// `letter - 'a'`) of those it reports (I, M, A, F, D, C; V is left out:
/// no vector state is switched, Linux likewise reports V only with its
/// vector support). F and D only with `user_fp` (the kernel switches the
/// F/D registers). `has` says whether the harts have a letter.
pub fn riscv_hwcap(has: impl Fn(u8) -> bool, user_fp: bool) -> u64 {
    b"imafdc"
        .iter()
        .filter(|&&l| user_fp || !matches!(l, b'f' | b'd'))
        .filter(|&&l| has(l))
        .fold(0, |acc, &l| acc | 1 << (l - b'a'))
}

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
fn hwcaps() -> (u64, u64) {
    let (pfr0, isar0, isar1, isar2): (u64, u64, u64, u64);
    // SAFETY: the ID registers are readable at EL1 and have no side effects
    // (ID_AA64ISAR2_EL1 reads as zero before Armv8.7, by its generic name).
    unsafe {
        core::arch::asm!("mrs {0}, id_aa64pfr0_el1", out(reg) pfr0, options(nomem, nostack));
        core::arch::asm!("mrs {0}, id_aa64isar0_el1", out(reg) isar0, options(nomem, nostack));
        core::arch::asm!("mrs {0}, id_aa64isar1_el1", out(reg) isar1, options(nomem, nostack));
        core::arch::asm!("mrs {0}, s3_0_c0_c6_2", out(reg) isar2, options(nomem, nostack));
    }
    (arm64_hwcap(pfr0, isar0), arm64_hwcap2(isar0, isar1, isar2))
}

#[cfg(all(target_arch = "riscv64", target_os = "none"))]
fn hwcaps() -> (u64, u64) {
    (crate::arch::riscv64::fpu::hwcap(), 0)
}

#[cfg(not(target_os = "none"))]
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
    // The stack a signal frame needs (x86_64 always; AArch64 and RISC-V
    // where their frames exist).
    #[cfg(any(
        target_arch = "x86_64",
        all(
            feature = "alloc",
            target_os = "none",
            any(target_arch = "aarch64", target_arch = "riscv64")
        )
    ))]
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
    // As Linux: x86_64 and arm64 define ELF_HWCAP2, RISC-V does not.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    push(AtHwcap2, hwcap2);
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
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
    fn arm64_hwcap2_reports_only_plain_instructions() {
        // Cortex-A72: none of them.
        assert_eq!(arm64_hwcap2(0x0001_1120, 0, 0), 0);
        // ISAR0: TS = 2 (FLAGM2), RNDR = 1; ISAR1: DPB = 2, LRCPC = 3,
        // FRINTTS = 1, BF16 = 2 (EBF16), DGH = 1, I8MM = 1; ISAR2: CSSC,
        // RPRFM, BC = 1.
        let isar0 = (1 << 60) | (2 << 52);
        let isar1 = 2 | (3 << 20) | (1 << 32) | (2 << 44) | (1 << 48) | (1 << 52);
        let isar2 = (1 << 52) | (1 << 48) | (1 << 20);
        let want = [0, 7, 8, 13, 14, 15, 16, 32, 34, 35, 44, 46]
            .iter()
            .fold(0u64, |a, b| a | 1 << b);
        assert_eq!(arm64_hwcap2(isar0, isar1, isar2), want);
        // Lower levels: TS = 1 is FLAGM only (an AT_HWCAP bit), DPB = 1 and
        // LRCPC = 2 not enough, BF16 = 1 is BF16 without EBF16.
        let lower = arm64_hwcap2(1 << 52, 1 | (2 << 20) | (1 << 44), 0);
        assert_eq!(lower, 1 << 14);
        // Kernel-dependent features stay clear: MOPS (ISAR2[19:16]), WFxT
        // (ISAR2[3:0]), RPRES (ISAR2[7:4]).
        assert_eq!(arm64_hwcap2(0, 0, (1 << 16) | 1 | (1 << 4)), 0);
    }

    #[test]
    fn arm64_hwcap_follows_the_id_registers() {
        // Cortex-A72 (QEMU): FP and AdvSIMD (0), AES + PMULL (2), SHA1,
        // SHA2, CRC32; no LSE atomics, no half precision.
        let pfr0 = 0x0000_0000_0000_0011;
        let isar0 = 0x0000_0000_0001_1120;
        assert_eq!(arm64_hwcap(pfr0, isar0), 0b1111_1011);
        // No FP/AdvSIMD (0xF): neither bit; half precision (1) and LSE (2).
        assert_eq!(arm64_hwcap(0xFF << 16, 0), 0);
        assert_eq!(arm64_hwcap(0x11 << 16, 2 << 20), 0b111_0000_0011);
    }

    #[test]
    fn riscv_hwcap_has_the_base_letters_the_kernel_supports() {
        let rv64gc = |l: u8| b"imafdcv".contains(&l);
        let bit = |l: u8| 1u64 << (l - b'a');
        let base = bit(b'i') | bit(b'm') | bit(b'a') | bit(b'c');
        // V is never reported (no vector state is switched); F and D only
        // when user FP is.
        assert_eq!(riscv_hwcap(rv64gc, true), base | bit(b'f') | bit(b'd'));
        assert_eq!(riscv_hwcap(rv64gc, false), base);
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

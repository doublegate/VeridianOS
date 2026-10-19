//! Which memory a process may make writable and executable (ADR 0017).
//!
//! - **W^X, the default:** no mapping is writable and executable at once; mmap
//!   and mprotect with PROT_WRITE|PROT_EXEC are EACCES (Linux allows them).
//! - **The opt-in:** a program whose ELF note asks for it
//!   (`VERIDIAN_FCTL_WXNEEDED`, `<veridian/wxneeded.h>`) gets Linux's
//!   behaviour. Only the main program's note counts, at exec; nothing turns W^X
//!   off at run time.
//! - **MDWE (`PR_SET_MDWE`), as Linux:** refuses writable-and-executable memory
//!   whatever the note says, and memory becoming executable that was not. One
//!   way only; kept across fork and exec unless set with `PR_MDWE_NO_INHERIT`.
//!
//! Kept per address space, as Linux keeps MDWE in the mm.

/// `PR_MDWE_REFUSE_EXEC_GAIN`: refuse W+X and gaining exec.
pub const PR_MDWE_REFUSE_EXEC_GAIN: u32 = 1;
/// `PR_MDWE_NO_INHERIT`: not kept across fork or exec.
pub const PR_MDWE_NO_INHERIT: u32 = 2;
/// The MDWE bits, as `PR_GET_MDWE` reports them.
const MDWE_BITS: u32 = PR_MDWE_REFUSE_EXEC_GAIN | PR_MDWE_NO_INHERIT;
/// The program opted in to writable-and-executable memory.
const WX_ALLOWED: u32 = 1 << 8;

const PROT_WRITE: usize = 0x2;
const PROT_EXEC: usize = 0x4;

/// Why `PR_SET_MDWE` refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdweError {
    /// Unknown bits, NO_INHERIT alone, or nonzero unused arguments (EINVAL).
    Invalid,
    /// An attempt to change bits already set (EPERM).
    Locked,
}

/// One address space's policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WxPolicy(u32);

impl WxPolicy {
    /// The policy as stored.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The bits to store.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The MDWE bits, as `PR_GET_MDWE` returns them.
    pub const fn mdwe(self) -> u32 {
        self.0 & MDWE_BITS
    }

    fn refuses_exec_gain(self) -> bool {
        self.0 & PR_MDWE_REFUSE_EXEC_GAIN != 0
    }

    /// Whether the program opted in (and MDWE has not taken it away).
    pub const fn wx_allowed(self) -> bool {
        self.0 & WX_ALLOWED != 0 && self.0 & PR_MDWE_REFUSE_EXEC_GAIN == 0
    }

    /// Whether new memory with `prot` is refused (mmap; EACCES).
    pub fn refuses_map(self, prot: usize) -> bool {
        prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0 && !self.wx_allowed()
    }

    /// Whether the loader refuses a segment with `prot`: under MDWE only
    /// (Linux maps a program's W+X segment through mmap, which MDWE
    /// refuses); W^X itself leaves the program's own segments as linked.
    pub fn refuses_segment(self, prot: usize) -> bool {
        self.refuses_exec_gain() && prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0
    }

    /// Whether mprotect to `prot` of a mapping that was executable
    /// (`was_exec`) or not is refused (EACCES): as `refuses_map`, and under
    /// MDWE memory may not become executable (Linux's
    /// `map_deny_write_exec`).
    pub fn refuses_protect(self, prot: usize, was_exec: bool) -> bool {
        self.refuses_map(prot) || (self.refuses_exec_gain() && prot & PROT_EXEC != 0 && !was_exec)
    }

    /// `PR_SET_MDWE(bits, a3, a4, a5)`, as Linux's `prctl_set_mdwe`.
    pub fn set_mdwe(self, bits: usize, a3: usize, a4: usize, a5: usize) -> Result<Self, MdweError> {
        if a3 != 0 || a4 != 0 || a5 != 0 || bits & !(MDWE_BITS as usize) != 0 {
            return Err(MdweError::Invalid);
        }
        let bits = bits as u32;
        if bits & PR_MDWE_NO_INHERIT != 0 && bits & PR_MDWE_REFUSE_EXEC_GAIN == 0 {
            return Err(MdweError::Invalid);
        }
        let current = self.mdwe();
        if current != 0 && current != bits {
            return Err(MdweError::Locked);
        }
        Ok(Self(self.0 | bits))
    }

    /// MDWE in a new address space made from this one (fork, exec): kept,
    /// unless set with NO_INHERIT (Linux's `mmf_init_legacy_flags`).
    fn inherited_mdwe(self) -> u32 {
        if self.0 & PR_MDWE_NO_INHERIT != 0 {
            0
        } else {
            self.mdwe()
        }
    }

    /// The policy of a fork child: MDWE as inherited, the opt-in kept (it
    /// runs the same program).
    pub fn forked(self) -> Self {
        Self(self.inherited_mdwe() | (self.0 & WX_ALLOWED))
    }

    /// The policy after exec of a program that asks for W+X (`wx_needed`)
    /// or not: MDWE as inherited, the opt-in from the new program only.
    pub fn exec(self, wx_needed: bool) -> Self {
        Self(self.inherited_mdwe() | if wx_needed { WX_ALLOWED } else { 0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: usize = 1;
    const W: usize = PROT_WRITE;
    const X: usize = PROT_EXEC;

    #[test]
    fn wx_is_refused_unless_the_program_opted_in() {
        let p = WxPolicy::default();
        assert!(p.refuses_map(R | W | X));
        assert!(p.refuses_map(W | X));
        assert!(!p.refuses_map(R | X));
        assert!(!p.refuses_map(R | W));
        assert!(p.refuses_protect(R | W | X, true));
        // W^X alone does not refuse gaining exec (mprotect RW -> RX).
        assert!(!p.refuses_protect(R | X, false));
        assert!(!p.refuses_segment(R | W | X));

        let opted = p.exec(true);
        assert!(opted.wx_allowed());
        assert!(!opted.refuses_map(R | W | X));
        assert!(!opted.refuses_protect(R | W | X, false));
    }

    #[test]
    fn mdwe_follows_linux() {
        let p = WxPolicy::default().exec(true);
        assert_eq!(p.mdwe(), 0);
        // Argument checks.
        assert_eq!(p.set_mdwe(4, 0, 0, 0), Err(MdweError::Invalid));
        assert_eq!(
            p.set_mdwe(PR_MDWE_NO_INHERIT as usize, 0, 0, 0),
            Err(MdweError::Invalid)
        );
        assert_eq!(p.set_mdwe(1, 1, 0, 0), Err(MdweError::Invalid));
        // Setting nothing when nothing is set is allowed and changes nothing.
        assert_eq!(p.set_mdwe(0, 0, 0, 0), Ok(p));

        let m = p.set_mdwe(1, 0, 0, 0).unwrap();
        assert_eq!(m.mdwe(), PR_MDWE_REFUSE_EXEC_GAIN);
        // Stronger than the note.
        assert!(!m.wx_allowed());
        assert!(m.refuses_map(R | W | X));
        assert!(m.refuses_segment(R | W | X));
        assert!(!m.refuses_map(R | X));
        // No gaining exec; keeping it is fine.
        assert!(m.refuses_protect(R | X, false));
        assert!(!m.refuses_protect(R | X, true));
        assert!(!m.refuses_protect(R, false));
        // One way: same bits again succeed, anything else is EPERM.
        assert_eq!(m.set_mdwe(1, 0, 0, 0), Ok(m));
        assert_eq!(m.set_mdwe(0, 0, 0, 0), Err(MdweError::Locked));
        assert_eq!(m.set_mdwe(3, 0, 0, 0), Err(MdweError::Locked));
    }

    #[test]
    fn mdwe_inheritance_follows_linux() {
        let m = WxPolicy::default().set_mdwe(1, 0, 0, 0).unwrap();
        assert_eq!(m.forked().mdwe(), 1);
        assert_eq!(m.exec(false).mdwe(), 1);
        // The note cannot undo inherited MDWE.
        assert!(m.exec(true).refuses_map(R | W | X));

        let n = WxPolicy::default().set_mdwe(3, 0, 0, 0).unwrap();
        assert_eq!(n.mdwe(), 3);
        assert_eq!(n.forked().mdwe(), 0);
        assert_eq!(n.exec(false).mdwe(), 0);
    }

    #[test]
    fn the_opt_in_belongs_to_the_program() {
        let opted = WxPolicy::default().exec(true);
        assert!(
            opted.forked().wx_allowed(),
            "a fork child runs the same program"
        );
        assert!(
            !opted.exec(false).wx_allowed(),
            "exec of another program drops it"
        );
        let back = WxPolicy::from_bits(opted.bits());
        assert_eq!(back, opted);
    }
}

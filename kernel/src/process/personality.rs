//! Linux's process personality (`include/uapi/linux/personality.h`): an
//! execution domain in the low byte (only PER_LINUX here, as Linux has no
//! other domain left) and bug-emulation flags above it. personality(2)
//! stores any value; these flags change what the kernel does:
//!
//! - READ_IMPLIES_EXEC: readable memory is also executable (mmap, mprotect),
//!   honoured only for a program with the W^X opt-in (ADR 0017,
//!   `mm::wx::WxPolicy::read_implies_exec`);
//! - STICKY_TIMEOUTS: select, pselect6 and ppoll leave their timeout unchanged;
//! - UNAME26: uname reports a 2.6 release ([`uname26_release`]).
//!
//! Per thread, as on Linux, and inherited by every thread and process it
//! creates.

use alloc::string::String;

pub const UNAME26: u32 = 0x0002_0000;
pub const ADDR_NO_RANDOMIZE: u32 = 0x0004_0000;
pub const MMAP_PAGE_ZERO: u32 = 0x0010_0000;
pub const ADDR_COMPAT_LAYOUT: u32 = 0x0020_0000;
pub const READ_IMPLIES_EXEC: u32 = 0x0040_0000;
pub const STICKY_TIMEOUTS: u32 = 0x0400_0000;

/// The flags a set-user-ID, set-group-ID or capability-granting exec
/// clears (Linux's PER_CLEAR_ON_SETID).
pub const PER_CLEAR_ON_SETID: u32 =
    READ_IMPLIES_EXEC | ADDR_NO_RANDOMIZE | ADDR_COMPAT_LAYOUT | MMAP_PAGE_ZERO;

/// personality(2)'s query value: report without changing.
pub const QUERY: u32 = 0xFFFF_FFFF;

/// The personality after an exec: READ_IMPLIES_EXEC is cleared, as Linux's
/// x86-64 set_personality_64bit does for every 64-bit program (none asks
/// for it in its headers), and a `secure` exec (one that grants privilege)
/// clears PER_CLEAR_ON_SETID too.
pub fn after_exec(personality: u32, secure: bool) -> u32 {
    let cleared = if secure { PER_CLEAR_ON_SETID } else { 0 };
    personality & !(READ_IMPLIES_EXEC | cleared)
}

/// The release UNAME26 reports for `release`, as Linux's override_release:
/// "2.6." and 60 plus the minor version, then whatever follows the leading
/// digits and dots (at most three numbers) -- "2.6.63-veridian" for
/// "7.3.0-veridian".
pub fn uname26_release(release: &str) -> String {
    let bytes = release.as_bytes();
    let mut rest = 0;
    let mut dots = 0;
    while rest < bytes.len() {
        let b = bytes[rest];
        if b == b'.' {
            dots += 1;
            if dots >= 3 {
                break;
            }
        } else if !b.is_ascii_digit() {
            break;
        }
        rest += 1;
    }
    let minor: u32 = release
        .split('.')
        .nth(1)
        .and_then(|m| {
            let end = m.find(|c: char| !c.is_ascii_digit()).unwrap_or(m.len());
            m[..end].parse().ok()
        })
        .unwrap_or(0);
    alloc::format!("2.6.{}{}", minor + 60, &release[rest..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_have_linuxs_values() {
        assert_eq!(UNAME26, 0x0002_0000);
        assert_eq!(PER_CLEAR_ON_SETID, 0x0074_0000);
    }

    #[test]
    fn exec_clears_read_implies_exec_and_secure_exec_more() {
        let p = READ_IMPLIES_EXEC | ADDR_NO_RANDOMIZE | STICKY_TIMEOUTS | 0x8;
        assert_eq!(
            after_exec(p, false),
            ADDR_NO_RANDOMIZE | STICKY_TIMEOUTS | 0x8
        );
        assert_eq!(after_exec(p, true), STICKY_TIMEOUTS | 0x8);
    }

    #[test]
    fn uname26_rewrites_the_release_as_linux_does() {
        assert_eq!(uname26_release("7.3.0-veridian"), "2.6.63-veridian");
        assert_eq!(uname26_release("3.0.1"), "2.6.60");
        // A fourth number stays in the rest, as Linux stops at three.
        assert_eq!(uname26_release("4.12.3.9-x"), "2.6.72.9-x");
    }
}

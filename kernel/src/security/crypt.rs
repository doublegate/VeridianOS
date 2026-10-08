//! SHA-512-crypt (`$6$`), the password hash of `/etc/shadow` (N-131).
//!
//! Ulrich Drepper's "Unix crypt using SHA-256 and SHA-512" (version 0.6),
//! as glibc, libxcrypt and musl implement it, so the kernel and the C
//! libraries (BusyBox's `login`, `su` and `passwd` through musl's `crypt`)
//! read and write the same hashes. The salt is at most 16 characters; the
//! round count defaults to 5000 and is clamped to 1000..=999,999,999, as
//! the specification and musl do (libxcrypt refuses fewer than 1000).

extern crate alloc;

use alloc::{string::String, vec::Vec};

/// Rounds when a setting names none.
pub const DEFAULT_ROUNDS: u32 = 5000;
const MIN_ROUNDS: u32 = 1000;
const MAX_ROUNDS: u32 = 999_999_999;
const MAX_SALT: usize = 16;
/// Longest password hashed, as musl's KEY_MAX: the algorithm hashes the
/// password repeated as many times as it is long, so the work and the
/// buffer grow with its square.
pub const MAX_KEY: usize = 256;

/// crypt(3)'s base-64 alphabet.
const ITOA64: &[u8; 64] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn sha512(data: &[u8]) -> [u8; 64] {
    *crate::crypto::hash::sha512(data).as_bytes()
}

/// `n` bytes made of `block` repeated (the spec's P and S sequences).
fn repeat_to(block: &[u8; 64], n: usize) -> Vec<u8> {
    block.iter().copied().cycle().take(n).collect()
}

/// A `$6$` setting, parsed: the round count (and whether it was given)
/// and the salt.
struct Setting<'a> {
    rounds: u32,
    rounds_given: bool,
    salt: &'a [u8],
}

fn parse_setting(setting: &str) -> Option<Setting<'_>> {
    let rest = setting.strip_prefix("$6$")?;
    let (rounds, rounds_given, rest) = match rest.strip_prefix("rounds=") {
        Some(r) => {
            let end = r.find('$')?;
            let digits = &r[..end];
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let n: u64 = digits.parse().ok()?;
            let n = n.clamp(MIN_ROUNDS as u64, MAX_ROUNDS as u64) as u32;
            (n, true, &r[end + 1..])
        }
        None => (DEFAULT_ROUNDS, false, rest),
    };
    // The salt ends at the next '$' (or the end), and is cut to 16.
    let salt = rest.split('$').next().unwrap_or("").as_bytes();
    let salt = &salt[..salt.len().min(MAX_SALT)];
    if salt.iter().any(|&b| b == b':' || b == b'\n') {
        return None;
    }
    Some(Setting {
        rounds,
        rounds_given,
        salt,
    })
}

/// crypt(`password`, `setting`) for a `$6$` setting: the full hash string,
/// or `None` for a setting that is not SHA-512-crypt or a password longer
/// than [`MAX_KEY`] (musl's crypt refuses those too).
pub fn sha512_crypt(password: &[u8], setting: &str) -> Option<String> {
    if password.len() > MAX_KEY {
        return None;
    }
    let s = parse_setting(setting)?;
    let (p, salt) = (password, s.salt);

    // B = SHA512(P S P)
    let mut buf = Vec::with_capacity(2 * p.len() + salt.len() + 128);
    buf.extend_from_slice(p);
    buf.extend_from_slice(salt);
    buf.extend_from_slice(p);
    let b = sha512(&buf);

    // A = SHA512(P S, B for each byte of P, then B or P per bit of len(P))
    buf.clear();
    buf.extend_from_slice(p);
    buf.extend_from_slice(salt);
    buf.extend_from_slice(&repeat_to(&b, p.len()));
    let mut len = p.len();
    while len > 0 {
        if len & 1 != 0 {
            buf.extend_from_slice(&b);
        } else {
            buf.extend_from_slice(p);
        }
        len >>= 1;
    }
    let mut c = sha512(&buf);

    // DP = SHA512(P repeated len(P) times); P' its first len(P) bytes.
    buf.clear();
    for _ in 0..p.len() {
        buf.extend_from_slice(p);
    }
    let pseq = repeat_to(&sha512(&buf), p.len());

    // DS = SHA512(S repeated 16 + A[0] times); S' its first len(S) bytes.
    buf.clear();
    for _ in 0..16 + c[0] as usize {
        buf.extend_from_slice(salt);
    }
    let sseq = repeat_to(&sha512(&buf), salt.len());

    for i in 0..s.rounds {
        buf.clear();
        if i & 1 != 0 {
            buf.extend_from_slice(&pseq);
        } else {
            buf.extend_from_slice(&c);
        }
        if i % 3 != 0 {
            buf.extend_from_slice(&sseq);
        }
        if i % 7 != 0 {
            buf.extend_from_slice(&pseq);
        }
        if i & 1 != 0 {
            buf.extend_from_slice(&c);
        } else {
            buf.extend_from_slice(&pseq);
        }
        c = sha512(&buf);
    }
    // The password is not left in the kernel heap.
    wipe(&mut buf);
    let mut pseq = pseq;
    wipe(&mut pseq);

    let mut out = String::from("$6$");
    if s.rounds_given {
        out.push_str("rounds=");
        out.push_str(&alloc::format!("{}", s.rounds));
        out.push('$');
    }
    out.push_str(core::str::from_utf8(salt).ok()?);
    out.push('$');
    const ORDER: [(usize, usize, usize); 21] = [
        (0, 21, 42),
        (22, 43, 1),
        (44, 2, 23),
        (3, 24, 45),
        (25, 46, 4),
        (47, 5, 26),
        (6, 27, 48),
        (28, 49, 7),
        (50, 8, 29),
        (9, 30, 51),
        (31, 52, 10),
        (53, 11, 32),
        (12, 33, 54),
        (34, 55, 13),
        (56, 14, 35),
        (15, 36, 57),
        (37, 58, 16),
        (59, 17, 38),
        (18, 39, 60),
        (40, 61, 19),
        (62, 20, 41),
    ];
    for (x, y, z) in ORDER {
        b64_from_24bit(&mut out, c[x], c[y], c[z], 4);
    }
    b64_from_24bit(&mut out, 0, 0, c[63], 2);
    Some(out)
}

fn b64_from_24bit(out: &mut String, b2: u8, b1: u8, b0: u8, n: usize) {
    let mut w = ((b2 as u32) << 16) | ((b1 as u32) << 8) | b0 as u32;
    for _ in 0..n {
        out.push(ITOA64[(w & 0x3f) as usize] as char);
        w >>= 6;
    }
}

fn wipe(buf: &mut [u8]) {
    for byte in buf.iter_mut() {
        // SAFETY: `byte` is a valid, aligned reference into `buf`; volatile
        // so the wipe is not optimized away as a dead store.
        unsafe { core::ptr::write_volatile(byte, 0) };
    }
}

/// A new hash of `password`: a random 16-character salt and `rounds`.
/// `None` for a password longer than [`MAX_KEY`].
pub fn hash_password(password: &str, rounds: u32) -> Option<String> {
    let mut random = [0u8; MAX_SALT];
    let _ = crate::crypto::random::get_random().fill_bytes(&mut random);
    let salt: String = random
        .iter()
        .map(|&b| ITOA64[(b & 0x3f) as usize] as char)
        .collect();
    let setting = alloc::format!("$6$rounds={}${}", rounds, salt);
    sha512_crypt(password.as_bytes(), &setting)
}

/// Whether `password` hashes to `stored` (a `$6$` hash), compared in
/// constant time. Any other stored form -- `!`, `*`, an empty field, a
/// hash of another scheme -- matches no password.
pub fn verify_password(password: &str, stored: &str) -> bool {
    let Some(computed) = sha512_crypt(password.as_bytes(), stored) else {
        return false;
    };
    computed.len() == stored.len()
        && crate::crypto::constant_time::ct_eq_bytes(computed.as_bytes(), stored.as_bytes()) == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `setting` with `password` gives `expected`, which verifies.
    fn check(setting: &str, password: &str, expected: &str) {
        assert_eq!(
            sha512_crypt(password.as_bytes(), setting).as_deref(),
            Some(expected),
            "{}",
            setting
        );
        // A full hash is its own setting.
        assert!(verify_password(password, expected));
    }

    // The specification's vectors, checked against glibc's crypt(3). Its
    // two with 77,777 and 123,456 rounds take minutes in an unoptimized
    // build; they are replaced by glibc's results for the same passwords
    // and salts (the 16-character salt boundary included) at 1000 rounds.
    // One vector per test, so they run in parallel.

    #[test]
    fn vector_default_rounds() {
        check(
            "$6$saltstring",
            "Hello world!",
            "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/\
             O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1",
        );
    }

    #[test]
    fn vector_long_salt_is_cut() {
        check(
            "$6$rounds=10000$saltstringsaltstring",
            "Hello world!",
            "$6$rounds=10000$saltstringsaltst$OW1/O6BYHV6BcXZu8QVeXbDWra3Oeqh0sbHbbMCVNSnCM/\
             UrjmM0Dp8vOuZeHBy/YTBmSK6H9qs/y3RnOaw5v.",
        );
    }

    #[test]
    fn vector_explicit_default_rounds() {
        check(
            "$6$rounds=5000$toolongsaltstring",
            "This is just a test",
            "$6$rounds=5000$toolongsaltstrin$lQ8jolhgVRVhY4b5pZKaysCLi0QBxGoNeKQzQ3glMhwllF7oGDZxUhx1yxdYcz/e1JSbq3y6JMxxl8audkUEm0",
        );
    }

    #[test]
    fn vector_long_password() {
        check(
            "$6$rounds=1400$anotherlongsaltstring",
            "a very much longer text to encrypt.  This one even stretches over morethan one line.",
            "$6$rounds=1400$anotherlongsalts$POfYwTEok97VWcjxIiSOjiykti.o/pQs.\
             wPvMxQ6Fm7I6IoYN3CmLs66x9t0oSwbtEW7o7UmJEiDwGqd8p4ur1",
        );
    }

    #[test]
    fn vectors_at_minimum_rounds() {
        // Too few rounds are raised to the minimum (the specification;
        // musl too).
        check(
            "$6$rounds=10$roundstoolow",
            "the minimum number is still observed",
            "$6$rounds=1000$roundstoolow$kUMsbe306n21p9R.FRkW3IGn.\
             S9NPN0x50YhH1xhLsPuWGsUSklZt58jaTfF4ZEQpyUNGc0dqbpBYYBaHHrsX.",
        );
        check(
            "$6$rounds=1000$short",
            "we have a short salt string but not a short password",
            "$6$rounds=1000$short$yKjiOaKKa9hmO09NoaGIIFVrNG7WY8CArzJJlZJ5viSZrI9TzqYa2t7AA6THmblNo18mGtDJZYi2nxBwwsjsN.",
        );
        check(
            "$6$rounds=1000$asaltof16chars..",
            "a short string",
            "$6$rounds=1000$asaltof16chars..$rk5zk5.ds1b1IiNEv9FYP3.vld6KmfGD7JtlY/\
             LdCUSQLhz0267kfFr6WB9IUJJQ0Zy.6XIlEm1bhq.gf4bRe/",
        );
        // An empty password and an empty salt.
        check(
            "$6$rounds=1000$emptypass",
            "",
            "$6$rounds=1000$emptypass$3wLYQw5ctLLSwtsjhvfScdsOgwSMcdlQ5UfbvE7jkiphunABxPKG.\
             vWD59wu03D7nl5.IRp3aELUUD1m3iyTd/",
        );
        check(
            "$6$rounds=1000$",
            "x",
            "$6$rounds=1000$$MwL1ngOSTyhRTmswE6q2bTvDqHdFuhV10m2l0x3JOy.OEau0xfOpeR/\
             0OC9iLEfgib0feJ9KJveLUAQeA8NXr1",
        );
        assert!(!verify_password(
            "X",
            "$6$rounds=1000$$MwL1ngOSTyhRTmswE6q2bTvDqHdFuhV10m2l0x3JOy.OEau0xfOpeR/\
             0OC9iLEfgib0feJ9KJveLUAQeA8NXr1"
        ));
    }

    #[test]
    fn overlong_keys_hash_to_nothing() {
        let long = [b'a'; MAX_KEY + 1];
        assert_eq!(sha512_crypt(&long, "$6$rounds=1000$salt"), None);
        let at_limit = [b'a'; MAX_KEY];
        assert!(sha512_crypt(&at_limit, "$6$rounds=1000$salt").is_some());
    }

    #[test]
    fn other_forms_match_nothing() {
        for stored in [
            "",
            "!",
            "*",
            "!$6$x$y",
            "$1$abc$def",
            "$6$rounds=x$s$h",
            "$6$a:b$h",
        ] {
            assert!(!verify_password("", stored), "{}", stored);
            assert!(!verify_password("password", stored), "{}", stored);
        }
    }

    #[test]
    fn new_hashes_verify_and_differ() {
        let a = hash_password("secret", 1000).unwrap();
        let b = hash_password("secret", 1000).unwrap();
        assert_eq!(hash_password(&"a".repeat(MAX_KEY + 1), 1000), None);
        assert!(a.starts_with("$6$rounds=1000$"));
        assert_ne!(a, b, "salts are random");
        assert!(verify_password("secret", &a) && verify_password("secret", &b));
        assert!(!verify_password("Secret", &a));
    }
}

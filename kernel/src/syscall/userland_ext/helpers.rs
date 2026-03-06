//! Shared helper functions for userland_ext submodules
//!
//! No-std compatible parsing and formatting utilities used across
//! the io_uring, ptrace, coredump, users, privilege, and cron modules.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::string::String;

/// Parse a u8 from a string (no_std compatible)
pub(crate) fn parse_u8(s: &str) -> Option<u8> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut result: u16 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u16)?;
        if result > 255 {
            return None;
        }
    }
    Some(result as u8)
}

/// Parse a u32 from a string (no_std compatible)
pub(crate) fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut result: u64 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u64)?;
        if result > u32::MAX as u64 {
            return None;
        }
    }
    Some(result as u32)
}

/// Parse a u64 from a string (no_std compatible)
pub(crate) fn parse_u64(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut result: u64 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(result)
}

/// Push a u32 as decimal string to a String (no_std compatible)
pub(crate) fn push_u32_str(s: &mut String, val: u32) {
    if val == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut pos = buf.len();
    let mut v = val;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    for &b in &buf[pos..] {
        s.push(b as char);
    }
}

/// Push a u64 as decimal string to a String (no_std compatible)
pub(crate) fn push_u64_str(s: &mut String, val: u64) {
    if val == 0 {
        s.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut pos = buf.len();
    let mut v = val;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    for &b in &buf[pos..] {
        s.push(b as char);
    }
}

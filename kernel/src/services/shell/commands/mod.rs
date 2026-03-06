//! Built-in command implementations for the VeridianOS shell.
//!
//! This module contains all built-in shell commands organized by category:
//! - [`filesystem`] - Directory navigation and file operations
//! - [`system`] - System information, shell control, process management
//! - [`network`] - Network configuration and diagnostics
//! - [`hardware`] - Hardware discovery (PCI, USB, block devices)
//! - [`security`] - Security subsystem commands
//! - [`crypto`] - Cryptographic hash commands
//! - [`desktop`] - Desktop/GUI and audio commands
//! - [`package`] - Package management

#![allow(unused_variables, unused_assignments)]

mod crypto;
mod desktop;
mod filesystem;
mod hardware;
mod network;
mod package;
mod security;
mod system;

// Re-export all command structs so that `mod.rs` (the shell module) can
// continue to import them via `commands::FooCommand`.
// ============================================================================
// Shared Helper Functions
// ============================================================================
use alloc::{format, string::String, vec::Vec};

pub(super) use crypto::*;
pub(super) use desktop::*;
pub(super) use filesystem::*;
pub(super) use hardware::*;
pub(super) use network::*;
pub(super) use package::*;
pub(super) use security::*;
pub(super) use system::*;

/// Read a file from VFS and return its contents as a String.
/// Uses a 4096-byte buffer with offset-based reading to handle larger files.
pub(super) fn read_file_to_string(path: &str) -> Result<String, String> {
    match crate::fs::get_vfs().read().resolve_path(path) {
        Ok(node) => {
            let mut result = Vec::new();
            let mut buffer = [0u8; 4096];
            let mut offset = 0;

            loop {
                match node.read(offset, &mut buffer) {
                    Ok(0) => break,
                    Ok(bytes_read) => {
                        result.extend_from_slice(&buffer[..bytes_read]);
                        offset += bytes_read;
                    }
                    Err(e) => return Err(format!("{}", e)),
                }
            }

            match core::str::from_utf8(&result) {
                Ok(s) => Ok(String::from(s)),
                Err(_) => Err(String::from("binary file (not UTF-8)")),
            }
        }
        Err(e) => Err(format!("{}", e)),
    }
}

/// Check if a year is a leap year
pub(super) fn is_leap_year(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Evaluate a test expression and return true/false
pub(super) fn evaluate_test(args: &[String]) -> bool {
    match args.len() {
        0 => false,
        1 => !args[0].is_empty(),
        2 => match args[0].as_str() {
            "-z" => args[1].is_empty(),
            "-n" => !args[1].is_empty(),
            "-f" => crate::fs::file_exists(&args[1]),
            "-d" => {
                // Check if path is a directory
                match crate::fs::get_vfs().read().resolve_path(&args[1]) {
                    Ok(node) => match node.metadata() {
                        Ok(meta) => meta.node_type == crate::fs::NodeType::Directory,
                        Err(_) => false,
                    },
                    Err(_) => false,
                }
            }
            "!" => !evaluate_test(&args[1..]),
            _ => false,
        },
        3 => match args[1].as_str() {
            "=" | "==" => args[0] == args[2],
            "!=" => args[0] != args[2],
            "-eq" => parse_i64(&args[0]) == parse_i64(&args[2]),
            "-ne" => parse_i64(&args[0]) != parse_i64(&args[2]),
            "-lt" => parse_i64(&args[0]) < parse_i64(&args[2]),
            "-gt" => parse_i64(&args[0]) > parse_i64(&args[2]),
            "-le" => parse_i64(&args[0]) <= parse_i64(&args[2]),
            "-ge" => parse_i64(&args[0]) >= parse_i64(&args[2]),
            _ => false,
        },
        _ => false,
    }
}

/// Parse a string as i64, defaulting to 0 on failure
pub(super) fn parse_i64(s: &str) -> i64 {
    s.parse::<i64>().unwrap_or(0)
}

/// Parse a simple IPv6 address string (colon-hex notation).
///
/// Supports full notation (8 groups) and compressed :: notation.
pub(super) fn parse_ipv6_address(s: &str) -> Option<crate::net::Ipv6Address> {
    // Handle special cases
    if s == "::1" {
        return Some(crate::net::Ipv6Address::LOCALHOST);
    }
    if s == "::" {
        return Some(crate::net::Ipv6Address::UNSPECIFIED);
    }

    // Split on ':'
    let parts: Vec<&str> = s.split(':').collect();

    // Check for :: (double colon) indicating compressed zeros
    let has_double_colon = s.contains("::");

    if has_double_colon {
        // Find the position of :: and expand it
        let mut groups: Vec<u16> = Vec::new();
        let mut found_gap = false;
        let mut gap_pos = 0;

        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                if !found_gap && (i == 0 || (i > 0 && parts[i.wrapping_sub(1)].is_empty())) {
                    if !found_gap {
                        found_gap = true;
                        gap_pos = groups.len();
                    }
                    continue;
                }
                if !found_gap {
                    found_gap = true;
                    gap_pos = groups.len();
                }
                continue;
            }
            let val = u16::from_str_radix(part, 16).ok()?;
            groups.push(val);
        }

        // Fill in zeros for the :: gap
        let zeros_needed = 8usize.checked_sub(groups.len())?;
        let mut result = [0u8; 16];
        let mut idx = 0;

        for group in &groups[..gap_pos] {
            let bytes = group.to_be_bytes();
            result[idx] = bytes[0];
            result[idx + 1] = bytes[1];
            idx += 2;
        }

        idx += zeros_needed * 2;

        for group in &groups[gap_pos..] {
            let bytes = group.to_be_bytes();
            result[idx] = bytes[0];
            result[idx + 1] = bytes[1];
            idx += 2;
        }

        Some(crate::net::Ipv6Address(result))
    } else {
        // Full notation: exactly 8 groups
        if parts.len() != 8 {
            return None;
        }

        let mut result = [0u8; 16];
        for (i, part) in parts.iter().enumerate() {
            let val = u16::from_str_radix(part, 16).ok()?;
            let bytes = val.to_be_bytes();
            result[i * 2] = bytes[0];
            result[i * 2 + 1] = bytes[1];
        }

        Some(crate::net::Ipv6Address(result))
    }
}

/// Translate PCI class code to human-readable name
pub(super) fn pci_class_name(class: u8) -> &'static str {
    match class {
        0x01 => "Storage controller",
        0x02 => "Network controller",
        0x03 => "Display controller",
        0x04 => "Multimedia controller",
        0x06 => "Bridge device",
        0x08 => "System peripheral",
        0x0C => "Serial bus controller",
        _ => "Unknown device",
    }
}

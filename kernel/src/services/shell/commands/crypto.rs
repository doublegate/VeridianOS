//! Cryptographic hash commands.

#![allow(unused_variables, unused_assignments)]

use alloc::string::String;

use crate::services::shell::{BuiltinCommand, CommandResult, Shell};

pub(in crate::services::shell) struct Sha256sumCommand;
impl BuiltinCommand for Sha256sumCommand {
    fn name(&self) -> &str {
        "sha256sum"
    }
    fn description(&self) -> &str {
        "Compute SHA-256 hash of a file"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: sha256sum <file>");
            return CommandResult::Error(String::from("missing filename"));
        }
        for filename in args {
            match crate::fs::read_file(filename) {
                Ok(data) => {
                    let hash = crate::crypto::hash::sha256(&data);
                    for byte in &hash.0 {
                        crate::print!("{:02x}", byte);
                    }
                    crate::println!("  {}", filename);
                }
                Err(e) => {
                    crate::println!("sha256sum: {}: {:?}", filename, e);
                }
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct Blake3sumCommand;
impl BuiltinCommand for Blake3sumCommand {
    fn name(&self) -> &str {
        "blake3sum"
    }
    fn description(&self) -> &str {
        "Compute BLAKE3 hash of a file"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: blake3sum <file>");
            return CommandResult::Error(String::from("missing filename"));
        }
        for filename in args {
            match crate::fs::read_file(filename) {
                Ok(data) => {
                    let hash = crate::crypto::hash::blake3(&data);
                    for byte in &hash.0 {
                        crate::print!("{:02x}", byte);
                    }
                    crate::println!("  {}", filename);
                }
                Err(e) => {
                    crate::println!("blake3sum: {}: {:?}", filename, e);
                }
            }
        }
        CommandResult::Success(0)
    }
}

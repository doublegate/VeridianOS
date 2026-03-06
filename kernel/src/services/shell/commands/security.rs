//! Security subsystem commands (capabilities, MAC, audit, TPM).

#![allow(unused_variables, unused_assignments)]

use alloc::string::String;

use crate::services::shell::{BuiltinCommand, CommandResult, Shell};

pub(in crate::services::shell) struct CapCommand;
impl BuiltinCommand for CapCommand {
    fn name(&self) -> &str {
        "cap"
    }
    fn description(&self) -> &str {
        "Capability system statistics"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let sub = args.first().map(|s| s.as_str()).unwrap_or("stats");
        match sub {
            "stats" | "" => {
                let stats = crate::cap::manager::cap_manager().stats();
                crate::println!("=== Capability Statistics ===");
                crate::println!(
                    "Created:    {}",
                    stats
                        .capabilities_created
                        .load(core::sync::atomic::Ordering::Relaxed)
                );
                crate::println!(
                    "Delegated:  {}",
                    stats
                        .capabilities_delegated
                        .load(core::sync::atomic::Ordering::Relaxed)
                );
                crate::println!(
                    "Revoked:    {}",
                    stats
                        .capabilities_revoked
                        .load(core::sync::atomic::Ordering::Relaxed)
                );
                crate::println!(
                    "Deleted:    {}",
                    stats
                        .capabilities_deleted
                        .load(core::sync::atomic::Ordering::Relaxed)
                );
            }
            _ => {
                crate::println!("cap: unknown subcommand '{}'. Use: stats", sub);
                return CommandResult::Error(String::from("unknown subcommand"));
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct MacCommand;
impl BuiltinCommand for MacCommand {
    fn name(&self) -> &str {
        "mac"
    }
    fn description(&self) -> &str {
        "Mandatory Access Control status"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let sub = args.first().map(|s| s.as_str()).unwrap_or("status");
        match sub {
            "status" | "" => {
                let enabled = crate::security::mac::is_enabled();
                let count = crate::security::mac::get_policy_count();
                crate::println!("=== MAC Status ===");
                crate::println!(
                    "Mode:       {}",
                    if enabled { "enforcing" } else { "permissive" }
                );
                crate::println!("Rules:      {}", count);
            }
            "test" => {
                if args.len() < 4 {
                    crate::println!(
                        "Usage: mac test <source_type> <target_type> <read|write|execute>"
                    );
                    return CommandResult::Error(String::from("missing arguments"));
                }
                let access = match args[3].as_str() {
                    "read" => crate::security::AccessType::Read,
                    "write" => crate::security::AccessType::Write,
                    "execute" => crate::security::AccessType::Execute,
                    _ => {
                        crate::println!("mac: invalid access type '{}'", args[3]);
                        return CommandResult::Error(String::from("invalid access type"));
                    }
                };
                let allowed = crate::security::mac::check_access(&args[1], &args[2], access);
                crate::println!(
                    "{} -> {} ({}): {}",
                    args[1],
                    args[2],
                    args[3],
                    if allowed { "ALLOWED" } else { "DENIED" }
                );
            }
            _ => {
                crate::println!("mac: unknown subcommand '{}'. Use: status, test", sub);
                return CommandResult::Error(String::from("unknown subcommand"));
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct AuditCommand;
impl BuiltinCommand for AuditCommand {
    fn name(&self) -> &str {
        "audit"
    }
    fn description(&self) -> &str {
        "Security audit status"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let sub = args.first().map(|s| s.as_str()).unwrap_or("status");
        match sub {
            "status" | "" => {
                let stats = crate::security::audit::get_detailed_stats();
                crate::println!("=== Audit Status ===");
                crate::println!("Total events:     {}", stats.total_events);
                crate::println!("Filtered:         {}", stats.filtered_events);
                crate::println!("Persisted:        {}", stats.persisted_events);
                crate::println!("Alerts triggered: {}", stats.alerts_triggered);
                crate::println!(
                    "Buffer:           {}/{}",
                    stats.buffer_count,
                    stats.buffer_capacity
                );
            }
            "enable" => {
                crate::security::audit::enable();
                crate::println!("Audit logging enabled");
            }
            "disable" => {
                crate::security::audit::disable();
                crate::println!("Audit logging disabled");
            }
            _ => {
                crate::println!(
                    "audit: unknown subcommand '{}'. Use: status, enable, disable",
                    sub
                );
                return CommandResult::Error(String::from("unknown subcommand"));
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct TpmCommand;
impl BuiltinCommand for TpmCommand {
    fn name(&self) -> &str {
        "tpm"
    }
    fn description(&self) -> &str {
        "TPM device status"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let sub = args.first().map(|s| s.as_str()).unwrap_or("status");
        match sub {
            "status" | "" => {
                let available = crate::security::tpm::is_available();
                crate::println!("=== TPM Status ===");
                crate::println!("Available:   {}", if available { "yes" } else { "no" });
                if available {
                    crate::security::tpm::with_tpm(|tpm| {
                        crate::println!(
                            "Mode:        {}",
                            if tpm.is_software_emulation() {
                                "emulated"
                            } else {
                                "hardware"
                            }
                        );
                        crate::println!(
                            "Initialized: {}",
                            if tpm.is_initialized() { "yes" } else { "no" }
                        );
                    });
                }
            }
            "pcr" => {
                if args.len() < 2 {
                    crate::println!("Usage: tpm pcr <index>");
                    return CommandResult::Error(String::from("missing PCR index"));
                }
                let index: u8 = match args[1].parse() {
                    Ok(v) if v < 24 => v,
                    _ => {
                        crate::println!("tpm: invalid PCR index (0-23)");
                        return CommandResult::Error(String::from("invalid PCR index"));
                    }
                };
                match crate::security::tpm::pcr_read(index) {
                    Ok(value) => {
                        crate::print!("PCR[{}]: ", index);
                        for byte in &value {
                            crate::print!("{:02x}", byte);
                        }
                        crate::println!();
                    }
                    Err(e) => {
                        crate::println!("tpm: failed to read PCR[{}]: {:?}", index, e);
                    }
                }
            }
            _ => {
                crate::println!("tpm: unknown subcommand '{}'. Use: status, pcr", sub);
                return CommandResult::Error(String::from("unknown subcommand"));
            }
        }
        CommandResult::Success(0)
    }
}

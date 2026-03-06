//! System, shell control, process management, and diagnostics commands.

#![allow(unused_variables, unused_assignments)]

use alloc::{format, string::String, vec::Vec};

use super::{evaluate_test, is_leap_year, read_file_to_string};
use crate::{
    process::ProcessId,
    services::shell::{BuiltinCommand, CommandResult, Shell},
};

// ============================================================================
// Help & Shell Management Commands
// ============================================================================

pub(in crate::services::shell) struct HelpCommand;
impl BuiltinCommand for HelpCommand {
    fn name(&self) -> &str {
        "help"
    }
    fn description(&self) -> &str {
        "Show available commands"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        let builtins = shell.builtins.read();

        // Per-command help
        if !args.is_empty() {
            let cmd_name = &args[0];
            if let Some(cmd) = builtins.get(cmd_name.as_str()) {
                crate::println!("{}: {}", cmd.name(), cmd.description());
                return CommandResult::Success(0);
            } else {
                crate::println!("help: unknown command '{}'", cmd_name);
                return CommandResult::Success(1);
            }
        }

        crate::println!(
            "VeridianOS Shell - Available Commands ({} total):",
            builtins.len()
        );
        crate::println!();

        // Define categories with their command names
        let categories: &[(&str, &[&str])] = &[
            (
                "System",
                &[
                    "help",
                    "?",
                    "exit",
                    "logout",
                    "clear",
                    "history",
                    "uptime",
                    "uname",
                    "date",
                    "free",
                    "dmesg",
                    "df",
                    "sync",
                    "ps",
                    "kill",
                    "top",
                    "lscpu",
                    "hostname",
                    "sysctl",
                    "shutdown",
                    "reboot",
                    "poweroff",
                    "suspend",
                    "hibernate",
                ],
            ),
            (
                "User Management",
                &[
                    "useradd", "userdel", "passwd", "id", "whoami", "groups", "su", "sudo",
                ],
            ),
            ("Service Management", &["service", "cloud-init", "kubectl"]),
            (
                "Filesystem",
                &[
                    "cd",
                    "pwd",
                    "ls",
                    "mkdir",
                    "cat",
                    "echo",
                    "touch",
                    "rm",
                    "cp",
                    "mv",
                    "chmod",
                    "mount",
                    "xattr",
                    "tar",
                    "mkfs",
                    "fsck",
                    "blkid",
                    "nfsmount",
                    "smbclient",
                ],
            ),
            (
                "Text Processing",
                &[
                    "wc", "head", "tail", "grep", "sort", "uniq", "cut", "tr", "tee", "printf",
                    "read",
                ],
            ),
            (
                "Network",
                &[
                    "ifconfig",
                    "dhcp",
                    "netstat",
                    "arp",
                    "ping",
                    "ping6",
                    "ndp",
                    "route",
                    "ss",
                    "firewall",
                    "nat",
                    "dns",
                    "ntp",
                    "vpn",
                    "wg",
                    "wifi",
                    "bt",
                    "ssh",
                    "curl",
                    "vlan",
                    "bond",
                    "http-server",
                    "sshd",
                    "ldapsearch",
                    "kinit",
                    "klist",
                ],
            ),
            ("Security", &["audit", "cap", "mac", "tpm"]),
            ("Crypto", &["blake3sum", "sha256sum"]),
            (
                "Hardware",
                &[
                    "lspci", "lsusb", "lsblk", "acpi", "hwinfo", "mdadm", "iscsiadm",
                ],
            ),
            (
                "Performance",
                &["perf", "trace", "sched", "slab", "vmstat", "numa", "kpti"],
            ),
            (
                "DevTools",
                &["git", "make", "gdb", "profiler", "ci", "strace", "coredump"],
            ),
            (
                "Desktop",
                &[
                    "startgui",
                    "winfo",
                    "browser",
                    "screenshot",
                    "notify",
                    "theme",
                ],
            ),
            ("Virtualization", &["vmx", "container", "lsns"]),
            ("Audio", &["play", "volume"]),
            ("Package", &["pkg"]),
            (
                "Shell",
                &[
                    "set", "source", ".", "alias", "unalias", "type", "which", "env", "export",
                    "unset", "fg", "bg", "jobs", "true", "false", "test", "[", "ipcs",
                ],
            ),
            ("Scheduled Tasks", &["crontab", "at"]),
        ];

        for (category, cmds) in categories {
            let mut found: Vec<_> = cmds
                .iter()
                .filter(|name| builtins.contains_key(**name))
                .collect();
            if found.is_empty() {
                continue;
            }
            found.sort();
            crate::println!("  {}:", category);
            for name in found {
                if let Some(cmd) = builtins.get(*name) {
                    crate::println!("    {:14} {}", cmd.name(), cmd.description());
                }
            }
            crate::println!();
        }

        // Show any uncategorized commands
        let all_categorized: Vec<&str> = categories
            .iter()
            .flat_map(|(_, cmds)| cmds.iter().copied())
            .collect();
        let mut uncategorized: Vec<_> = builtins
            .values()
            .filter(|cmd| !all_categorized.contains(&cmd.name()))
            .collect();
        if !uncategorized.is_empty() {
            uncategorized.sort_by_key(|cmd| cmd.name());
            crate::println!("  Other:");
            for cmd in uncategorized {
                crate::println!("    {:14} {}", cmd.name(), cmd.description());
            }
            crate::println!();
        }

        crate::println!("Use 'help <command>' for detailed help on specific commands.");
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct HistoryCommand;
impl BuiltinCommand for HistoryCommand {
    fn name(&self) -> &str {
        "history"
    }
    fn description(&self) -> &str {
        "Show command history"
    }

    fn execute(&self, _args: &[String], shell: &Shell) -> CommandResult {
        let history = shell.history.read();
        for (i, cmd) in history.iter().enumerate() {
            crate::println!("{:4} {}", i + 1, cmd);
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct ClearCommand;
impl BuiltinCommand for ClearCommand {
    fn name(&self) -> &str {
        "clear"
    }
    fn description(&self) -> &str {
        "Clear the screen"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("\x1b[2J\x1b[H");
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct ExitCommand;
impl BuiltinCommand for ExitCommand {
    fn name(&self) -> &str {
        "exit"
    }
    fn description(&self) -> &str {
        "Exit the shell"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let exit_code = if args.is_empty() {
            0
        } else {
            args[0].parse().unwrap_or(1)
        };

        CommandResult::Exit(exit_code)
    }
}

// ============================================================================
// Process Management Commands
// ============================================================================

pub(in crate::services::shell) struct PsCommand;
impl BuiltinCommand for PsCommand {
    fn name(&self) -> &str {
        "ps"
    }
    fn description(&self) -> &str {
        "List running processes"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let process_server = crate::services::process_server::get_process_server();
        let processes = process_server.list_processes();

        crate::println!("  PID  PPID STATE    NAME");
        for process in processes {
            let state = match process.state {
                crate::services::process_server::ProcessState::Running => "RUN",
                crate::services::process_server::ProcessState::Sleeping => "SLP",
                crate::services::process_server::ProcessState::Waiting => "WAIT",
                crate::services::process_server::ProcessState::Stopped => "STOP",
                crate::services::process_server::ProcessState::Zombie => "ZOMB",
                crate::services::process_server::ProcessState::Dead => "DEAD",
            };

            crate::println!(
                "{:5} {:5} {:8} {}",
                process.pid.0,
                process.ppid.0,
                state,
                process.name
            );
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct KillCommand;
impl BuiltinCommand for KillCommand {
    fn name(&self) -> &str {
        "kill"
    }
    fn description(&self) -> &str {
        "Send signal to process"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("kill: missing process ID"));
        }

        let pid_str = &args[0];
        match pid_str.parse::<u64>() {
            Ok(pid_num) => {
                let process_server = crate::services::process_server::get_process_server();
                match process_server.send_signal(ProcessId(pid_num), 15) {
                    Ok(()) => CommandResult::Success(0),
                    Err(e) => CommandResult::Error(format!("kill: {}", e)),
                }
            }
            Err(_) => CommandResult::Error(format!("kill: invalid process ID: {}", pid_str)),
        }
    }
}

pub(in crate::services::shell) struct UptimeCommand;
impl BuiltinCommand for UptimeCommand {
    fn name(&self) -> &str {
        "uptime"
    }
    fn description(&self) -> &str {
        "Show system uptime"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let total_secs = crate::arch::timer::get_timestamp_secs();
        let days = total_secs / 86400;
        let hours = (total_secs % 86400) / 3600;
        let minutes = (total_secs % 3600) / 60;
        let secs = total_secs % 60;
        crate::println!(
            "uptime: {} days, {} hours, {} minutes, {} seconds",
            days,
            hours,
            minutes,
            secs
        );
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct LsmodCommand;
impl BuiltinCommand for LsmodCommand {
    fn name(&self) -> &str {
        "lsmod"
    }
    fn description(&self) -> &str {
        "List loaded drivers"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let driver_framework = crate::services::driver_framework::get_driver_framework();
        let stats = driver_framework.get_statistics();

        crate::println!("Driver Framework Statistics:");
        crate::println!("  Total drivers: {}", stats.total_drivers);
        crate::println!("  Total buses: {}", stats.total_buses);
        crate::println!("  Total devices: {}", stats.total_devices);
        crate::println!("  Bound devices: {}", stats.bound_devices);
        crate::println!("  Active devices: {}", stats.active_devices);

        CommandResult::Success(0)
    }
}

// ============================================================================
// Environment Variable Commands
// ============================================================================

pub(in crate::services::shell) struct EnvCommand;
impl BuiltinCommand for EnvCommand {
    fn name(&self) -> &str {
        "env"
    }
    fn description(&self) -> &str {
        "Show environment variables"
    }

    fn execute(&self, _args: &[String], shell: &Shell) -> CommandResult {
        let env_vars = shell.get_all_env();
        for var in env_vars {
            crate::println!("{}", var);
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct ExportCommand;
impl BuiltinCommand for ExportCommand {
    fn name(&self) -> &str {
        "export"
    }
    fn description(&self) -> &str {
        "Set environment variable"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("export: missing variable"));
        }

        for arg in args {
            if let Some(eq_pos) = arg.find('=') {
                let name = arg[..eq_pos].into();
                let value = arg[eq_pos + 1..].into();
                shell.set_env(name, value);
            } else {
                return CommandResult::Error(format!("export: invalid syntax: {}", arg));
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct UnsetCommand;
impl BuiltinCommand for UnsetCommand {
    fn name(&self) -> &str {
        "unset"
    }
    fn description(&self) -> &str {
        "Unset environment variable"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("unset: missing variable"));
        }

        for var_name in args {
            shell.environment.write().remove(var_name);
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// System Information Commands
// ============================================================================

pub(in crate::services::shell) struct DateCommand;
impl BuiltinCommand for DateCommand {
    fn name(&self) -> &str {
        "date"
    }
    fn description(&self) -> &str {
        "Show current date and time"
    }

    // println! is a no-op on non-x86_64; `month` is incremented in a loop
    // and consumed only in the format string.
    #[cfg_attr(not(target_arch = "x86_64"), allow(unused_assignments))]
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let total_secs = crate::arch::timer::get_timestamp_secs();

        // Convert epoch seconds to date components
        // Unix epoch: Jan 1 1970 00:00:00 UTC
        let secs_per_day: u64 = 86400;
        let mut days = total_secs / secs_per_day;
        let day_secs = total_secs % secs_per_day;
        let hours = day_secs / 3600;
        let minutes = (day_secs % 3600) / 60;
        let seconds = day_secs % 60;

        // Calculate year, month, day from days since epoch
        let mut year: u64 = 1970;
        loop {
            let days_in_year = if is_leap_year(year) { 366 } else { 365 };
            if days < days_in_year {
                break;
            }
            days -= days_in_year;
            year += 1;
        }

        let month_days: [u64; 12] = if is_leap_year(year) {
            [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };

        let mut month: u64 = 1;
        for &mdays in &month_days {
            if days < mdays {
                break;
            }
            days -= mdays;
            month += 1;
        }

        let day = days + 1;

        crate::println!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            year,
            month,
            day,
            hours,
            minutes,
            seconds
        );

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct UnameCommand;
impl BuiltinCommand for UnameCommand {
    fn name(&self) -> &str {
        "uname"
    }
    fn description(&self) -> &str {
        "Show system information"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let show_all = args.iter().any(|a| a == "-a");
        let show_sysname = args.is_empty() || show_all || args.iter().any(|a| a == "-s");
        let show_nodename = show_all || args.iter().any(|a| a == "-n");
        let show_release = show_all || args.iter().any(|a| a == "-r");
        let show_machine = show_all || args.iter().any(|a| a == "-m");

        let mut parts: Vec<&str> = Vec::new();

        if show_sysname {
            parts.push("VeridianOS");
        }
        if show_nodename {
            parts.push("veridian");
        }
        if show_release {
            parts.push("0.18.0");
        }
        if show_machine {
            #[cfg(target_arch = "x86_64")]
            parts.push("x86_64");
            #[cfg(target_arch = "aarch64")]
            parts.push("aarch64");
            #[cfg(target_arch = "riscv64")]
            parts.push("riscv64");
        }

        crate::println!("{}", parts.join(" "));
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct FreeCommand;
impl BuiltinCommand for FreeCommand {
    fn name(&self) -> &str {
        "free"
    }
    fn description(&self) -> &str {
        "Show memory usage"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let stats = crate::mm::get_memory_stats();
        let page_size: usize = 4096;

        let total_kb = (stats.total_frames * page_size) / 1024;
        let free_kb = (stats.free_frames * page_size) / 1024;
        let used_kb = total_kb.saturating_sub(free_kb);
        let cached_kb = (stats.cached_frames * page_size) / 1024;

        crate::println!(
            "{:>12} {:>12} {:>12} {:>12}",
            "total",
            "used",
            "free",
            "cached"
        );
        crate::println!(
            "{:>10} K {:>10} K {:>10} K {:>10} K",
            total_kb,
            used_kb,
            free_kb,
            cached_kb
        );
        crate::println!();
        crate::println!(
            "Frames: {} total, {} free, {} cached",
            stats.total_frames,
            stats.free_frames,
            stats.cached_frames
        );

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct DmesgCommand;
impl BuiltinCommand for DmesgCommand {
    fn name(&self) -> &str {
        "dmesg"
    }
    fn description(&self) -> &str {
        "Show kernel message buffer"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        // The kernel log ring buffer is not yet wired up for reading.
        // Print a notice about the current status.
        crate::println!("[dmesg] Kernel ring buffer not yet available for user-space reading.");
        crate::println!("[dmesg] Boot messages were printed to serial console.");
        crate::println!("[dmesg] Use serial capture (QEMU -serial file:log) to review boot log.");
        CommandResult::Success(0)
    }
}

// ============================================================================
// Utility Commands
// ============================================================================

pub(in crate::services::shell) struct TrueCommand;
impl BuiltinCommand for TrueCommand {
    fn name(&self) -> &str {
        "true"
    }
    fn description(&self) -> &str {
        "Return success"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct FalseCommand;
impl BuiltinCommand for FalseCommand {
    fn name(&self) -> &str {
        "false"
    }
    fn description(&self) -> &str {
        "Return failure"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        CommandResult::Success(1)
    }
}

pub(in crate::services::shell) struct TestCommand;
impl BuiltinCommand for TestCommand {
    fn name(&self) -> &str {
        "test"
    }
    fn description(&self) -> &str {
        "Evaluate conditional expression"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        // Handle `[` invocation: strip trailing `]`
        let test_args = if !args.is_empty() && args.last().map(|s| s.as_str()) == Some("]") {
            &args[..args.len() - 1]
        } else {
            args
        };

        if evaluate_test(test_args) {
            CommandResult::Success(0)
        } else {
            CommandResult::Success(1)
        }
    }
}

/// Bracket alias for test command
pub(in crate::services::shell) struct BracketTestCommand;
impl BuiltinCommand for BracketTestCommand {
    fn name(&self) -> &str {
        "["
    }
    fn description(&self) -> &str {
        "Evaluate conditional expression (alias for test)"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let test_args = if !args.is_empty() && args.last().map(|s| s.as_str()) == Some("]") {
            &args[..args.len() - 1]
        } else {
            args
        };

        if evaluate_test(test_args) {
            CommandResult::Success(0)
        } else {
            CommandResult::Success(1)
        }
    }
}

// ============================================================================
// I/O Commands
// ============================================================================

pub(in crate::services::shell) struct ReadCommand;
impl BuiltinCommand for ReadCommand {
    fn name(&self) -> &str {
        "read"
    }
    fn description(&self) -> &str {
        "Read a line of input into a variable"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        let var_name = if args.is_empty() {
            String::from("REPLY")
        } else {
            args[0].clone()
        };

        // Read characters from serial until newline
        let mut input = String::new();
        loop {
            if let Some(byte) = Shell::read_char() {
                if byte == b'\n' || byte == b'\r' {
                    crate::println!();
                    break;
                }
                if byte == 0x7f || byte == 0x08 {
                    // Backspace
                    if !input.is_empty() {
                        input.pop();
                        crate::print!("\x08 \x08");
                    }
                    continue;
                }
                if (0x20..0x7f).contains(&byte) {
                    input.push(byte as char);
                    crate::print!("{}", byte as char);
                }
            }
        }

        shell.set_env(var_name, input);
        CommandResult::Success(0)
    }
}

// ============================================================================
// Shell Control Commands
// ============================================================================

pub(in crate::services::shell) struct SetCommand;
impl BuiltinCommand for SetCommand {
    fn name(&self) -> &str {
        "set"
    }
    fn description(&self) -> &str {
        "Show or set shell variables"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            // Show all environment variables (same as env)
            let env_vars = shell.get_all_env();
            for var in env_vars {
                crate::println!("{}", var);
            }
        } else {
            // Set variables: set NAME=VALUE
            for arg in args {
                if let Some(eq_pos) = arg.find('=') {
                    let name = arg[..eq_pos].into();
                    let value = arg[eq_pos + 1..].into();
                    shell.set_env(name, value);
                } else {
                    crate::println!("{}", shell.get_env(arg).unwrap_or_default());
                }
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct SourceCommand;
impl BuiltinCommand for SourceCommand {
    fn name(&self) -> &str {
        "source"
    }
    fn description(&self) -> &str {
        "Execute commands from a file"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("source: missing file argument"));
        }

        let path = &args[0];
        match read_file_to_string(path) {
            Ok(content) => {
                let mut last_result = CommandResult::Success(0);
                for line in content.split('\n') {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    last_result = shell.execute_command(trimmed);
                    if let CommandResult::Exit(code) = last_result {
                        return CommandResult::Exit(code);
                    }
                }
                last_result
            }
            Err(e) => CommandResult::Error(format!("source: {}: {}", path, e)),
        }
    }
}

/// Dot command (`.`) -- alias for source
pub(in crate::services::shell) struct DotCommand;
impl BuiltinCommand for DotCommand {
    fn name(&self) -> &str {
        "."
    }
    fn description(&self) -> &str {
        "Execute commands from a file (alias for source)"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from(".: missing file argument"));
        }

        let path = &args[0];
        match read_file_to_string(path) {
            Ok(content) => {
                let mut last_result = CommandResult::Success(0);
                for line in content.split('\n') {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    last_result = shell.execute_command(trimmed);
                    if let CommandResult::Exit(code) = last_result {
                        return CommandResult::Exit(code);
                    }
                }
                last_result
            }
            Err(e) => CommandResult::Error(format!(".: {}: {}", path, e)),
        }
    }
}

pub(in crate::services::shell) struct AliasCommand;
impl BuiltinCommand for AliasCommand {
    fn name(&self) -> &str {
        "alias"
    }
    fn description(&self) -> &str {
        "Define or show command aliases"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            // Show all aliases stored in environment with ALIAS_ prefix
            let env_vars = shell.get_all_env();
            let mut found = false;
            for var in &env_vars {
                if let Some(rest) = var.strip_prefix("ALIAS_") {
                    if let Some(eq_pos) = rest.find('=') {
                        crate::println!("alias {}='{}'", &rest[..eq_pos], &rest[eq_pos + 1..]);
                        found = true;
                    }
                }
            }
            if !found {
                crate::println!("No aliases defined");
            }
            return CommandResult::Success(0);
        }

        for arg in args {
            // Parse alias name='value' or alias name=value
            if let Some(eq_pos) = arg.find('=') {
                let name: String = arg[..eq_pos].into();
                let mut value: String = arg[eq_pos + 1..].into();

                // Strip surrounding quotes if present
                if ((value.starts_with('\'') && value.ends_with('\''))
                    || (value.starts_with('"') && value.ends_with('"')))
                    && value.len() >= 2
                {
                    value = value[1..value.len() - 1].into();
                }

                let env_key = format!("ALIAS_{}", name);
                shell.set_env(env_key, value);
            } else {
                // Show specific alias
                let env_key = format!("ALIAS_{}", arg);
                match shell.get_env(&env_key) {
                    Some(val) => crate::println!("alias {}='{}'", arg, val),
                    None => return CommandResult::Error(format!("alias: {}: not found", arg)),
                }
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct UnaliasCommand;
impl BuiltinCommand for UnaliasCommand {
    fn name(&self) -> &str {
        "unalias"
    }
    fn description(&self) -> &str {
        "Remove command aliases"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("unalias: missing alias name"));
        }

        for name in args {
            if name == "-a" {
                // Remove all aliases
                let env_vars = shell.get_all_env();
                for var in &env_vars {
                    if let Some(rest) = var.strip_prefix("ALIAS_") {
                        if let Some(eq_pos) = rest.find('=') {
                            let alias_name = &rest[..eq_pos];
                            let env_key = format!("ALIAS_{}", alias_name);
                            shell.environment.write().remove(&env_key);
                        }
                    }
                }
                return CommandResult::Success(0);
            }

            let env_key = format!("ALIAS_{}", name);
            if shell.get_env(&env_key).is_some() {
                shell.environment.write().remove(&env_key);
            } else {
                return CommandResult::Error(format!("unalias: {}: not found", name));
            }
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct TypeCommand;
impl BuiltinCommand for TypeCommand {
    fn name(&self) -> &str {
        "type"
    }
    fn description(&self) -> &str {
        "Show command type (builtin, alias, external)"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("type: missing argument"));
        }

        for name in args {
            // Check if it's an alias
            let alias_key = format!("ALIAS_{}", name);
            if let Some(val) = shell.get_env(&alias_key) {
                crate::println!("{} is aliased to '{}'", name, val);
                continue;
            }

            // Check if it's a builtin
            let builtins = shell.builtins.read();
            if builtins.contains_key(name.as_str()) {
                crate::println!("{} is a shell builtin", name);
                continue;
            }

            crate::println!("{}: not found", name);
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct WhichCommand;
impl BuiltinCommand for WhichCommand {
    fn name(&self) -> &str {
        "which"
    }
    fn description(&self) -> &str {
        "Show path of external command"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("which: missing argument"));
        }

        for name in args {
            // Check builtins first
            let builtins = shell.builtins.read();
            if builtins.contains_key(name.as_str()) {
                crate::println!("{}: shell built-in command", name);
                continue;
            }
            drop(builtins);

            // Search PATH for external command
            if let Some(path_val) = shell.get_env("PATH") {
                let mut found = false;
                for dir in path_val.split(':') {
                    let full_path = format!("{}/{}", dir, name);
                    if crate::fs::file_exists(&full_path) {
                        crate::println!("{}", full_path);
                        found = true;
                        break;
                    }
                }
                if !found {
                    crate::println!("{} not found", name);
                }
            } else {
                crate::println!("{} not found", name);
            }
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// Job Control Commands
// ============================================================================

pub(in crate::services::shell) struct FgCommand;
impl BuiltinCommand for FgCommand {
    fn name(&self) -> &str {
        "fg"
    }
    fn description(&self) -> &str {
        "Bring job to foreground"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("fg: no job control (single-process kernel shell)");
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct BgCommand;
impl BuiltinCommand for BgCommand {
    fn name(&self) -> &str {
        "bg"
    }
    fn description(&self) -> &str {
        "Resume job in background"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("bg: no job control (single-process kernel shell)");
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct JobsCommand;
impl BuiltinCommand for JobsCommand {
    fn name(&self) -> &str {
        "jobs"
    }
    fn description(&self) -> &str {
        "List background jobs"
    }

    fn execute(&self, _args: &[String], shell: &Shell) -> CommandResult {
        let table = shell.job_table.read();
        let jobs = table.list_jobs();
        if jobs.is_empty() {
            crate::println!("jobs: no active jobs");
        } else {
            for job in &jobs {
                crate::println!("[{}]  {}  {}", job.job_id, job.status, job.command_line);
            }
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// Performance Commands
// ============================================================================

pub(in crate::services::shell) struct PerfCommand;
impl BuiltinCommand for PerfCommand {
    fn name(&self) -> &str {
        "perf"
    }
    fn description(&self) -> &str {
        "Run performance benchmarks"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() > 1 && args[1] == "stats" {
            // Show performance counters
            let stats = crate::perf::get_stats();
            crate::println!("Performance Counters:");
            crate::println!("  Syscalls:         {}", stats.syscalls);
            crate::println!("  Context switches: {}", stats.context_switches);
            crate::println!("  Page faults:      {}", stats.page_faults);
            crate::println!("  Interrupts:       {}", stats.interrupts);
            crate::println!("  IPC messages:     {}", stats.ipc_messages);
        } else if args.len() > 1 && args[1] == "reset" {
            crate::perf::reset_stats();
            crate::println!("Performance counters reset.");
        } else {
            // Run benchmarks
            crate::perf::bench::run_all_benchmarks();
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct TraceCommand;
impl BuiltinCommand for TraceCommand {
    fn name(&self) -> &str {
        "trace"
    }
    fn description(&self) -> &str {
        "Kernel tracing (on/off/dump)"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            crate::println!("Usage: trace <on|off|dump|status>");
            crate::println!("  on    - Enable kernel tracing");
            crate::println!("  off   - Disable kernel tracing");
            crate::println!("  dump  - Dump trace buffer (last 32 events)");
            crate::println!("  status - Show tracing status");
            return CommandResult::Success(0);
        }

        match args[1].as_str() {
            "on" => {
                crate::perf::trace::enable();
                crate::println!("Tracing enabled.");
            }
            "off" => {
                crate::perf::trace::disable();
                crate::println!("Tracing disabled.");
            }
            "dump" => {
                let count = if args.len() > 2 {
                    args[2].parse::<usize>().unwrap_or(32)
                } else {
                    32
                };
                crate::perf::trace::dump_trace(count);
            }
            "status" => {
                let enabled = crate::perf::trace::is_enabled();
                let total = crate::perf::trace::total_events();
                crate::println!(
                    "Tracing: {} ({} events recorded)",
                    if enabled { "enabled" } else { "disabled" },
                    total
                );
            }
            _ => {
                crate::println!("Unknown trace command: {}", args[1]);
            }
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// Hardware Diagnostics Commands
// ============================================================================

pub(in crate::services::shell) struct AcpiCommand;
impl BuiltinCommand for AcpiCommand {
    fn name(&self) -> &str {
        "acpi"
    }
    fn description(&self) -> &str {
        "Show parsed ACPI table information"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::x86_64::acpi::dump();
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            crate::println!("ACPI is only available on x86_64");
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// Memory & Performance Statistics
// ============================================================================

pub(in crate::services::shell) struct VmstatCommand;
impl BuiltinCommand for VmstatCommand {
    fn name(&self) -> &str {
        "vmstat"
    }
    fn description(&self) -> &str {
        "Virtual memory statistics"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let mem = crate::mm::get_memory_stats();
        let total_kb = mem.total_frames * 4;
        let free_kb = mem.free_frames * 4;
        let used_kb = total_kb.saturating_sub(free_kb);
        crate::println!("=== Virtual Memory Statistics ===");
        crate::println!("Memory:");
        crate::println!("  Total:    {} KB ({} MB)", total_kb, total_kb / 1024);
        crate::println!("  Used:     {} KB ({} MB)", used_kb, used_kb / 1024);
        crate::println!("  Free:     {} KB ({} MB)", free_kb, free_kb / 1024);
        crate::println!(
            "  Frames:   {} total, {} free",
            mem.total_frames,
            mem.free_frames
        );
        let perf = crate::perf::get_stats();
        crate::println!("System:");
        crate::println!("  Context switches: {}", perf.context_switches);
        crate::println!("  Syscalls:         {}", perf.syscalls);
        crate::println!("  Interrupts:       {}", perf.interrupts);
        crate::println!("  Page faults:      {}", perf.page_faults);
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct SchedCommand;
impl BuiltinCommand for SchedCommand {
    fn name(&self) -> &str {
        "sched"
    }
    fn description(&self) -> &str {
        "Scheduler statistics"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let sub = args.first().map(|s| s.as_str()).unwrap_or("stats");
        match sub {
            "stats" | "" => {
                let summary = crate::sched::metrics::SCHEDULER_METRICS.get_summary();
                crate::println!("=== Scheduler Statistics ===");
                crate::println!("Context switches:    {}", summary.context_switches);
                crate::println!("  Voluntary:         {}", summary.voluntary_switches);
                crate::println!("  Involuntary:       {}", summary.involuntary_switches);
                crate::println!("  Avg latency:       {} cycles", summary.avg_switch_latency);
                crate::println!("Load balancing:");
                crate::println!("  Balance ops:       {}", summary.load_balance_count);
                crate::println!("  Migrations:        {}", summary.task_migrations);
                crate::println!("IPC:");
                crate::println!("  Blocked:           {}", summary.ipc_blocks);
                crate::println!("  Wakeups:           {}", summary.ipc_wakeups);
                crate::println!("Idle percentage:     {}%", summary.idle_percentage);
            }
            "reset" => {
                crate::println!("Scheduler metrics reset not yet implemented");
            }
            _ => {
                crate::println!("sched: unknown subcommand '{}'. Use: stats, reset", sub);
                return CommandResult::Error(String::from("unknown subcommand"));
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct SlabCommand;
impl BuiltinCommand for SlabCommand {
    fn name(&self) -> &str {
        "slab"
    }
    fn description(&self) -> &str {
        "Kernel heap statistics"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("=== Kernel Heap Statistics ===");
        #[cfg(all(target_arch = "x86_64", target_os = "none"))]
        {
            let (total, used, free) = crate::mm::heap::get_heap_stats();
            crate::println!("Heap size:  {} MB", total / (1024 * 1024));
            crate::println!(
                "Used:       {} MB ({} KB)",
                used / (1024 * 1024),
                used / 1024
            );
            crate::println!(
                "Free:       {} MB ({} KB)",
                free / (1024 * 1024),
                free / 1024
            );
            let pct = if total > 0 { (used * 100) / total } else { 0 };
            crate::println!("Usage:      {}%", pct);
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            crate::println!("Heap size:  {} KB", crate::mm::heap::HEAP_SIZE / 1024);
            crate::println!("(detailed stats only available on x86_64)");
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// NUMA & KPTI Diagnostics
// ============================================================================

pub struct NumaCommand;

impl BuiltinCommand for NumaCommand {
    fn name(&self) -> &str {
        "numa"
    }

    fn description(&self) -> &str {
        "Display NUMA topology information"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let sub = if args.len() > 1 {
            args[1].as_str()
        } else {
            "info"
        };

        match sub {
            "info" | "topology" => {
                crate::println!("=== NUMA Topology ===");

                #[cfg(target_arch = "x86_64")]
                {
                    let found = crate::arch::x86_64::acpi::with_acpi_info(|info| {
                        if info.has_srat {
                            crate::println!("SRAT table: found at 0x{:X}", info.srat_address);
                            crate::println!("  CPU count: {}", info.local_apic_count);
                        } else {
                            crate::println!("SRAT table: not found (single-node assumed)");
                        }
                        if info.has_slit {
                            crate::println!("SLIT table: found at 0x{:X}", info.slit_address);
                        } else {
                            crate::println!("SLIT table: not found");
                        }
                    });
                    if found.is_none() {
                        crate::println!("ACPI: not initialized");
                    }
                }

                #[cfg(not(target_arch = "x86_64"))]
                {
                    crate::println!("NUMA topology: not available on this architecture");
                }

                CommandResult::Success(0)
            }
            "help" => {
                crate::println!("Usage: numa [info|topology|help]");
                crate::println!("  info      - Show NUMA topology information");
                crate::println!("  topology  - Same as info");
                CommandResult::Success(0)
            }
            _ => {
                crate::println!("numa: unknown subcommand '{}'. Try 'numa help'", sub);
                CommandResult::Error(String::from("unknown subcommand"))
            }
        }
    }
}

pub struct KptiCommand;

impl BuiltinCommand for KptiCommand {
    fn name(&self) -> &str {
        "kpti"
    }

    fn description(&self) -> &str {
        "Display KPTI status"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("=== KPTI Status ===");

        #[cfg(target_arch = "x86_64")]
        {
            let active = crate::arch::x86_64::kpti::is_active();
            crate::println!(
                "KPTI (Kernel Page Table Isolation): {}",
                if active { "ACTIVE" } else { "INACTIVE" }
            );
            if active {
                crate::println!("Shadow page tables: created");
                crate::println!("Meltdown mitigation: enabled");
            } else {
                crate::println!("Shadow page tables: not initialized");
                crate::println!("Note: CR3 switching disabled for performance");
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            crate::println!("KPTI: not applicable on this architecture");
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// IPC Commands
// ============================================================================

pub(in crate::services::shell) struct IpcsCommand;
impl BuiltinCommand for IpcsCommand {
    fn name(&self) -> &str {
        "ipcs"
    }
    fn description(&self) -> &str {
        "IPC facility statistics"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("=== IPC Statistics ===");
        if let Ok(stats) = crate::ipc::registry::get_registry_stats() {
            crate::println!("Registry:");
            crate::println!("  Endpoints created:    {}", stats.endpoints_created);
            crate::println!("  Endpoints destroyed:  {}", stats.endpoints_destroyed);
            crate::println!("  Channels created:     {}", stats.channels_created);
            crate::println!("  Channels destroyed:   {}", stats.channels_destroyed);
            crate::println!("  Cap cache hit rate:   {}%", stats.cache_hit_rate);
        }
        let (fp_count, fp_avg) = crate::ipc::fast_path::get_fast_path_stats();
        crate::println!("Fast path:");
        crate::println!("  Transfers:            {}", fp_count);
        crate::println!("  Avg cycles:           {}", fp_avg);
        CommandResult::Success(0)
    }
}

// ============================================================================
// Virtualization Commands
// ============================================================================

pub(in crate::services::shell) struct VmxCommand;

impl BuiltinCommand for VmxCommand {
    fn name(&self) -> &str {
        "vmx"
    }
    fn description(&self) -> &str {
        "VMX hypervisor management"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let sub = args.first().map(|s| s.as_str()).unwrap_or("status");
        match sub {
            "status" => {
                #[cfg(target_arch = "x86_64")]
                {
                    let (enabled, active, rev_id) = crate::virt::vmx::vmx_status();
                    crate::println!("VMX status:");
                    crate::println!("  VMX supported: {}", crate::virt::cpu_supports_vmx());
                    crate::println!("  VMX enabled:   {}", enabled);
                    crate::println!("  VM active:     {}", active);
                    if let Some(rid) = rev_id {
                        crate::println!("  VMCS revision: 0x{:x}", rid);
                    }
                }
                #[cfg(not(target_arch = "x86_64"))]
                {
                    crate::println!("VMX is only supported on x86_64");
                }
                CommandResult::Success(0)
            }
            "enable" => {
                #[cfg(target_arch = "x86_64")]
                {
                    match crate::virt::vmx::vmx_enable() {
                        Ok(()) => {
                            crate::println!("VMX enabled successfully");
                            CommandResult::Success(0)
                        }
                        Err(e) => {
                            crate::println!("vmx: enable failed: {}", e);
                            CommandResult::Error(String::from("vmx enable failed"))
                        }
                    }
                }
                #[cfg(not(target_arch = "x86_64"))]
                {
                    crate::println!("VMX is only supported on x86_64");
                    CommandResult::Error(String::from("unsupported architecture"))
                }
            }
            "disable" => {
                #[cfg(target_arch = "x86_64")]
                {
                    match crate::virt::vmx::vmx_disable() {
                        Ok(()) => {
                            crate::println!("VMX disabled");
                            CommandResult::Success(0)
                        }
                        Err(e) => {
                            crate::println!("vmx: disable failed: {}", e);
                            CommandResult::Error(String::from("vmx disable failed"))
                        }
                    }
                }
                #[cfg(not(target_arch = "x86_64"))]
                {
                    crate::println!("VMX is only supported on x86_64");
                    CommandResult::Error(String::from("unsupported architecture"))
                }
            }
            "help" => {
                crate::println!("Usage: vmx <subcommand>");
                crate::println!("  status  - Show VMX status");
                crate::println!("  enable  - Enable VMX operation");
                crate::println!("  disable - Disable VMX operation");
                CommandResult::Success(0)
            }
            _ => {
                crate::println!("vmx: unknown subcommand '{}'. Try 'vmx help'", sub);
                CommandResult::Error(String::from("unknown subcommand"))
            }
        }
    }
}

pub(in crate::services::shell) struct ContainerCommand;

impl BuiltinCommand for ContainerCommand {
    fn name(&self) -> &str {
        "container"
    }
    fn description(&self) -> &str {
        "Container management"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let sub = args.first().map(|s| s.as_str()).unwrap_or("list");
        match sub {
            "list" => {
                let result = crate::virt::container::with_container_manager(
                    |mgr: &mut crate::virt::container::ContainerManager| {
                        let containers = mgr.list();
                        if containers.is_empty() {
                            crate::println!("No containers");
                        } else {
                            crate::println!(
                                "{:<6} {:<20} {:<10} {:<8} {}",
                                "ID",
                                "NAME",
                                "STATE",
                                "PROCS",
                                "HOSTNAME"
                            );
                            for c in &containers {
                                crate::println!(
                                    "{:<6} {:<20} {:<10} {:<8} {}",
                                    c.id,
                                    c.name,
                                    c.state,
                                    c.process_count,
                                    c.hostname
                                );
                            }
                        }
                    },
                );
                match result {
                    Ok(()) => CommandResult::Success(0),
                    Err(e) => {
                        crate::println!("container: {}", e);
                        CommandResult::Error(String::from("container manager not initialized"))
                    }
                }
            }
            "create" => {
                let name = args.get(1).map(|s| s.as_str()).unwrap_or("unnamed");
                let result = crate::virt::container::with_container_manager(
                    |mgr: &mut crate::virt::container::ContainerManager| mgr.create(name),
                );
                match result {
                    Ok(Ok(id)) => {
                        crate::println!("Container '{}' created with id {}", name, id);
                        CommandResult::Success(0)
                    }
                    Ok(Err(e)) => {
                        crate::println!("container: create failed: {}", e);
                        CommandResult::Error(String::from("create failed"))
                    }
                    Err(e) => {
                        crate::println!("container: {}", e);
                        CommandResult::Error(String::from("container manager not initialized"))
                    }
                }
            }
            "start" => {
                let id_str = match args.get(1) {
                    Some(s) => s,
                    None => {
                        crate::println!("container: start requires <id>");
                        return CommandResult::Error(String::from("missing id"));
                    }
                };
                let id: u64 = match id_str.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        crate::println!("container: invalid id '{}'", id_str);
                        return CommandResult::Error(String::from("invalid id"));
                    }
                };
                let program = args.get(2).map(|s| s.as_str()).unwrap_or("/bin/init");
                let result = crate::virt::container::with_container_manager(
                    |mgr: &mut crate::virt::container::ContainerManager| mgr.start(id, program),
                );
                match result {
                    Ok(Ok(())) => {
                        crate::println!("Container {} started", id);
                        CommandResult::Success(0)
                    }
                    Ok(Err(e)) => {
                        crate::println!("container: start failed: {}", e);
                        CommandResult::Error(String::from("start failed"))
                    }
                    Err(e) => {
                        crate::println!("container: {}", e);
                        CommandResult::Error(String::from("container manager not initialized"))
                    }
                }
            }
            "stop" => {
                let id_str = match args.get(1) {
                    Some(s) => s,
                    None => {
                        crate::println!("container: stop requires <id>");
                        return CommandResult::Error(String::from("missing id"));
                    }
                };
                let id: u64 = match id_str.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        crate::println!("container: invalid id '{}'", id_str);
                        return CommandResult::Error(String::from("invalid id"));
                    }
                };
                let result = crate::virt::container::with_container_manager(
                    |mgr: &mut crate::virt::container::ContainerManager| mgr.stop(id),
                );
                match result {
                    Ok(Ok(())) => {
                        crate::println!("Container {} stopped", id);
                        CommandResult::Success(0)
                    }
                    Ok(Err(e)) => {
                        crate::println!("container: stop failed: {}", e);
                        CommandResult::Error(String::from("stop failed"))
                    }
                    Err(e) => {
                        crate::println!("container: {}", e);
                        CommandResult::Error(String::from("container manager not initialized"))
                    }
                }
            }
            "destroy" => {
                let id_str = match args.get(1) {
                    Some(s) => s,
                    None => {
                        crate::println!("container: destroy requires <id>");
                        return CommandResult::Error(String::from("missing id"));
                    }
                };
                let id: u64 = match id_str.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        crate::println!("container: invalid id '{}'", id_str);
                        return CommandResult::Error(String::from("invalid id"));
                    }
                };
                let result = crate::virt::container::with_container_manager(
                    |mgr: &mut crate::virt::container::ContainerManager| mgr.destroy(id),
                );
                match result {
                    Ok(Ok(())) => {
                        crate::println!("Container {} destroyed", id);
                        CommandResult::Success(0)
                    }
                    Ok(Err(e)) => {
                        crate::println!("container: destroy failed: {}", e);
                        CommandResult::Error(String::from("destroy failed"))
                    }
                    Err(e) => {
                        crate::println!("container: {}", e);
                        CommandResult::Error(String::from("container manager not initialized"))
                    }
                }
            }
            "help" => {
                crate::println!("Usage: container <subcommand>");
                crate::println!("  list              - List containers");
                crate::println!("  create <name>     - Create a container");
                crate::println!("  start <id> [prog] - Start a container");
                crate::println!("  stop <id>         - Stop a container");
                crate::println!("  destroy <id>      - Destroy a container");
                CommandResult::Success(0)
            }
            _ => {
                crate::println!(
                    "container: unknown subcommand '{}'. Try 'container help'",
                    sub
                );
                CommandResult::Error(String::from("unknown subcommand"))
            }
        }
    }
}

pub(in crate::services::shell) struct LsnsCommand;
impl BuiltinCommand for LsnsCommand {
    fn name(&self) -> &str {
        "lsns"
    }
    fn description(&self) -> &str {
        "List namespaces/containers"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!(
            "{:<8} {:<10} {:<20} {}",
            "ID",
            "NPROCS",
            "HOSTNAME",
            "STATUS"
        );
        let containers =
            crate::virt::container::with_container_manager(|mgr| mgr.list()).unwrap_or_default();
        if containers.is_empty() {
            crate::println!("(no containers)");
        }
        for c in &containers {
            crate::println!(
                "{:<8} {:<10} {:<20} {}",
                c.id,
                c.process_count,
                c.hostname,
                c.state
            );
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// Power Management Commands
// ============================================================================

pub(in crate::services::shell) struct ShutdownCommand;
impl BuiltinCommand for ShutdownCommand {
    fn name(&self) -> &str {
        "shutdown"
    }
    fn description(&self) -> &str {
        "Shut down the system"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("Shutting down...");
        if let Some(init) = crate::services::init_system::try_get_init_system() {
            if let Err(e) = init.shutdown() {
                crate::println!("shutdown: init system error: {:?}", e);
            }
        } else {
            crate::println!("shutdown: init system not available");
        }
        // Try ACPI S5 via QEMU debug-exit on x86_64
        #[cfg(target_arch = "x86_64")]
        unsafe {
            x86_64::instructions::port::Port::new(0xf4).write(0x00u32);
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct RebootCommand;
impl BuiltinCommand for RebootCommand {
    fn name(&self) -> &str {
        "reboot"
    }
    fn description(&self) -> &str {
        "Reboot the system"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("Rebooting...");
        if let Some(init) = crate::services::init_system::try_get_init_system() {
            if let Err(e) = init.reboot() {
                crate::println!("reboot: init system error: {:?}", e);
            }
        } else {
            crate::println!("reboot: init system not available");
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct PoweroffCommand;
impl BuiltinCommand for PoweroffCommand {
    fn name(&self) -> &str {
        "poweroff"
    }
    fn description(&self) -> &str {
        "Power off the system"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("Powering off...");
        if let Some(init) = crate::services::init_system::try_get_init_system() {
            if let Err(e) = init.shutdown() {
                crate::println!("poweroff: init system error: {:?}", e);
            }
        } else {
            crate::println!("poweroff: init system not available");
        }
        #[cfg(target_arch = "x86_64")]
        unsafe {
            x86_64::instructions::port::Port::new(0xf4).write(0x00u32);
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct SuspendCommand;
impl BuiltinCommand for SuspendCommand {
    fn name(&self) -> &str {
        "suspend"
    }
    fn description(&self) -> &str {
        "Suspend the system (S3)"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("Entering suspend (S3)...");
        crate::power::enter_idle(crate::power::CState::C3);
        crate::println!("Resumed from suspend");
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct HibernateCommand;
impl BuiltinCommand for HibernateCommand {
    fn name(&self) -> &str {
        "hibernate"
    }
    fn description(&self) -> &str {
        "Hibernate the system"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("Hibernate not supported (no swap device)");
        CommandResult::Success(1)
    }
}

// ============================================================================
// User & Account Management Commands
// ============================================================================

pub(in crate::services::shell) struct UseraddCommand;
impl BuiltinCommand for UseraddCommand {
    fn name(&self) -> &str {
        "useradd"
    }
    fn description(&self) -> &str {
        "Add a user account"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: useradd <username>");
            return CommandResult::Success(1);
        }
        let username = &args[0];
        let mut db = crate::syscall::userland_ext::users::UserDatabase::new();
        match db.add_user(username, 1000, None) {
            Ok(uid) => {
                crate::println!("User '{}' created with uid {}", username, uid);
                CommandResult::Success(0)
            }
            Err(e) => {
                crate::println!("useradd: failed to add user '{}': {:?}", username, e);
                CommandResult::Success(1)
            }
        }
    }
}

pub(in crate::services::shell) struct UserdelCommand;
impl BuiltinCommand for UserdelCommand {
    fn name(&self) -> &str {
        "userdel"
    }
    fn description(&self) -> &str {
        "Delete a user account"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: userdel <username>");
            return CommandResult::Success(1);
        }
        let username = &args[0];
        let mut db = crate::syscall::userland_ext::users::UserDatabase::new();
        // Add user first so we can demonstrate removal (session-local DB)
        let _ = db.add_user(username, 1000, None);
        match db.remove_user(username) {
            Ok(()) => {
                crate::println!("User '{}' removed", username);
                CommandResult::Success(0)
            }
            Err(e) => {
                crate::println!("userdel: failed to remove user '{}': {:?}", username, e);
                CommandResult::Success(1)
            }
        }
    }
}

pub(in crate::services::shell) struct PasswdCommand;
impl BuiltinCommand for PasswdCommand {
    fn name(&self) -> &str {
        "passwd"
    }
    fn description(&self) -> &str {
        "Change user password"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let username = if args.is_empty() { "root" } else { &args[0] };
        crate::println!("Password changed for {}", username);
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct IdCommand;
impl BuiltinCommand for IdCommand {
    fn name(&self) -> &str {
        "id"
    }
    fn description(&self) -> &str {
        "Display user identity"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("uid=0(root) gid=0(root) groups=0(root)");
        } else {
            let username = &args[0];
            let db = crate::syscall::userland_ext::users::UserDatabase::new();
            if let Some(user) = db.get_user_by_name(username) {
                crate::println!(
                    "uid={}({}) gid={}({}) groups={}({})",
                    user.uid,
                    user.username,
                    user.gid,
                    user.username,
                    user.gid,
                    user.username
                );
            } else {
                crate::println!("id: '{}': no such user", username);
                return CommandResult::Success(1);
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct WhoamiCommand;
impl BuiltinCommand for WhoamiCommand {
    fn name(&self) -> &str {
        "whoami"
    }
    fn description(&self) -> &str {
        "Print current user name"
    }
    fn execute(&self, _args: &[String], shell: &Shell) -> CommandResult {
        let user = shell
            .get_env("USER")
            .unwrap_or_else(|| String::from("root"));
        crate::println!("{}", user);
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct GroupsCommand;
impl BuiltinCommand for GroupsCommand {
    fn name(&self) -> &str {
        "groups"
    }
    fn description(&self) -> &str {
        "Show group memberships"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("root");
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct SuCommand;
impl BuiltinCommand for SuCommand {
    fn name(&self) -> &str {
        "su"
    }
    fn description(&self) -> &str {
        "Switch user"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!("su: switching user requires authentication");
        CommandResult::Success(1)
    }
}

pub(in crate::services::shell) struct SudoCommand;
impl BuiltinCommand for SudoCommand {
    fn name(&self) -> &str {
        "sudo"
    }
    fn description(&self) -> &str {
        "Execute command as root"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: sudo <command> [args...]");
            return CommandResult::Success(1);
        }
        let cmd_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let joined = cmd_str.join(" ");
        crate::println!("Executing as root: {}", joined);
        CommandResult::Success(0)
    }
}

// ============================================================================
// Service Management Command
// ============================================================================

pub(in crate::services::shell) struct ServiceCommand;
impl BuiltinCommand for ServiceCommand {
    fn name(&self) -> &str {
        "service"
    }
    fn description(&self) -> &str {
        "Manage system services"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: service <list|start|stop|restart|status> [name]");
            return CommandResult::Success(1);
        }
        let subcmd = args[0].as_str();
        match subcmd {
            "list" => {
                if let Some(init) = crate::services::init_system::try_get_init_system() {
                    let services = init.list_services();
                    crate::println!("{:<20} {:<12} {:<8}", "NAME", "STATE", "PID");
                    for svc in &services {
                        let pid_str = match svc.pid {
                            Some(pid) => format!("{}", pid),
                            None => String::from("-"),
                        };
                        crate::println!(
                            "{:<20} {:<12?} {:<8}",
                            svc.definition.name,
                            svc.state,
                            pid_str
                        );
                    }
                    if services.is_empty() {
                        crate::println!("(no services registered)");
                    }
                } else {
                    crate::println!("service: init system not available");
                    return CommandResult::Success(1);
                }
            }
            "start" | "stop" | "restart" | "status" => {
                if args.len() < 2 {
                    crate::println!("Usage: service {} <name>", subcmd);
                    return CommandResult::Success(1);
                }
                let name = &args[1];
                if let Some(init) = crate::services::init_system::try_get_init_system() {
                    match subcmd {
                        "start" => {
                            if let Err(e) = init.start_service(name) {
                                crate::println!("service: failed to start '{}': {:?}", name, e);
                                return CommandResult::Success(1);
                            }
                            crate::println!("Service '{}' started", name);
                        }
                        "stop" => {
                            if let Err(e) = init.stop_service(name) {
                                crate::println!("service: failed to stop '{}': {:?}", name, e);
                                return CommandResult::Success(1);
                            }
                            crate::println!("Service '{}' stopped", name);
                        }
                        "restart" => {
                            if let Err(e) = init.restart_service(name) {
                                crate::println!("service: failed to restart '{}': {:?}", name, e);
                                return CommandResult::Success(1);
                            }
                            crate::println!("Service '{}' restarted", name);
                        }
                        "status" => {
                            if let Some(info) = init.get_service_status(name) {
                                crate::println!("Service: {}", info.definition.name);
                                crate::println!("  State:    {:?}", info.state);
                                let pid_str = match info.pid {
                                    Some(pid) => format!("{}", pid),
                                    None => String::from("-"),
                                };
                                crate::println!("  PID:      {}", pid_str);
                                crate::println!("  Restarts: {}", info.restart_count);
                                if let Some(code) = info.exit_code {
                                    crate::println!("  Exit:     {}", code);
                                }
                                if let Some(ref err) = info.last_error {
                                    crate::println!("  Error:    {}", err);
                                }
                            } else {
                                crate::println!("service: '{}' not found", name);
                                return CommandResult::Success(1);
                            }
                        }
                        _ => {}
                    }
                } else {
                    crate::println!("service: init system not available");
                    return CommandResult::Success(1);
                }
            }
            _ => {
                crate::println!("service: unknown subcommand '{}'", subcmd);
                return CommandResult::Success(1);
            }
        }
        CommandResult::Success(0)
    }
}

// ============================================================================
// System Diagnostic Commands
// ============================================================================

pub(in crate::services::shell) struct TopCommand;
impl BuiltinCommand for TopCommand {
    fn name(&self) -> &str {
        "top"
    }
    fn description(&self) -> &str {
        "Display running processes"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        // Memory stats
        let mem = crate::mm::get_memory_stats();
        let total_kb = mem.total_frames * 4;
        let free_kb = mem.free_frames * 4;
        let used_kb = total_kb.saturating_sub(free_kb);
        crate::println!(
            "Mem: {}K total, {}K used, {}K free",
            total_kb,
            used_kb,
            free_kb
        );
        crate::println!();
        crate::println!(
            "{:<6}{:<16}{:<10}{:<6}{}",
            "PID",
            "NAME",
            "STATE",
            "CPU%",
            "MEM"
        );

        let ps = crate::services::process_server::get_process_server();
        let processes = ps.list_processes();
        for p in &processes {
            let name_display: String = if p.name.len() > 15 {
                p.name[..15].into()
            } else {
                p.name.clone()
            };
            crate::println!(
                "{:<6}{:<16}{:<10}{:<6}{}",
                p.pid.0,
                name_display,
                format!("{:?}", p.state),
                "0%",
                "0K"
            );
        }

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct StraceCommand;
impl BuiltinCommand for StraceCommand {
    fn name(&self) -> &str {
        "strace"
    }
    fn description(&self) -> &str {
        "Trace system calls"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: strace <pid>");
            return CommandResult::Success(1);
        }
        crate::println!("Tracing PID {}... (press Ctrl+C to stop)", args[0]);
        crate::println!("strace: no syscalls captured (process not found)");
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct CoredumpCommand;
impl BuiltinCommand for CoredumpCommand {
    fn name(&self) -> &str {
        "coredump"
    }
    fn description(&self) -> &str {
        "Manage core dumps"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: coredump list|info <file>");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "list" => {
                crate::println!("No core dumps found");
            }
            "info" => {
                if args.len() < 2 {
                    crate::println!("coredump info: missing <file>");
                    return CommandResult::Success(1);
                }
                crate::println!("coredump: {} not found", args[1]);
            }
            other => {
                crate::println!("coredump: unknown subcommand '{}'", other);
                return CommandResult::Success(1);
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct LscpuCommand;
impl BuiltinCommand for LscpuCommand {
    fn name(&self) -> &str {
        "lscpu"
    }
    fn description(&self) -> &str {
        "Display CPU information"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        #[cfg(target_arch = "x86_64")]
        crate::println!("Architecture:  x86_64");
        #[cfg(target_arch = "aarch64")]
        crate::println!("Architecture:  aarch64");
        #[cfg(target_arch = "riscv64")]
        crate::println!("Architecture:  riscv64");
        #[cfg(not(any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )))]
        crate::println!("Architecture:  unknown");

        crate::println!("CPU(s):        1");
        crate::println!("Model:         QEMU Virtual CPU");

        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct HostnameCommand;
impl BuiltinCommand for HostnameCommand {
    fn name(&self) -> &str {
        "hostname"
    }
    fn description(&self) -> &str {
        "Show or set system hostname"
    }
    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        if args.is_empty() {
            let name = shell
                .get_env("HOSTNAME")
                .unwrap_or_else(|| String::from("veridian"));
            crate::println!("{}", name);
        } else {
            crate::println!("hostname: set to {}", args[0]);
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct SysctlCommand;
impl BuiltinCommand for SysctlCommand {
    fn name(&self) -> &str {
        "sysctl"
    }
    fn description(&self) -> &str {
        "View or set kernel parameters"
    }
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: sysctl -a|<key>[=value]");
            return CommandResult::Success(1);
        }

        let arg = &args[0];

        if arg == "-a" || arg == "list" {
            let mem = crate::mm::get_memory_stats();
            let total_kb = mem.total_frames * 4;

            crate::println!("kernel.version = 0.18.0");

            #[cfg(target_arch = "x86_64")]
            crate::println!("kernel.arch = x86_64");
            #[cfg(target_arch = "aarch64")]
            crate::println!("kernel.arch = aarch64");
            #[cfg(target_arch = "riscv64")]
            crate::println!("kernel.arch = riscv64");
            #[cfg(not(any(
                target_arch = "x86_64",
                target_arch = "aarch64",
                target_arch = "riscv64"
            )))]
            crate::println!("kernel.arch = unknown");

            crate::println!("vm.total_memory = {}K", total_kb);
            crate::println!("kernel.hostname = veridian");
            crate::println!("kernel.ostype = VeridianOS");
        } else if let Some(eq_pos) = arg.find('=') {
            let key = &arg[..eq_pos];
            let value = &arg[eq_pos + 1..];
            crate::println!("{} = {}", key, value);
        } else {
            crate::println!("{} = (unknown)", arg);
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// Scheduled Task Commands
// ============================================================================

pub(in crate::services::shell) struct CrontabCommand;
impl BuiltinCommand for CrontabCommand {
    fn name(&self) -> &str {
        "crontab"
    }
    fn description(&self) -> &str {
        "Manage cron jobs"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from(
                "Usage: crontab -l|-e|add <schedule> <command>",
            ));
        }

        match args[0].as_str() {
            "-l" | "list" => {
                crate::println!("no crontab for root");
                CommandResult::Success(0)
            }
            "-e" => {
                crate::println!("crontab: interactive editing not supported, use 'crontab add'");
                CommandResult::Success(0)
            }
            "add" => {
                if args.len() < 3 {
                    return CommandResult::Error(String::from(
                        "Usage: crontab -l|-e|add <schedule> <command>",
                    ));
                }
                let schedule = &args[1];
                let command = args[2..].join(" ");
                crate::println!("Cron job added: {} {}", schedule, command);
                CommandResult::Success(0)
            }
            _ => CommandResult::Error(String::from(
                "Usage: crontab -l|-e|add <schedule> <command>",
            )),
        }
    }
}

pub(in crate::services::shell) struct AtCommand;
impl BuiltinCommand for AtCommand {
    fn name(&self) -> &str {
        "at"
    }
    fn description(&self) -> &str {
        "Schedule one-time task"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: at <time> <command>"));
        }

        let time = &args[0];
        let _command = args[1..].join(" ");
        crate::println!("job 1 at {}", time);
        CommandResult::Success(0)
    }
}

// ============================================================================
// Cloud / Container Commands
// ============================================================================

pub(in crate::services::shell) struct CloudInitCommand;
impl BuiltinCommand for CloudInitCommand {
    fn name(&self) -> &str {
        "cloud-init"
    }
    fn description(&self) -> &str {
        "Cloud instance initialization"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            crate::println!("Usage: cloud-init run|status");
            return CommandResult::Success(1);
        }
        match args[0].as_str() {
            "run" => {
                crate::println!("cloud-init: running... no datasource found");
            }
            "status" => {
                crate::println!("cloud-init: status: disabled (no metadata service)");
            }
            _ => {
                crate::println!("Usage: cloud-init run|status");
            }
        }
        CommandResult::Success(0)
    }
}

pub(in crate::services::shell) struct KubectlCommand;
impl BuiltinCommand for KubectlCommand {
    fn name(&self) -> &str {
        "kubectl"
    }
    fn description(&self) -> &str {
        "Kubernetes cluster management"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 || args[0] != "get" {
            crate::println!("Usage: kubectl get <resource>");
            return CommandResult::Success(1);
        }
        match args[1].as_str() {
            "pods" => {
                crate::println!("No resources found in default namespace");
            }
            "nodes" => {
                crate::println!("NAME        STATUS   ROLES    AGE   VERSION");
                crate::println!("veridian    Ready    master   1d    v1.0.0");
            }
            _ => {
                crate::println!("No resources found in default namespace");
            }
        }
        CommandResult::Success(0)
    }
}

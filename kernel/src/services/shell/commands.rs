//! Built-in command implementations for the VeridianOS shell.
//!
//! This module contains all built-in shell commands organized by category:
//! - Help and shell management (help, history, clear, exit)
//! - Directory navigation (cd, pwd, ls, mkdir)
//! - File operations (cat, echo, touch, rm)
//! - Process management (ps, kill, uptime)
//! - System information (mount, lsmod)
//! - Environment variables (env, export, unset)

#![allow(unused_variables, unused_assignments)]

use alloc::{format, string::String, vec::Vec};

use super::{BuiltinCommand, CommandResult, Shell};
use crate::process::ProcessId;

// ============================================================================
// Help & Shell Management Commands
// ============================================================================

pub(super) struct HelpCommand;
impl BuiltinCommand for HelpCommand {
    fn name(&self) -> &str {
        "help"
    }
    fn description(&self) -> &str {
        "Show available commands"
    }

    fn execute(&self, _args: &[String], shell: &Shell) -> CommandResult {
        crate::println!("VeridianOS Shell - Available Commands:");
        crate::println!();

        let builtins = shell.builtins.read();
        let mut commands: Vec<_> = builtins.values().collect();
        commands.sort_by_key(|cmd| cmd.name());

        for cmd in commands {
            crate::println!("  {:12} - {}", cmd.name(), cmd.description());
        }

        crate::println!();
        crate::println!("Use 'command --help' for detailed help on specific commands.");

        CommandResult::Success(0)
    }
}

pub(super) struct HistoryCommand;
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

pub(super) struct ClearCommand;
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

pub(super) struct ExitCommand;
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
// Directory Navigation Commands
// ============================================================================

pub(super) struct CdCommand;
impl BuiltinCommand for CdCommand {
    fn name(&self) -> &str {
        "cd"
    }
    fn description(&self) -> &str {
        "Change current directory"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        let target = if args.is_empty() {
            shell.get_env("HOME").unwrap_or_else(|| String::from("/"))
        } else {
            args[0].clone()
        };

        match shell.set_cwd(target.clone()) {
            Ok(()) => {
                // Synchronize VFS CWD so resolve_path() handles relative paths
                if let Some(vfs_lock) = crate::fs::try_get_vfs() {
                    let _ = vfs_lock.write().set_cwd(shell.get_cwd());
                }
                shell.set_env(String::from("PWD"), target);
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("cd: {}: {}", target, e)),
        }
    }
}

pub(super) struct PwdCommand;
impl BuiltinCommand for PwdCommand {
    fn name(&self) -> &str {
        "pwd"
    }
    fn description(&self) -> &str {
        "Print current working directory"
    }

    fn execute(&self, _args: &[String], shell: &Shell) -> CommandResult {
        crate::println!("{}", shell.get_cwd());
        CommandResult::Success(0)
    }
}

pub(super) struct LsCommand;
impl BuiltinCommand for LsCommand {
    fn name(&self) -> &str {
        "ls"
    }
    fn description(&self) -> &str {
        "List directory contents"
    }

    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        let path = if args.is_empty() {
            shell.get_cwd()
        } else {
            args[0].clone()
        };

        match crate::fs::get_vfs().read().resolve_path(&path) {
            Ok(node) => match node.readdir() {
                Ok(entries) => {
                    for entry in entries {
                        let type_char = match entry.node_type {
                            crate::fs::NodeType::Directory => 'd',
                            crate::fs::NodeType::File => '-',
                            crate::fs::NodeType::CharDevice => 'c',
                            crate::fs::NodeType::BlockDevice => 'b',
                            crate::fs::NodeType::Pipe => 'p',
                            crate::fs::NodeType::Socket => 's',
                            crate::fs::NodeType::Symlink => 'l',
                        };
                        crate::println!("{} {}", type_char, entry.name);
                    }
                    CommandResult::Success(0)
                }
                Err(e) => CommandResult::Error(format!("ls: {}", e)),
            },
            Err(e) => CommandResult::Error(format!("ls: {}: {}", path, e)),
        }
    }
}

pub(super) struct MkdirCommand;
impl BuiltinCommand for MkdirCommand {
    fn name(&self) -> &str {
        "mkdir"
    }
    fn description(&self) -> &str {
        "Create directories"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("mkdir: missing operand"));
        }

        for path in args {
            match crate::fs::get_vfs()
                .read()
                .mkdir(path, crate::fs::Permissions::default())
            {
                Ok(()) => {}
                Err(e) => return CommandResult::Error(format!("mkdir: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// File Operation Commands
// ============================================================================

pub(super) struct CatCommand;
impl BuiltinCommand for CatCommand {
    fn name(&self) -> &str {
        "cat"
    }
    fn description(&self) -> &str {
        "Display file contents"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("cat: missing file operand"));
        }

        for path in args {
            match crate::fs::get_vfs().read().resolve_path(path) {
                Ok(node) => {
                    let mut buffer = [0u8; 4096];
                    let mut offset = 0;

                    loop {
                        match node.read(offset, &mut buffer) {
                            Ok(0) => break, // EOF
                            Ok(bytes_read) => {
                                // Convert to string and print
                                if let Ok(text) = core::str::from_utf8(&buffer[..bytes_read]) {
                                    crate::print!("{}", text);
                                }
                                offset += bytes_read;
                            }
                            Err(e) => {
                                return CommandResult::Error(format!("cat: {}: {}", path, e));
                            }
                        }
                    }
                }
                Err(e) => return CommandResult::Error(format!("cat: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(super) struct EchoCommand;
impl BuiltinCommand for EchoCommand {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "Display text"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if !args.is_empty() {
            let output = args.join(" ");
            crate::println!("{}", output);
        } else {
            crate::println!();
        }
        CommandResult::Success(0)
    }
}

pub(super) struct TouchCommand;
impl BuiltinCommand for TouchCommand {
    fn name(&self) -> &str {
        "touch"
    }
    fn description(&self) -> &str {
        "Create empty files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("touch: missing file operand"));
        }

        for path in args {
            // Check if the file already exists
            if crate::fs::file_exists(path) {
                // File exists -- update timestamps (metadata update)
                continue;
            }

            // File doesn't exist -- create it via VFS
            if let Some(vfs) = crate::fs::try_get_vfs() {
                let vfs_guard = vfs.read();
                // Split into parent path and filename
                let (parent_path, filename) = if let Some(pos) = path.rfind('/') {
                    if pos == 0 {
                        ("/", &path[1..])
                    } else {
                        (&path[..pos], &path[pos + 1..])
                    }
                } else {
                    // Relative to cwd
                    (vfs_guard.get_cwd(), path.as_str())
                };

                match vfs_guard.resolve_path(parent_path) {
                    Ok(parent) => {
                        if let Err(e) = parent.create(filename, crate::fs::Permissions::default()) {
                            return CommandResult::Error(format!(
                                "touch: cannot create '{}': {}",
                                path, e
                            ));
                        }
                    }
                    Err(e) => {
                        return CommandResult::Error(format!(
                            "touch: cannot create '{}': parent directory not found: {}",
                            path, e
                        ));
                    }
                }
            } else {
                return CommandResult::Error(String::from("touch: VFS not initialized"));
            }
        }

        CommandResult::Success(0)
    }
}

pub(super) struct RmCommand;
impl BuiltinCommand for RmCommand {
    fn name(&self) -> &str {
        "rm"
    }
    fn description(&self) -> &str {
        "Remove files and directories"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("rm: missing operand"));
        }

        for path in args {
            match crate::fs::get_vfs().read().unlink(path) {
                Ok(()) => {}
                Err(e) => return CommandResult::Error(format!("rm: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// Process Management Commands
// ============================================================================

pub(super) struct PsCommand;
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

pub(super) struct KillCommand;
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

pub(super) struct UptimeCommand;
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

// ============================================================================
// System Information Commands
// ============================================================================

pub(super) struct MountCommand;
impl BuiltinCommand for MountCommand {
    fn name(&self) -> &str {
        "mount"
    }
    fn description(&self) -> &str {
        "Show mounted filesystems"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        if let Some(vfs) = crate::fs::try_get_vfs() {
            let vfs_guard = vfs.read();
            let mounts = vfs_guard.list_mounts();
            for (path, fs_name, readonly) in &mounts {
                let mode = if *readonly { "ro" } else { "rw" };
                crate::println!("{} on {} ({})", path, fs_name, mode);
            }
        } else {
            crate::println!("mount: VFS not initialized");
        }
        CommandResult::Success(0)
    }
}

pub(super) struct LsmodCommand;
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

pub(super) struct EnvCommand;
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

pub(super) struct ExportCommand;
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

pub(super) struct UnsetCommand;
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
// Package Management Commands
// ============================================================================

pub(super) struct PkgCommand;
impl BuiltinCommand for PkgCommand {
    fn name(&self) -> &str {
        "pkg"
    }
    fn description(&self) -> &str {
        "Package management (install, remove, update, upgrade, list, search, info, verify)"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from(
                "Usage: pkg <install|remove|update|upgrade|list|search|info|verify> [args...]",
            ));
        }

        let subcommand = args[0].as_str();
        let sub_args = &args[1..];

        match subcommand {
            "install" => pkg_install(sub_args),
            "remove" => pkg_remove(sub_args),
            "update" => pkg_update(),
            "upgrade" => pkg_upgrade(sub_args),
            "list" => pkg_list(sub_args),
            "search" => pkg_search(sub_args),
            "info" => pkg_info(sub_args),
            "verify" => pkg_verify(sub_args),
            _ => CommandResult::Error(format!("pkg: unknown subcommand '{}'", subcommand)),
        }
    }
}

/// Install a package by name
fn pkg_install(args: &[String]) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(String::from("Usage: pkg install <name>"));
    }

    let name = &args[0];
    match crate::pkg::with_package_manager(|mgr| mgr.install(name.clone(), String::from("*"))) {
        Some(Ok(())) => {
            crate::println!("Package '{}' installed successfully", name);
            CommandResult::Success(0)
        }
        Some(Err(e)) => CommandResult::Error(format!("pkg install: {}", e)),
        None => CommandResult::Error(String::from("pkg: package manager not initialized")),
    }
}

/// Remove an installed package
fn pkg_remove(args: &[String]) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(String::from("Usage: pkg remove <name>"));
    }

    let name = &args[0];
    match crate::pkg::with_package_manager(|mgr| mgr.remove(name)) {
        Some(Ok(())) => {
            crate::println!("Package '{}' removed successfully", name);
            CommandResult::Success(0)
        }
        Some(Err(e)) => CommandResult::Error(format!("pkg remove: {}", e)),
        None => CommandResult::Error(String::from("pkg: package manager not initialized")),
    }
}

/// Refresh repository index
fn pkg_update() -> CommandResult {
    match crate::pkg::with_package_manager(|mgr| mgr.update()) {
        Some(Ok(())) => {
            crate::println!("Package index updated");
            CommandResult::Success(0)
        }
        Some(Err(e)) => CommandResult::Error(format!("pkg update: {}", e)),
        None => CommandResult::Error(String::from("pkg: package manager not initialized")),
    }
}

/// Upgrade packages (reinstall with latest version)
fn pkg_upgrade(args: &[String]) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(String::from(
            "Usage: pkg upgrade <name> or pkg upgrade --all",
        ));
    }

    let target = args[0].as_str();

    if target == "--all" {
        // Upgrade all: remove and reinstall each package
        match crate::pkg::with_package_manager(|mgr| {
            let packages = mgr.list_installed();
            let count = packages.len();
            for (name, _version) in &packages {
                crate::println!("  Upgrading {} ...", name);
                // Remove and reinstall to get the latest version
                let _ = mgr.remove(name);
                let _ = mgr.install(name.clone(), String::from("*"));
            }
            count
        }) {
            Some(count) => {
                crate::println!("Upgraded {} package(s)", count);
                CommandResult::Success(0)
            }
            None => CommandResult::Error(String::from("pkg: package manager not initialized")),
        }
    } else {
        match crate::pkg::with_package_manager(|mgr| {
            mgr.remove(&String::from(target))?;
            mgr.install(String::from(target), String::from("*"))
        }) {
            Some(Ok(())) => {
                crate::println!("Package '{}' upgraded successfully", target);
                CommandResult::Success(0)
            }
            Some(Err(e)) => CommandResult::Error(format!("pkg upgrade: {}", e)),
            None => CommandResult::Error(String::from("pkg: package manager not initialized")),
        }
    }
}

/// List packages
fn pkg_list(args: &[String]) -> CommandResult {
    let filter = args.first().map(|s| s.as_str()).unwrap_or("--installed");

    match filter {
        "--installed" => match crate::pkg::with_package_manager(|mgr| mgr.list_installed()) {
            Some(packages) => {
                if packages.is_empty() {
                    crate::println!("No packages installed");
                } else {
                    crate::println!("Installed packages:");
                    for (name, version) in &packages {
                        crate::println!(
                            "  {} {}.{}.{}",
                            name,
                            version.major,
                            version.minor,
                            version.patch
                        );
                    }
                    crate::println!("{} package(s) installed", packages.len());
                }
                CommandResult::Success(0)
            }
            None => CommandResult::Error(String::from("pkg: package manager not initialized")),
        },
        "--available" => {
            crate::println!("Available packages (from repositories):");
            crate::println!("  Run 'pkg update' first to refresh the index");
            CommandResult::Success(0)
        }
        _ => CommandResult::Error(format!(
            "pkg list: unknown filter '{}' (use --installed or --available)",
            filter
        )),
    }
}

/// Search installed packages by name substring
fn pkg_search(args: &[String]) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(String::from("Usage: pkg search <query>"));
    }

    let query = &args[0];
    match crate::pkg::with_package_manager(|mgr| {
        let packages = mgr.list_installed();
        let mut count = 0usize;
        for (name, version) in &packages {
            if name.contains(query.as_str()) {
                crate::println!(
                    "  {} {}.{}.{}",
                    name,
                    version.major,
                    version.minor,
                    version.patch
                );
                count += 1;
            }
        }
        count
    }) {
        Some(0) => {
            crate::println!("No packages found matching '{}'", query);
            CommandResult::Success(0)
        }
        Some(count) => {
            crate::println!("{} result(s)", count);
            CommandResult::Success(0)
        }
        None => CommandResult::Error(String::from("pkg: package manager not initialized")),
    }
}

/// Show package details
fn pkg_info(args: &[String]) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Error(String::from("Usage: pkg info <name>"));
    }

    let name = &args[0];
    match crate::pkg::with_package_manager(|mgr| mgr.get_metadata(name).cloned()) {
        Some(Some(meta)) => {
            crate::println!("Package: {}", meta.name);
            crate::println!(
                "Version: {}.{}.{}",
                meta.version.major,
                meta.version.minor,
                meta.version.patch
            );
            crate::println!("Author:  {}", meta.author);
            crate::println!("License: {}", meta.license);
            crate::println!("Description: {}", meta.description);
            crate::println!("Installed: yes");
            if !meta.dependencies.is_empty() {
                crate::println!("Dependencies:");
                for dep in &meta.dependencies {
                    crate::println!("  {} ({})", dep.name, dep.version_req);
                }
            }
            CommandResult::Success(0)
        }
        Some(None) => CommandResult::Error(format!("pkg info: package '{}' not found", name)),
        None => CommandResult::Error(String::from("pkg: package manager not initialized")),
    }
}

/// Verify package is installed
fn pkg_verify(args: &[String]) -> CommandResult {
    if args.is_empty() {
        // Verify all installed packages
        match crate::pkg::with_package_manager(|mgr| {
            let packages = mgr.list_installed();
            for (name, version) in &packages {
                crate::println!(
                    "  {} {}.{}.{} ... OK",
                    name,
                    version.major,
                    version.minor,
                    version.patch
                );
            }
            crate::println!("Verified: {} package(s)", packages.len());
            packages.len()
        }) {
            Some(_count) => CommandResult::Success(0),
            None => CommandResult::Error(String::from("pkg: package manager not initialized")),
        }
    } else {
        let name = &args[0];
        match crate::pkg::with_package_manager(|mgr| mgr.is_installed(name)) {
            Some(true) => {
                crate::println!("Package '{}': OK (installed)", name);
                CommandResult::Success(0)
            }
            Some(false) => CommandResult::Error(format!("pkg verify: '{}' is not installed", name)),
            None => CommandResult::Error(String::from("pkg: package manager not initialized")),
        }
    }
}

// ============================================================================
// Utility Commands
// ============================================================================

pub(super) struct TrueCommand;
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

pub(super) struct FalseCommand;
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

pub(super) struct TestCommand;
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
pub(super) struct BracketTestCommand;
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

/// Evaluate a test expression and return true/false
fn evaluate_test(args: &[String]) -> bool {
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
fn parse_i64(s: &str) -> i64 {
    s.parse::<i64>().unwrap_or(0)
}

// ============================================================================
// Text Processing Commands
// ============================================================================

pub(super) struct WcCommand;
impl BuiltinCommand for WcCommand {
    fn name(&self) -> &str {
        "wc"
    }
    fn description(&self) -> &str {
        "Count lines, words, and characters"
    }

    // println! is a no-op on non-x86_64; totals are accumulated for the
    // summary line but the final assignment is never "read" when the macro
    // expands to nothing.
    #[cfg_attr(not(target_arch = "x86_64"), allow(unused_assignments))]
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("wc: missing file operand"));
        }

        let mut total_lines = 0usize;
        let mut total_words = 0usize;
        let mut total_chars = 0usize;
        let multiple = args.len() > 1;

        for path in args {
            match read_file_to_string(path) {
                Ok(content) => {
                    let lines = content.matches('\n').count();
                    let words = content.split_whitespace().count();
                    let chars = content.len();
                    crate::println!("{:8}{:8}{:8} {}", lines, words, chars, path);
                    total_lines += lines;
                    total_words += words;
                    total_chars += chars;
                }
                Err(e) => return CommandResult::Error(format!("wc: {}: {}", path, e)),
            }
        }

        if multiple {
            crate::println!("{:8}{:8}{:8} total", total_lines, total_words, total_chars);
        }

        CommandResult::Success(0)
    }
}

pub(super) struct HeadCommand;
impl BuiltinCommand for HeadCommand {
    fn name(&self) -> &str {
        "head"
    }
    fn description(&self) -> &str {
        "Show first N lines of file (default 10)"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("head: missing file operand"));
        }

        let mut num_lines: usize = 10;
        let mut file_args_start = 0;

        // Parse -n NUM or -NUM option
        if args.len() >= 2 && args[0] == "-n" {
            num_lines = args[1].parse().unwrap_or(10);
            file_args_start = 2;
        } else if args[0].starts_with('-') {
            if let Ok(n) = args[0][1..].parse::<usize>() {
                num_lines = n;
                file_args_start = 1;
            }
        }

        let files = &args[file_args_start..];
        if files.is_empty() {
            return CommandResult::Error(String::from("head: missing file operand"));
        }

        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    if files.len() > 1 {
                        crate::println!("==> {} <==", path);
                    }
                    for (i, line) in content.split('\n').enumerate() {
                        if i >= num_lines {
                            break;
                        }
                        crate::println!("{}", line);
                    }
                }
                Err(e) => return CommandResult::Error(format!("head: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(super) struct TailCommand;
impl BuiltinCommand for TailCommand {
    fn name(&self) -> &str {
        "tail"
    }
    fn description(&self) -> &str {
        "Show last N lines of file (default 10)"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("tail: missing file operand"));
        }

        let mut num_lines: usize = 10;
        let mut file_args_start = 0;

        // Parse -n NUM or -NUM option
        if args.len() >= 2 && args[0] == "-n" {
            num_lines = args[1].parse().unwrap_or(10);
            file_args_start = 2;
        } else if args[0].starts_with('-') {
            if let Ok(n) = args[0][1..].parse::<usize>() {
                num_lines = n;
                file_args_start = 1;
            }
        }

        let files = &args[file_args_start..];
        if files.is_empty() {
            return CommandResult::Error(String::from("tail: missing file operand"));
        }

        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    if files.len() > 1 {
                        crate::println!("==> {} <==", path);
                    }
                    let lines: Vec<&str> = content.split('\n').collect();
                    let start = if lines.len() > num_lines {
                        lines.len() - num_lines
                    } else {
                        0
                    };
                    for line in &lines[start..] {
                        crate::println!("{}", line);
                    }
                }
                Err(e) => return CommandResult::Error(format!("tail: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(super) struct GrepCommand;
impl BuiltinCommand for GrepCommand {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search for pattern in files"
    }

    // println! is a no-op on non-x86_64, making the if/else branches
    // (which differ only in their format strings) appear identical to clippy.
    #[cfg_attr(not(target_arch = "x86_64"), allow(clippy::if_same_then_else))]
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: grep PATTERN FILE..."));
        }

        let mut case_insensitive = false;
        let mut show_line_numbers = false;
        let mut invert_match = false;
        let mut pattern_idx = 0;

        // Parse flags
        for (i, arg) in args.iter().enumerate() {
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        'i' => case_insensitive = true,
                        'n' => show_line_numbers = true,
                        'v' => invert_match = true,
                        _ => {}
                    }
                }
                pattern_idx = i + 1;
            } else {
                break;
            }
        }

        if pattern_idx >= args.len() || pattern_idx + 1 > args.len() {
            return CommandResult::Error(String::from("Usage: grep [-inv] PATTERN FILE..."));
        }

        let pattern = &args[pattern_idx];
        let files = &args[pattern_idx + 1..];

        if files.is_empty() {
            return CommandResult::Error(String::from("grep: missing file operand"));
        }

        let pattern_lower = if case_insensitive {
            pattern.to_ascii_lowercase()
        } else {
            String::new()
        };

        let mut found_any = false;
        let show_filename = files.len() > 1;

        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    for (line_num, line) in content.split('\n').enumerate() {
                        let matches = if case_insensitive {
                            line.to_ascii_lowercase().contains(pattern_lower.as_str())
                        } else {
                            line.contains(pattern.as_str())
                        };

                        let should_print = if invert_match { !matches } else { matches };

                        if should_print {
                            found_any = true;
                            let prefix = if show_filename {
                                format!("{}:", path)
                            } else {
                                String::new()
                            };
                            if show_line_numbers {
                                crate::println!("{}{}:{}", prefix, line_num + 1, line);
                            } else {
                                crate::println!("{}{}", prefix, line);
                            }
                        }
                    }
                }
                Err(e) => return CommandResult::Error(format!("grep: {}: {}", path, e)),
            }
        }

        if found_any {
            CommandResult::Success(0)
        } else {
            CommandResult::Success(1)
        }
    }
}

pub(super) struct SortCommand;
impl BuiltinCommand for SortCommand {
    fn name(&self) -> &str {
        "sort"
    }
    fn description(&self) -> &str {
        "Sort lines of text files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let mut reverse = false;
        let mut numeric = false;
        let mut file_args_start = 0;

        // Parse flags
        for (i, arg) in args.iter().enumerate() {
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        'r' => reverse = true,
                        'n' => numeric = true,
                        _ => {}
                    }
                }
                file_args_start = i + 1;
            } else {
                break;
            }
        }

        let files = &args[file_args_start..];
        if files.is_empty() {
            return CommandResult::Error(String::from("sort: missing file operand"));
        }

        // Collect all lines from all files
        let mut all_lines: Vec<String> = Vec::new();
        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    for line in content.split('\n') {
                        if !line.is_empty() {
                            all_lines.push(String::from(line));
                        }
                    }
                }
                Err(e) => return CommandResult::Error(format!("sort: {}: {}", path, e)),
            }
        }

        if numeric {
            all_lines.sort_by(|a, b| {
                let a_val = a.trim().parse::<i64>().unwrap_or(0);
                let b_val = b.trim().parse::<i64>().unwrap_or(0);
                a_val.cmp(&b_val)
            });
        } else {
            all_lines.sort();
        }

        if reverse {
            all_lines.reverse();
        }

        for line in &all_lines {
            crate::println!("{}", line);
        }

        CommandResult::Success(0)
    }
}

pub(super) struct UniqCommand;
impl BuiltinCommand for UniqCommand {
    fn name(&self) -> &str {
        "uniq"
    }
    fn description(&self) -> &str {
        "Remove adjacent duplicate lines"
    }

    // println! is a no-op on non-x86_64, making the if/else branches
    // (which differ only in their format strings) appear identical to clippy.
    #[cfg_attr(not(target_arch = "x86_64"), allow(clippy::if_same_then_else))]
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("uniq: missing file operand"));
        }

        let mut count_mode = false;
        let mut duplicate_only = false;
        let mut file_args_start = 0;

        for (i, arg) in args.iter().enumerate() {
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        'c' => count_mode = true,
                        'd' => duplicate_only = true,
                        _ => {}
                    }
                }
                file_args_start = i + 1;
            } else {
                break;
            }
        }

        let files = &args[file_args_start..];
        if files.is_empty() {
            return CommandResult::Error(String::from("uniq: missing file operand"));
        }

        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    let mut prev_line: Option<&str> = None;
                    let mut count: usize = 0;

                    for line in content.split('\n') {
                        if prev_line == Some(line) {
                            count += 1;
                        } else {
                            // Print previous line group
                            if let Some(prev) = prev_line {
                                let should_print = !duplicate_only || count > 1;
                                if should_print {
                                    if count_mode {
                                        crate::println!("{:7} {}", count, prev);
                                    } else {
                                        crate::println!("{}", prev);
                                    }
                                }
                            }
                            prev_line = Some(line);
                            count = 1;
                        }
                    }

                    // Print last group
                    if let Some(prev) = prev_line {
                        let should_print = !duplicate_only || count > 1;
                        if should_print {
                            if count_mode {
                                crate::println!("{:7} {}", count, prev);
                            } else {
                                crate::println!("{}", prev);
                            }
                        }
                    }
                }
                Err(e) => return CommandResult::Error(format!("uniq: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(super) struct CutCommand;
impl BuiltinCommand for CutCommand {
    fn name(&self) -> &str {
        "cut"
    }
    fn description(&self) -> &str {
        "Extract fields from lines"
    }

    // println! is a no-op on non-x86_64, making the if/else branches
    // (which differ only in their format strings) appear identical to clippy.
    #[cfg_attr(not(target_arch = "x86_64"), allow(clippy::if_same_then_else))]
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let mut delimiter = '\t';
        let mut field: Option<usize> = None;
        let mut file_args_start = 0;
        let mut i = 0;

        // Parse options
        while i < args.len() {
            match args[i].as_str() {
                "-d" => {
                    if i + 1 < args.len() {
                        delimiter = args[i + 1].chars().next().unwrap_or('\t');
                        i += 2;
                        file_args_start = i;
                    } else {
                        return CommandResult::Error(String::from(
                            "cut: option requires an argument -- 'd'",
                        ));
                    }
                }
                "-f" => {
                    if i + 1 < args.len() {
                        field = args[i + 1].parse().ok();
                        i += 2;
                        file_args_start = i;
                    } else {
                        return CommandResult::Error(String::from(
                            "cut: option requires an argument -- 'f'",
                        ));
                    }
                }
                arg if arg.starts_with("-d") => {
                    delimiter = arg[2..].chars().next().unwrap_or('\t');
                    i += 1;
                    file_args_start = i;
                }
                arg if arg.starts_with("-f") => {
                    field = arg[2..].parse().ok();
                    i += 1;
                    file_args_start = i;
                }
                _ => break,
            }
        }

        let field_num = match field {
            Some(f) if f >= 1 => f,
            _ => {
                return CommandResult::Error(String::from("cut: you must specify a field with -f"))
            }
        };

        let files = &args[file_args_start..];
        if files.is_empty() {
            return CommandResult::Error(String::from("cut: missing file operand"));
        }

        for path in files {
            match read_file_to_string(path) {
                Ok(content) => {
                    for line in content.split('\n') {
                        if line.is_empty() {
                            continue;
                        }
                        let fields: Vec<&str> = line.split(delimiter).collect();
                        if field_num <= fields.len() {
                            crate::println!("{}", fields[field_num - 1]);
                        } else {
                            crate::println!();
                        }
                    }
                }
                Err(e) => return CommandResult::Error(format!("cut: {}: {}", path, e)),
            }
        }

        CommandResult::Success(0)
    }
}

pub(super) struct TrCommand;
impl BuiltinCommand for TrCommand {
    fn name(&self) -> &str {
        "tr"
    }
    fn description(&self) -> &str {
        "Translate characters"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 3 {
            return CommandResult::Error(String::from(
                "Usage: tr SET1 SET2 STRING (or tr SET1 SET2 < file)",
            ));
        }

        let set1: Vec<char> = args[0].chars().collect();
        let set2: Vec<char> = args[1].chars().collect();
        let input = args[2..].join(" ");

        let mut output = String::new();
        for ch in input.chars() {
            let mut replaced = false;
            for (i, &s1) in set1.iter().enumerate() {
                if ch == s1 {
                    if i < set2.len() {
                        output.push(set2[i]);
                    } else if !set2.is_empty() {
                        // Use last char of set2 for overflow
                        output.push(set2[set2.len() - 1]);
                    }
                    replaced = true;
                    break;
                }
            }
            if !replaced {
                output.push(ch);
            }
        }

        crate::println!("{}", output);
        CommandResult::Success(0)
    }
}

pub(super) struct TeeCommand;
impl BuiltinCommand for TeeCommand {
    fn name(&self) -> &str {
        "tee"
    }
    fn description(&self) -> &str {
        "Read input and write to file and stdout"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: tee FILE TEXT..."));
        }

        let file_path = &args[0];
        let text = args[1..].join(" ");

        // Print to stdout
        crate::println!("{}", text);

        // Write to file
        match crate::fs::write_file(file_path, text.as_bytes()) {
            Ok(_) => CommandResult::Success(0),
            Err(e) => CommandResult::Error(format!("tee: {}: {}", file_path, e)),
        }
    }
}

pub(super) struct PrintfCommand;
impl BuiltinCommand for PrintfCommand {
    fn name(&self) -> &str {
        "printf"
    }
    fn description(&self) -> &str {
        "Formatted output"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("printf: missing format string"));
        }

        let fmt = &args[0];
        let fmt_args = &args[1..];
        let mut arg_idx = 0;
        let mut output = String::new();
        let mut chars = fmt.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '%' {
                match chars.next() {
                    Some('s') => {
                        if arg_idx < fmt_args.len() {
                            output.push_str(&fmt_args[arg_idx]);
                            arg_idx += 1;
                        }
                    }
                    Some('d') => {
                        if arg_idx < fmt_args.len() {
                            let val = fmt_args[arg_idx].parse::<i64>().unwrap_or(0);
                            output.push_str(&format!("{}", val));
                            arg_idx += 1;
                        }
                    }
                    Some('x') => {
                        if arg_idx < fmt_args.len() {
                            let val = fmt_args[arg_idx].parse::<u64>().unwrap_or(0);
                            output.push_str(&format!("{:x}", val));
                            arg_idx += 1;
                        }
                    }
                    Some('o') => {
                        if arg_idx < fmt_args.len() {
                            let val = fmt_args[arg_idx].parse::<u64>().unwrap_or(0);
                            output.push_str(&format!("{:o}", val));
                            arg_idx += 1;
                        }
                    }
                    Some('%') => output.push('%'),
                    Some(c) => {
                        output.push('%');
                        output.push(c);
                    }
                    None => output.push('%'),
                }
            } else if ch == '\\' {
                match chars.next() {
                    Some('n') => output.push('\n'),
                    Some('t') => output.push('\t'),
                    Some('\\') => output.push('\\'),
                    Some('0') => output.push('\0'),
                    Some(c) => {
                        output.push('\\');
                        output.push(c);
                    }
                    None => output.push('\\'),
                }
            } else {
                output.push(ch);
            }
        }

        crate::print!("{}", output);
        CommandResult::Success(0)
    }
}

// ============================================================================
// I/O Commands
// ============================================================================

pub(super) struct ReadCommand;
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
// File Management Commands
// ============================================================================

pub(super) struct CpCommand;
impl BuiltinCommand for CpCommand {
    fn name(&self) -> &str {
        "cp"
    }
    fn description(&self) -> &str {
        "Copy files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: cp SOURCE DEST"));
        }

        let source = &args[0];
        let dest = &args[1];

        match crate::fs::read_file(source) {
            Ok(data) => match crate::fs::write_file(dest, &data) {
                Ok(_) => CommandResult::Success(0),
                Err(e) => CommandResult::Error(format!("cp: cannot create '{}': {}", dest, e)),
            },
            Err(e) => CommandResult::Error(format!("cp: cannot read '{}': {}", source, e)),
        }
    }
}

pub(super) struct MvCommand;
impl BuiltinCommand for MvCommand {
    fn name(&self) -> &str {
        "mv"
    }
    fn description(&self) -> &str {
        "Move (rename) files"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: mv SOURCE DEST"));
        }

        let source = &args[0];
        let dest = &args[1];

        // Read source
        let data = match crate::fs::read_file(source) {
            Ok(d) => d,
            Err(e) => return CommandResult::Error(format!("mv: cannot read '{}': {}", source, e)),
        };

        // Write to destination
        if let Err(e) = crate::fs::write_file(dest, &data) {
            return CommandResult::Error(format!("mv: cannot write '{}': {}", dest, e));
        }

        // Remove source
        match crate::fs::get_vfs().read().unlink(source) {
            Ok(()) => CommandResult::Success(0),
            Err(e) => CommandResult::Error(format!("mv: cannot remove '{}': {}", source, e)),
        }
    }
}

pub(super) struct ChmodCommand;
impl BuiltinCommand for ChmodCommand {
    fn name(&self) -> &str {
        "chmod"
    }
    fn description(&self) -> &str {
        "Change file permissions"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::Error(String::from("Usage: chmod MODE FILE"));
        }

        let mode_str = &args[0];
        let path = &args[1];

        // Parse octal mode (e.g., 755, 644)
        let mode = match u16::from_str_radix(mode_str, 8) {
            Ok(m) => m,
            Err(_) => {
                return CommandResult::Error(format!(
                    "chmod: invalid mode '{}' (use octal, e.g., 755)",
                    mode_str
                ))
            }
        };

        // Verify file exists
        match crate::fs::get_vfs().read().resolve_path(path) {
            Ok(_node) => {
                // In a full implementation, we would set permissions on the node.
                // For now, acknowledge the operation.
                crate::println!("chmod: set mode {:o} on {}", mode, path);
                CommandResult::Success(0)
            }
            Err(e) => CommandResult::Error(format!("chmod: {}: {}", path, e)),
        }
    }
}

// ============================================================================
// System Information Commands
// ============================================================================

pub(super) struct DateCommand;
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

/// Check if a year is a leap year
fn is_leap_year(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

pub(super) struct UnameCommand;
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
            parts.push("0.7.1");
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

pub(super) struct FreeCommand;
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

pub(super) struct DmesgCommand;
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

pub(super) struct DfCommand;
impl BuiltinCommand for DfCommand {
    fn name(&self) -> &str {
        "df"
    }
    fn description(&self) -> &str {
        "Show filesystem disk space usage"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::println!(
            "{:<16} {:>10} {:>10} {:>10} {:>6} {}",
            "Filesystem",
            "Size",
            "Used",
            "Avail",
            "Use%",
            "Mounted on"
        );

        if let Some(vfs) = crate::fs::try_get_vfs() {
            let vfs_guard = vfs.read();
            let mounts = vfs_guard.list_mounts();
            for (path, fs_name, _readonly) in &mounts {
                // RamFS/DevFS/ProcFS are in-memory, show nominal values
                crate::println!(
                    "{:<16} {:>10} {:>10} {:>10} {:>5}% {}",
                    fs_name,
                    "-",
                    "-",
                    "-",
                    "0",
                    path
                );
            }
        } else {
            crate::println!("df: VFS not initialized");
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// Shell Control Commands
// ============================================================================

pub(super) struct SetCommand;
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

pub(super) struct SourceCommand;
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
pub(super) struct DotCommand;
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

pub(super) struct AliasCommand;
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

pub(super) struct UnaliasCommand;
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

pub(super) struct TypeCommand;
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

pub(super) struct WhichCommand;
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

pub(super) struct FgCommand;
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

pub(super) struct BgCommand;
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

pub(super) struct JobsCommand;
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
// Helper Functions
// ============================================================================

/// Read a file from VFS and return its contents as a String.
/// Uses a 4096-byte buffer with offset-based reading to handle larger files.
fn read_file_to_string(path: &str) -> Result<String, String> {
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

// ============================================================================
// Filesystem sync command
// ============================================================================

pub(super) struct SyncCommand;
impl BuiltinCommand for SyncCommand {
    fn name(&self) -> &str {
        "sync"
    }
    fn description(&self) -> &str {
        "Flush all pending writes to disk"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        if let Some(vfs) = crate::fs::try_get_vfs() {
            match vfs.read().sync() {
                Ok(()) => {
                    crate::println!("sync: filesystems synced");
                    CommandResult::Success(0)
                }
                Err(e) => {
                    crate::println!("sync: error: {:?}", e);
                    CommandResult::Error(String::from("sync failed"))
                }
            }
        } else {
            crate::println!("sync: VFS not initialized");
            CommandResult::Error(String::from("VFS not initialized"))
        }
    }
}

// ============================================================================
// Performance Commands
// ============================================================================

pub(super) struct PerfCommand;
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

pub(super) struct TraceCommand;
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

pub(super) struct AcpiCommand;
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
// Network Commands
// ============================================================================

pub(super) struct IfconfigCommand;
impl BuiltinCommand for IfconfigCommand {
    fn name(&self) -> &str {
        "ifconfig"
    }
    fn description(&self) -> &str {
        "Display network interface configuration"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let devices = crate::net::device::list_devices();
        if devices.is_empty() {
            crate::println!("No network interfaces found.");
            return CommandResult::Success(0);
        }

        for dev_name in &devices {
            crate::net::device::with_device(dev_name, |dev| {
                let mac = dev.mac_address();
                let state = dev.state();
                let stats = dev.statistics();
                let caps = dev.capabilities();

                crate::println!(
                    "{}: flags=<{:?}> mtu {}",
                    dev.name(),
                    state,
                    caps.max_transmission_unit
                );
                crate::println!(
                    "        ether {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                    mac.0[0],
                    mac.0[1],
                    mac.0[2],
                    mac.0[3],
                    mac.0[4],
                    mac.0[5]
                );

                // Show IP config for non-loopback interfaces
                if dev.name() != "lo0" {
                    let config = crate::net::ip::get_interface_config();
                    let ip = config.ip_addr;
                    let mask = config.subnet_mask;
                    crate::println!(
                        "        inet {}.{}.{}.{} netmask {}.{}.{}.{}",
                        ip.0[0],
                        ip.0[1],
                        ip.0[2],
                        ip.0[3],
                        mask.0[0],
                        mask.0[1],
                        mask.0[2],
                        mask.0[3],
                    );
                } else {
                    crate::println!("        inet 127.0.0.1 netmask 255.0.0.0");
                }

                crate::println!(
                    "        RX packets {} bytes {}  errors {} dropped {}",
                    stats.rx_packets,
                    stats.rx_bytes,
                    stats.rx_errors,
                    stats.rx_dropped
                );
                crate::println!(
                    "        TX packets {} bytes {}  errors {} dropped {}",
                    stats.tx_packets,
                    stats.tx_bytes,
                    stats.tx_errors,
                    stats.tx_dropped
                );
                crate::println!();
            });
        }

        CommandResult::Success(0)
    }
}

pub(super) struct DhcpCommand;
impl BuiltinCommand for DhcpCommand {
    fn name(&self) -> &str {
        "dhcp"
    }
    fn description(&self) -> &str {
        "Trigger DHCP discovery on primary interface"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        match crate::net::dhcp::start_dhcp() {
            Ok(()) => {
                crate::println!("DHCP discovery initiated.");
                if let Some(state) = crate::net::dhcp::get_dhcp_state() {
                    crate::println!("Current state: {:?}", state);
                }
            }
            Err(e) => {
                crate::println!("DHCP failed: {:?}", e);
            }
        }
        CommandResult::Success(0)
    }
}

pub(super) struct NetstatCommand;
impl BuiltinCommand for NetstatCommand {
    fn name(&self) -> &str {
        "netstat"
    }
    fn description(&self) -> &str {
        "Show network socket and connection statistics"
    }

    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        let net_stats = crate::net::get_stats();
        let tcp_stats = crate::net::tcp::get_stats();
        let udp_stats = crate::net::udp::get_stats();

        crate::println!("Network Statistics:");
        crate::println!("  Packets sent:     {}", net_stats.packets_sent);
        crate::println!("  Packets received: {}", net_stats.packets_received);
        crate::println!("  Bytes sent:       {}", net_stats.bytes_sent);
        crate::println!("  Bytes received:   {}", net_stats.bytes_received);
        crate::println!("  Errors:           {}", net_stats.errors);
        crate::println!();
        crate::println!("TCP:");
        crate::println!("  Active connections: {}", tcp_stats.active_connections);
        crate::println!("  Bytes sent:         {}", tcp_stats.total_bytes_sent);
        crate::println!("  Bytes received:     {}", tcp_stats.total_bytes_recv);
        crate::println!("  Retransmissions:    {}", tcp_stats.retransmissions);
        crate::println!();
        crate::println!("UDP:");
        crate::println!("  Active sockets:     {}", udp_stats.active_sockets);
        crate::println!("  Datagrams queued:   {}", udp_stats.datagrams_queued);

        CommandResult::Success(0)
    }
}

pub(super) struct ArpCommand;
impl BuiltinCommand for ArpCommand {
    fn name(&self) -> &str {
        "arp"
    }
    fn description(&self) -> &str {
        "Show ARP cache entries"
    }

    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        // Check for flush subcommand
        if !args.is_empty() && args[0] == "flush" {
            crate::net::arp::flush_cache();
            crate::println!("ARP cache flushed.");
            return CommandResult::Success(0);
        }

        let entries = crate::net::arp::get_cache_entries();
        if entries.is_empty() {
            crate::println!("ARP cache is empty.");
        } else {
            crate::println!("{:<18} {:<20} {}", "IP Address", "MAC Address", "Type");
            for (ip, mac) in &entries {
                crate::println!(
                    "{}.{}.{}.{:<10} {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}   dynamic",
                    ip.0[0],
                    ip.0[1],
                    ip.0[2],
                    ip.0[3],
                    mac.0[0],
                    mac.0[1],
                    mac.0[2],
                    mac.0[3],
                    mac.0[4],
                    mac.0[5],
                );
            }
            crate::println!();
            crate::println!("{} entries", entries.len());
        }

        CommandResult::Success(0)
    }
}

// ============================================================================
// Desktop / GUI Commands
// ============================================================================

pub(super) struct StartGuiCommand;
impl BuiltinCommand for StartGuiCommand {
    fn name(&self) -> &str {
        "startgui"
    }
    fn description(&self) -> &str {
        "Start the graphical desktop environment"
    }
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        crate::desktop::renderer::start_desktop();
        CommandResult::Success(0)
    }
}

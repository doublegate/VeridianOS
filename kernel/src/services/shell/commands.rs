//! Built-in command implementations for the VeridianOS shell.
//!
//! This module contains all built-in shell commands organized by category:
//! - Help and shell management (help, history, clear, exit)
//! - Directory navigation (cd, pwd, ls, mkdir)
//! - File operations (cat, echo, touch, rm)
//! - Process management (ps, kill, uptime)
//! - System information (mount, lsmod)
//! - Environment variables (env, export, unset)

#![allow(unused_variables)]

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

        // TODO(phase3): Implement file creation via VFS create call
        for path in args {
            crate::println!("touch: {} (not implemented)", path);
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
        // TODO(phase3): Get actual system uptime from clock subsystem
        crate::println!("uptime: 0 days, 0 hours, 0 minutes");
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
        // TODO(phase3): Query VFS for actual mount information
        crate::println!("/ on ramfs (rw)");
        crate::println!("/dev on devfs (rw)");
        crate::println!("/proc on procfs (rw)");
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

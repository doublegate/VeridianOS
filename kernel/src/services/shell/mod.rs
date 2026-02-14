//! VeridianOS Shell Implementation
//!
//! Basic shell with command parsing and built-in commands.
//!
//! # Module structure
//!
//! - [`mod.rs`](self) - Shell struct, main loop, command dispatch, and public
//!   types
//! - [`commands`] - All built-in command implementations
//! - [`state`] - Global singleton management (init, get_shell, try_get_shell)

// Many variables in this module are only used in println! calls which are
// no-ops on some architectures (like AArch64), causing unused variable warnings.
#![allow(unused_variables)]

mod commands;
mod state;

// Re-export the public API from the state module so that callers using
// `services::shell::init()`, `services::shell::get_shell()`, etc. continue
// to work without any path changes.
use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use commands::{
    CatCommand, CdCommand, ClearCommand, EchoCommand, EnvCommand, ExitCommand, ExportCommand,
    HelpCommand, HistoryCommand, KillCommand, LsCommand, LsmodCommand, MkdirCommand, MountCommand,
    PsCommand, PwdCommand, RmCommand, TouchCommand, UnsetCommand, UptimeCommand,
};
use spin::RwLock;
pub use state::{get_shell, init, run_shell, try_get_shell};

/// Command execution result
#[derive(Debug)]
pub enum CommandResult {
    Success(i32),
    Error(String),
    NotFound,
    Exit(i32),
}

/// Shell built-in command
pub trait BuiltinCommand: Send + Sync {
    /// Get command name
    fn name(&self) -> &str;

    /// Get command description
    fn description(&self) -> &str;

    /// Execute the command
    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult;
}

/// Shell environment variable
#[derive(Debug, Clone)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// Shell configuration
#[derive(Debug, Clone)]
pub struct ShellConfig {
    pub prompt: String,
    pub history_size: usize,
    pub path: Vec<String>,
    pub editor: String,
    pub pager: String,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            prompt: String::from("veridian $ "),
            history_size: 1000,
            path: vec![
                String::from("/bin"),
                String::from("/usr/bin"),
                String::from("/sbin"),
                String::from("/usr/sbin"),
            ],
            editor: String::from("vi"),
            pager: String::from("less"),
        }
    }
}

/// VeridianOS Shell
pub struct Shell {
    /// Shell configuration
    config: ShellConfig,

    /// Environment variables
    pub(crate) environment: RwLock<BTreeMap<String, String>>,

    /// Command history
    pub(crate) history: RwLock<Vec<String>>,

    /// Built-in commands
    pub(crate) builtins: RwLock<BTreeMap<String, Box<dyn BuiltinCommand>>>,

    /// Current working directory
    cwd: RwLock<String>,

    /// Last exit code
    last_exit_code: RwLock<i32>,

    /// Shell is running
    running: RwLock<bool>,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    /// Create a new shell
    pub fn new() -> Self {
        let shell = Self {
            config: ShellConfig::default(),
            environment: RwLock::new(BTreeMap::new()),
            history: RwLock::new(Vec::new()),
            builtins: RwLock::new(BTreeMap::new()),
            cwd: RwLock::new(String::from("/")),
            last_exit_code: RwLock::new(0),
            running: RwLock::new(true),
        };

        // Initialize environment
        shell.init_environment();

        // Register built-in commands
        shell.register_builtins();

        shell
    }

    /// Run the shell
    pub fn run(&self) -> ! {
        crate::println!("VeridianOS Shell v1.0");
        crate::println!("Type 'help' for available commands");

        *self.running.write() = true;

        loop {
            if !*self.running.read() {
                break;
            }

            // Display prompt
            let _prompt = self.expand_prompt();
            crate::print!("{}", _prompt);

            // Read command
            let command_line = self.read_line();

            // Add to history
            if !command_line.trim().is_empty() {
                self.add_to_history(command_line.clone());
            }

            // Execute command
            let result = self.execute_command(&command_line);

            // Handle result
            match result {
                CommandResult::Success(code) => {
                    *self.last_exit_code.write() = code;
                }
                CommandResult::Error(_msg) => {
                    crate::println!("vsh: {}", _msg);
                    *self.last_exit_code.write() = 1;
                }
                CommandResult::NotFound => {
                    if !command_line.trim().is_empty() {
                        crate::println!("vsh: command not found");
                        *self.last_exit_code.write() = 127;
                    }
                }
                CommandResult::Exit(code) => {
                    crate::println!("exit");
                    *self.last_exit_code.write() = code;
                    break;
                }
            }
        }

        // Exit the shell process
        crate::process::lifecycle::exit_process(*self.last_exit_code.read());

        // Should never reach here after exit_process
        loop {
            // SAFETY: These halt/wait-for-interrupt instructions are the
            // standard low-power idle mechanism for each architecture. They
            // are safe in this unreachable context after process exit.
            #[cfg(target_arch = "x86_64")]
            unsafe {
                core::arch::asm!("hlt")
            }

            #[cfg(target_arch = "aarch64")]
            unsafe {
                core::arch::asm!("wfi")
            }

            #[cfg(target_arch = "riscv64")]
            unsafe {
                core::arch::asm!("wfi")
            }

            #[cfg(not(any(
                target_arch = "x86_64",
                target_arch = "aarch64",
                target_arch = "riscv64"
            )))]
            core::hint::spin_loop();
        }
    }

    /// Execute a command line
    pub fn execute_command(&self, command_line: &str) -> CommandResult {
        let tokens = self.tokenize(command_line);
        if tokens.is_empty() {
            return CommandResult::Success(0);
        }

        let command = &tokens[0];
        let args = &tokens[1..];

        // Check for built-in commands
        if let Some(builtin) = self.builtins.read().get(command) {
            return builtin.execute(args, self);
        }

        // Try to execute external command
        self.execute_external_command(command, args)
    }

    /// Get current working directory
    pub fn get_cwd(&self) -> String {
        self.cwd.read().clone()
    }

    /// Set current working directory
    pub fn set_cwd(&self, path: String) -> Result<(), &'static str> {
        // Verify directory exists using VFS
        let vfs = crate::fs::get_vfs().read();
        let node = vfs.resolve_path(&path)?;
        let metadata = node.metadata()?;

        if metadata.node_type != crate::fs::NodeType::Directory {
            return Err("Not a directory");
        }

        *self.cwd.write() = path;
        Ok(())
    }

    /// Get environment variable
    pub fn get_env(&self, name: &str) -> Option<String> {
        self.environment.read().get(name).cloned()
    }

    /// Set environment variable
    pub fn set_env(&self, name: String, value: String) {
        self.environment.write().insert(name, value);
    }

    /// Get all environment variables
    pub fn get_all_env(&self) -> Vec<String> {
        self.environment
            .read()
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect()
    }

    /// Stop the shell
    pub fn stop(&self) {
        *self.running.write() = false;
    }

    /// Register a builtin command (public API for external modules)
    pub fn register_builtin(&self, command: Box<dyn BuiltinCommand>) {
        let mut builtins = self.builtins.write();
        builtins.insert(command.name().to_string(), command);
    }

    /// Register multiple builtin commands at once
    pub fn register_builtins_batch(&self, commands: Vec<Box<dyn BuiltinCommand>>) {
        let mut builtins = self.builtins.write();
        for command in commands {
            builtins.insert(command.name().to_string(), command);
        }
    }

    // ========================================================================
    // Private methods
    // ========================================================================

    fn init_environment(&self) {
        let mut env = self.environment.write();
        env.insert(
            String::from("PATH"),
            String::from("/bin:/usr/bin:/sbin:/usr/sbin"),
        );
        env.insert(String::from("HOME"), String::from("/"));
        env.insert(String::from("SHELL"), String::from("/bin/vsh"));
        env.insert(String::from("TERM"), String::from("veridian"));
        env.insert(String::from("USER"), String::from("root"));
        env.insert(String::from("PWD"), String::from("/"));
    }

    fn register_builtins(&self) {
        let mut builtins = self.builtins.write();

        // Help command
        builtins.insert("help".into(), Box::new(HelpCommand));
        builtins.insert("?".into(), Box::new(HelpCommand));

        // Directory commands
        builtins.insert("cd".into(), Box::new(CdCommand));
        builtins.insert("pwd".into(), Box::new(PwdCommand));
        builtins.insert("ls".into(), Box::new(LsCommand));
        builtins.insert("mkdir".into(), Box::new(MkdirCommand));

        // File commands
        builtins.insert("cat".into(), Box::new(CatCommand));
        builtins.insert("echo".into(), Box::new(EchoCommand));
        builtins.insert("touch".into(), Box::new(TouchCommand));
        builtins.insert("rm".into(), Box::new(RmCommand));

        // System commands
        builtins.insert("ps".into(), Box::new(PsCommand));
        builtins.insert("kill".into(), Box::new(KillCommand));
        builtins.insert("uptime".into(), Box::new(UptimeCommand));
        builtins.insert("mount".into(), Box::new(MountCommand));
        builtins.insert("lsmod".into(), Box::new(LsmodCommand));

        // Environment commands
        builtins.insert("env".into(), Box::new(EnvCommand));
        builtins.insert("export".into(), Box::new(ExportCommand));
        builtins.insert("unset".into(), Box::new(UnsetCommand));

        // Shell commands
        builtins.insert("history".into(), Box::new(HistoryCommand));
        builtins.insert("clear".into(), Box::new(ClearCommand));
        builtins.insert("exit".into(), Box::new(ExitCommand));
        builtins.insert("logout".into(), Box::new(ExitCommand));
    }

    fn tokenize(&self, command_line: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current_token = String::new();
        let mut in_quotes = false;
        let mut escape_next = false;

        for ch in command_line.chars() {
            if escape_next {
                current_token.push(ch);
                escape_next = false;
            } else if ch == '\\' {
                escape_next = true;
            } else if ch == '"' {
                in_quotes = !in_quotes;
            } else if ch.is_whitespace() && !in_quotes {
                if !current_token.is_empty() {
                    tokens.push(current_token.clone());
                    current_token.clear();
                }
            } else {
                current_token.push(ch);
            }
        }

        if !current_token.is_empty() {
            tokens.push(current_token);
        }

        tokens
    }

    fn execute_external_command(&self, command: &str, args: &[String]) -> CommandResult {
        // Try to find the command in PATH
        let path_env = self.get_env("PATH").unwrap_or_default();
        let paths: Vec<&str> = path_env.split(':').collect();

        for path_dir in paths {
            let full_path = if path_dir.ends_with('/') {
                format!("{}{}", path_dir, command)
            } else {
                format!("{}/{}", path_dir, command)
            };

            // Check if file exists using VFS
            if let Ok(_node) = crate::fs::get_vfs().read().resolve_path(&full_path) {
                // Load and execute the program
                match crate::userspace::load_user_program(
                    &full_path,
                    &args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                    &self
                        .get_all_env()
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>(),
                ) {
                    Ok(pid) => {
                        // Wait for the process to complete
                        // TODO(phase3): Implement proper process waiting (waitpid)
                        crate::println!("Started process {} with PID {}", command, pid.0);
                        return CommandResult::Success(0);
                    }
                    Err(e) => {
                        return CommandResult::Error(format!(
                            "Failed to execute {}: {}",
                            command, e
                        ));
                    }
                }
            }
        }

        CommandResult::NotFound
    }

    fn expand_prompt(&self) -> String {
        let mut prompt = self.config.prompt.clone();

        // Replace prompt variables
        prompt = prompt.replace("\\w", &self.get_cwd());
        prompt = prompt.replace("\\u", &self.get_env("USER").unwrap_or_default());
        prompt = prompt.replace("\\h", "veridian");
        prompt = prompt.replace(
            "\\$",
            if self.get_env("USER").as_deref() == Some("root") {
                "#"
            } else {
                "$"
            },
        );

        prompt
    }

    fn read_line(&self) -> String {
        // Simple line reading - in a real shell this would handle editing, history,
        // etc. For now, just return a placeholder
        String::from("help") // TODO(phase3): Implement line editing with
                             // keyboard input
    }

    fn add_to_history(&self, command: String) {
        let mut history = self.history.write();
        history.push(command);

        // Limit history size
        while history.len() > self.config.history_size {
            history.remove(0);
        }
    }
}

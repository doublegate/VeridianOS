//! VeridianOS Shell Implementation
//!
//! Basic shell with command parsing and built-in commands.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use alloc::boxed::Box;
use alloc::{format, vec};
use spin::RwLock;
use crate::process::ProcessId;

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
    environment: RwLock<BTreeMap<String, String>>,
    
    /// Command history
    history: RwLock<Vec<String>>,
    
    /// Built-in commands
    builtins: RwLock<BTreeMap<String, Box<dyn BuiltinCommand>>>,
    
    /// Current working directory
    cwd: RwLock<String>,
    
    /// Last exit code
    last_exit_code: RwLock<i32>,
    
    /// Shell is running
    running: RwLock<bool>,
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
            let prompt = self.expand_prompt();
            crate::print!("{}", prompt);
            
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
                CommandResult::Error(msg) => {
                    crate::println!("vsh: {}", msg);
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
            #[cfg(target_arch = "x86_64")]
            unsafe { core::arch::asm!("hlt") }
            
            #[cfg(target_arch = "aarch64")]
            unsafe { core::arch::asm!("wfi") }
            
            #[cfg(target_arch = "riscv64")]
            unsafe { core::arch::asm!("wfi") }
            
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "riscv64")))]
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
        self.environment.read()
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect()
    }
    
    /// Stop the shell
    pub fn stop(&self) {
        *self.running.write() = false;
    }
    
    // Private methods
    
    fn init_environment(&self) {
        let mut env = self.environment.write();
        env.insert(String::from("PATH"), String::from("/bin:/usr/bin:/sbin:/usr/sbin"));
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
                    &self.get_all_env().iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                ) {
                    Ok(pid) => {
                        // Wait for the process to complete
                        // TODO: Implement proper process waiting
                        crate::println!("Started process {} with PID {}", command, pid.0);
                        return CommandResult::Success(0);
                    }
                    Err(e) => {
                        return CommandResult::Error(format!("Failed to execute {}: {}", command, e));
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
        prompt = prompt.replace("\\$", if self.get_env("USER").as_deref() == Some("root") { "#" } else { "$" });
        
        prompt
    }
    
    fn read_line(&self) -> String {
        // Simple line reading - in a real shell this would handle editing, history, etc.
        // For now, just return a placeholder
        String::from("help") // TODO: Implement actual line reading
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

// Built-in command implementations

struct HelpCommand;
impl BuiltinCommand for HelpCommand {
    fn name(&self) -> &str { "help" }
    fn description(&self) -> &str { "Show available commands" }
    
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

struct CdCommand;
impl BuiltinCommand for CdCommand {
    fn name(&self) -> &str { "cd" }
    fn description(&self) -> &str { "Change current directory" }
    
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

struct PwdCommand;
impl BuiltinCommand for PwdCommand {
    fn name(&self) -> &str { "pwd" }
    fn description(&self) -> &str { "Print current working directory" }
    
    fn execute(&self, _args: &[String], shell: &Shell) -> CommandResult {
        crate::println!("{}", shell.get_cwd());
        CommandResult::Success(0)
    }
}

struct LsCommand;
impl BuiltinCommand for LsCommand {
    fn name(&self) -> &str { "ls" }
    fn description(&self) -> &str { "List directory contents" }
    
    fn execute(&self, args: &[String], shell: &Shell) -> CommandResult {
        let path = if args.is_empty() {
            shell.get_cwd()
        } else {
            args[0].clone()
        };
        
        match crate::fs::get_vfs().read().resolve_path(&path) {
            Ok(node) => {
                match node.readdir() {
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
                }
            }
            Err(e) => CommandResult::Error(format!("ls: {}: {}", path, e)),
        }
    }
}

struct MkdirCommand;
impl BuiltinCommand for MkdirCommand {
    fn name(&self) -> &str { "mkdir" }
    fn description(&self) -> &str { "Create directories" }
    
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("mkdir: missing operand"));
        }
        
        for path in args {
            match crate::fs::get_vfs().read().mkdir(path, crate::fs::Permissions::default()) {
                Ok(()) => {}
                Err(e) => return CommandResult::Error(format!("mkdir: {}: {}", path, e)),
            }
        }
        
        CommandResult::Success(0)
    }
}

struct CatCommand;
impl BuiltinCommand for CatCommand {
    fn name(&self) -> &str { "cat" }
    fn description(&self) -> &str { "Display file contents" }
    
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

struct EchoCommand;
impl BuiltinCommand for EchoCommand {
    fn name(&self) -> &str { "echo" }
    fn description(&self) -> &str { "Display text" }
    
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

struct TouchCommand;
impl BuiltinCommand for TouchCommand {
    fn name(&self) -> &str { "touch" }
    fn description(&self) -> &str { "Create empty files" }
    
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        if args.is_empty() {
            return CommandResult::Error(String::from("touch: missing file operand"));
        }
        
        // TODO: Implement file creation
        for path in args {
            crate::println!("touch: {} (not implemented)", path);
        }
        
        CommandResult::Success(0)
    }
}

struct RmCommand;
impl BuiltinCommand for RmCommand {
    fn name(&self) -> &str { "rm" }
    fn description(&self) -> &str { "Remove files and directories" }
    
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

struct PsCommand;
impl BuiltinCommand for PsCommand {
    fn name(&self) -> &str { "ps" }
    fn description(&self) -> &str { "List running processes" }
    
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
            
            crate::println!("{:5} {:5} {:8} {}", 
                process.pid.0, process.ppid.0, state, process.name);
        }
        
        CommandResult::Success(0)
    }
}

struct KillCommand;
impl BuiltinCommand for KillCommand {
    fn name(&self) -> &str { "kill" }
    fn description(&self) -> &str { "Send signal to process" }
    
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

struct UptimeCommand;
impl BuiltinCommand for UptimeCommand {
    fn name(&self) -> &str { "uptime" }
    fn description(&self) -> &str { "Show system uptime" }
    
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        // TODO: Get actual system uptime
        crate::println!("uptime: 0 days, 0 hours, 0 minutes");
        CommandResult::Success(0)
    }
}

struct MountCommand;
impl BuiltinCommand for MountCommand {
    fn name(&self) -> &str { "mount" }
    fn description(&self) -> &str { "Show mounted filesystems" }
    
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        // TODO: Show actual mount information
        crate::println!("/ on ramfs (rw)");
        crate::println!("/dev on devfs (rw)");
        crate::println!("/proc on procfs (rw)");
        CommandResult::Success(0)
    }
}

struct LsmodCommand;
impl BuiltinCommand for LsmodCommand {
    fn name(&self) -> &str { "lsmod" }
    fn description(&self) -> &str { "List loaded drivers" }
    
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

struct EnvCommand;
impl BuiltinCommand for EnvCommand {
    fn name(&self) -> &str { "env" }
    fn description(&self) -> &str { "Show environment variables" }
    
    fn execute(&self, _args: &[String], shell: &Shell) -> CommandResult {
        let env_vars = shell.get_all_env();
        for var in env_vars {
            crate::println!("{}", var);
        }
        CommandResult::Success(0)
    }
}

struct ExportCommand;
impl BuiltinCommand for ExportCommand {
    fn name(&self) -> &str { "export" }
    fn description(&self) -> &str { "Set environment variable" }
    
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

struct UnsetCommand;
impl BuiltinCommand for UnsetCommand {
    fn name(&self) -> &str { "unset" }
    fn description(&self) -> &str { "Unset environment variable" }
    
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

struct HistoryCommand;
impl BuiltinCommand for HistoryCommand {
    fn name(&self) -> &str { "history" }
    fn description(&self) -> &str { "Show command history" }
    
    fn execute(&self, _args: &[String], shell: &Shell) -> CommandResult {
        let history = shell.history.read();
        for (i, cmd) in history.iter().enumerate() {
            crate::println!("{:4} {}", i + 1, cmd);
        }
        CommandResult::Success(0)
    }
}

struct ClearCommand;
impl BuiltinCommand for ClearCommand {
    fn name(&self) -> &str { "clear" }
    fn description(&self) -> &str { "Clear the screen" }
    
    fn execute(&self, _args: &[String], _shell: &Shell) -> CommandResult {
        // TODO: Actually clear the screen
        crate::println!("\x1b[2J\x1b[H");
        CommandResult::Success(0)
    }
}

struct ExitCommand;
impl BuiltinCommand for ExitCommand {
    fn name(&self) -> &str { "exit" }
    fn description(&self) -> &str { "Exit the shell" }
    
    fn execute(&self, args: &[String], _shell: &Shell) -> CommandResult {
        let exit_code = if args.is_empty() {
            0
        } else {
            args[0].parse().unwrap_or(1)
        };
        
        CommandResult::Exit(exit_code)
    }
}

/// Global shell instance
/// Global shell instance using pointer pattern for all architectures
/// This avoids static mut Option issues and provides consistent behavior
static mut SHELL_PTR: *mut Shell = core::ptr::null_mut();

/// Initialize the shell
pub fn init() {
    use crate::println;
    
    unsafe {
        if !SHELL_PTR.is_null() {
            println!("[SHELL] WARNING: Already initialized! Skipping re-initialization.");
            return;
        }
        
        let shell = Shell::new();
        
        // Box it and leak to get a static pointer
        let shell_box = alloc::boxed::Box::new(shell);
        let shell_ptr = alloc::boxed::Box::leak(shell_box) as *mut Shell;
        
        // Memory barriers for AArch64
        #[cfg(target_arch = "aarch64")]
        {
            core::arch::asm!(
                "dsb sy",  // Data Synchronization Barrier
                "isb",     // Instruction Synchronization Barrier
                options(nostack, nomem, preserves_flags)
            );
        }
        
        // Memory barriers for RISC-V
        #[cfg(target_arch = "riscv64")]
        {
            core::arch::asm!(
                "fence rw, rw",  // Full memory fence
                options(nostack, nomem, preserves_flags)
            );
        }
        
        // Store the pointer
        SHELL_PTR = shell_ptr;
        
        // Memory barriers after assignment for AArch64
        #[cfg(target_arch = "aarch64")]
        {
            core::arch::asm!(
                "dsb sy",
                "isb",
                options(nostack, nomem, preserves_flags)
            );
        }
        
        // Memory barriers after assignment for RISC-V
        #[cfg(target_arch = "riscv64")]
        {
            core::arch::asm!(
                "fence rw, rw",
                options(nostack, nomem, preserves_flags)
            );
        }
        
        println!("[SHELL] Shell module loaded");
    }
}

/// Get the global shell
pub fn get_shell() -> &'static Shell {
    unsafe {
        if SHELL_PTR.is_null() {
            panic!("Shell not initialized");
        }
        &*SHELL_PTR
    }
}

/// Run shell as a process
pub fn run_shell() -> ! {
    let shell = get_shell();
    shell.run()
}
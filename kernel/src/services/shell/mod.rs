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

pub(crate) mod aliases;
pub(crate) mod ansi;
mod commands;
pub(crate) mod completion;
pub(crate) mod expand;
pub(crate) mod functions;
pub(crate) mod glob;
pub(crate) mod jobs;
pub(crate) mod line_editor;
pub(crate) mod redirect;
pub(crate) mod script;
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
    AliasCommand, BgCommand, BracketTestCommand, CatCommand, CdCommand, ChmodCommand, ClearCommand,
    CpCommand, CutCommand, DateCommand, DfCommand, DmesgCommand, DotCommand, EchoCommand,
    EnvCommand, ExitCommand, ExportCommand, FalseCommand, FgCommand, FreeCommand, GrepCommand,
    HeadCommand, HelpCommand, HistoryCommand, JobsCommand, KillCommand, LsCommand, LsmodCommand,
    MkdirCommand, MountCommand, MvCommand, PerfCommand, PkgCommand, PrintfCommand, PsCommand,
    PwdCommand, ReadCommand, RmCommand, SetCommand, SortCommand, SourceCommand, SyncCommand,
    TailCommand, TeeCommand, TestCommand, TouchCommand, TrCommand, TraceCommand, TrueCommand,
    TypeCommand, UnaliasCommand, UnameCommand, UniqCommand, UnsetCommand, UptimeCommand, WcCommand,
    WhichCommand,
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
            prompt: String::from("\\u@\\h:\\w\\$ "),
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
    pub(crate) last_exit_code: RwLock<i32>,

    /// Shell is running
    running: RwLock<bool>,

    /// Line editor for interactive input
    line_editor: RwLock<line_editor::LineEditor>,

    /// Job table for background process tracking
    pub(crate) job_table: RwLock<jobs::JobTable>,

    /// User-defined function registry
    pub(crate) function_registry: RwLock<functions::FunctionRegistry>,

    /// Command alias registry
    pub(crate) alias_registry: RwLock<aliases::AliasRegistry>,
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
            line_editor: RwLock::new(line_editor::LineEditor::new()),
            job_table: RwLock::new(jobs::JobTable::new()),
            function_registry: RwLock::new(functions::FunctionRegistry::new()),
            alias_registry: RwLock::new(aliases::AliasRegistry::new()),
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
        crate::graphics::fbcon::flush();

        *self.running.write() = true;

        loop {
            if !*self.running.read() {
                break;
            }

            // Display prompt
            let _prompt = self.expand_prompt();
            crate::print!("{}", _prompt);
            crate::graphics::fbcon::flush();
            crate::graphics::fbcon::update_cursor();

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

            crate::graphics::fbcon::flush();

            // Notify the user about completed background jobs
            self.notify_completed_jobs();
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

    /// Execute a command line.
    ///
    /// Supports `&&`, `||`, `;` operators, pipes (`|`), I/O redirections
    /// (`>`, `>>`, `<`, `<<<`, `2>`, `2>&1`), variable expansion, alias
    /// expansion, glob pattern expansion, command substitution (`$(...)`),
    /// and subshell grouping (`(cmd1; cmd2)`).
    pub fn execute_command(&self, command_line: &str) -> CommandResult {
        let trimmed = command_line.trim();
        crate::println!("[SHELL-EXEC] command_line='{}'", trimmed);
        if trimmed.is_empty() {
            return CommandResult::Success(0);
        }

        // --- Phase 0: Subshell grouping `(cmd1; cmd2)` ---
        if let Some(result) = self.try_execute_subshell(trimmed) {
            return result;
        }

        // --- Phase 1: Handle && / || / ; command lists ---
        if let Some(result) = self.try_execute_list(trimmed) {
            return result;
        }

        // --- Phase 2: Expand aliases ---
        let expanded_alias = aliases::expand_aliases(trimmed, &self.alias_registry.read());

        // --- Phase 3: Expand variables ---
        let exit_code = *self.last_exit_code.read();
        let env = self.environment.read().clone();
        let expanded = expand::expand_variables(&expanded_alias, &env, exit_code);

        // --- Phase 4: Check for background execution (`&` suffix) ---
        let (command_str, _is_background) =
            if expanded.ends_with('&') && !expanded.ends_with("&&") && !expanded.ends_with(">&") {
                (expanded[..expanded.len() - 1].trim(), true)
            } else {
                (expanded.as_str(), false)
            };

        if command_str.is_empty() {
            return CommandResult::Success(0);
        }

        // --- Phase 5: Handle pipes ---
        let pipe_segments: Vec<&str> = command_str.split('|').collect();
        if pipe_segments.len() > 1 {
            return self.execute_pipeline(&pipe_segments);
        }

        // --- Phase 6: Tokenize, expand globs, parse redirections ---
        let tokens = self.tokenize(command_str);
        if tokens.is_empty() {
            return CommandResult::Success(0);
        }

        let cwd = self.get_cwd();
        let tokens = glob::expand_globs(tokens, &cwd);

        let (cmd_tokens, redirections) = redirect::parse_redirections(&tokens);
        if cmd_tokens.is_empty() {
            return CommandResult::Success(0);
        }

        let command = &cmd_tokens[0];
        let args = &cmd_tokens[1..];

        // --- Phase 7: Check user-defined functions ---
        {
            let func_reg = self.function_registry.read();
            if let Some(func) = func_reg.get(command) {
                let body = func.body.clone();
                drop(func_reg);
                let mut last_result = CommandResult::Success(0);
                for line in &body {
                    last_result = self.execute_command(line);
                }
                return last_result;
            }
        }

        // --- Phase 8: Check built-in commands ---
        if let Some(builtin) = self.builtins.read().get(command) {
            crate::println!("[SHELL-EXEC] Found builtin: {}", command);
            let result = builtin.execute(args, self);

            // Apply output redirections if any
            if !redirections.is_empty() {
                for redir in &redirections {
                    if let redirect::Redirection::StdoutTo(path)
                    | redirect::Redirection::StdoutAppend(path) = redir
                    {
                        let _ = crate::fs::write_file(path, b"");
                    }
                }
            }

            return result;
        }

        // --- Phase 9: Try external command ---
        crate::println!("[SHELL-EXEC] Trying external command: {}", command);
        let result = self.execute_external_command(command, args);
        match &result {
            CommandResult::Success(code) => {
                crate::println!("[SHELL-EXEC] Result: Success({})", code)
            }
            CommandResult::Error(msg) => crate::println!("[SHELL-EXEC] Result: Error({})", msg),
            CommandResult::NotFound => crate::println!("[SHELL-EXEC] Result: NotFound"),
            CommandResult::Exit(code) => crate::println!("[SHELL-EXEC] Result: Exit({})", code),
        }
        result
    }

    /// Try to split and execute a command list using `;`, `&&`, or `||`.
    ///
    /// Returns `None` if the command contains no list operators.
    fn try_execute_list(&self, command_line: &str) -> Option<CommandResult> {
        // Split on `;` (sequential execution, lowest precedence)
        if command_line.contains(';') {
            let segments: Vec<&str> = command_line.split(';').collect();
            if segments.len() > 1 {
                let mut last_result = CommandResult::Success(0);
                for seg in segments {
                    let seg = seg.trim();
                    if !seg.is_empty() {
                        last_result = self.execute_command(seg);
                    }
                }
                return Some(last_result);
            }
        }

        // Split on `&&` (AND list — right only if left succeeds)
        if command_line.contains("&&") {
            let parts: Vec<&str> = command_line.splitn(2, "&&").collect();
            if parts.len() == 2 {
                let left_result = self.execute_command(parts[0].trim());
                match &left_result {
                    CommandResult::Success(0) => {
                        return Some(self.execute_command(parts[1].trim()));
                    }
                    _ => return Some(left_result),
                }
            }
        }

        // Split on `||` (OR list — right only if left fails)
        if command_line.contains("||") {
            let parts: Vec<&str> = command_line.splitn(2, "||").collect();
            if parts.len() == 2 {
                let left_result = self.execute_command(parts[0].trim());
                match &left_result {
                    CommandResult::Success(0) => return Some(left_result),
                    _ => return Some(self.execute_command(parts[1].trim())),
                }
            }
        }

        None
    }

    /// Try to execute a subshell grouping: `(cmd1; cmd2; ...)`.
    ///
    /// If the entire command line is wrapped in parentheses, the inner
    /// commands are executed sequentially in a nested scope. The subshell
    /// inherits the current environment but does not propagate changes back
    /// (since we have no fork semantics yet, environment isolation is
    /// noted but not enforced — matching the kernel-space limitation).
    ///
    /// Returns `None` if the command is not a subshell grouping.
    fn try_execute_subshell(&self, command_line: &str) -> Option<CommandResult> {
        let trimmed = command_line.trim();

        // Must start with '(' and end with ')'
        if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
            return None;
        }

        // Verify balanced parentheses — the outer parens must match
        let inner = &trimmed[1..trimmed.len() - 1];

        // Check that the opening '(' at position 0 matches the closing ')' at
        // the end, not some intermediate grouping like `(a) && (b)`.
        let mut depth = 1i32;
        for (i, ch) in inner.chars().enumerate() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && i < inner.len() - 1 {
                        // The opening paren closed before the end, so this is
                        // not a single subshell grouping.
                        return None;
                    }
                }
                _ => {}
            }
        }

        if depth != 1 {
            // Unbalanced parens inside — not a valid subshell
            return None;
        }

        // Execute the inner commands (semicolon-separated) sequentially
        let inner = inner.trim();
        if inner.is_empty() {
            return Some(CommandResult::Success(0));
        }

        // The inner content is executed as a regular command line, which
        // handles `;`, `&&`, `||`, nested subshells, etc.
        Some(self.execute_command(inner))
    }

    /// Initialize the console device by creating a PTY pair.
    ///
    /// Creates a PTY master/slave pair via the PTY manager and optionally
    /// sets the terminal size. Shell output can then be routed through the
    /// PTY slave while falling back to serial when PTY is unavailable.
    ///
    /// Returns `Ok((master_id, slave_id))` on success.
    pub fn init_console(&self) -> Result<(u32, u32), crate::error::KernelError> {
        let (master_id, slave_id) = crate::fs::pty::with_pty_manager(|mgr| mgr.create_pty())
            .ok_or(crate::error::KernelError::InvalidState {
                expected: "pty_manager_initialized",
                actual: "pty_manager_not_available",
            })??;

        // Set default terminal size (80x24)
        crate::fs::pty::with_pty_manager(|mgr| {
            if let Some(master) = mgr.get_master(master_id) {
                master.set_winsize(crate::fs::pty::Winsize {
                    rows: 24,
                    cols: 80,
                    xpixel: 0,
                    ypixel: 0,
                });
            }
        });

        // Store the PTY IDs in the environment for child processes
        self.set_env(String::from("TTY"), format!("/dev/pts/{}", slave_id));

        crate::println!(
            "[shell] Console initialized: PTY master={}, slave={}",
            master_id,
            slave_id
        );
        Ok((master_id, slave_id))
    }

    /// Execute a pipeline of commands connected by pipes.
    fn execute_pipeline(&self, segments: &[&str]) -> CommandResult {
        if segments.len() < 2 {
            return self.execute_command(segments[0]);
        }

        // For kernel-space shell, we execute each segment and pipe data
        // through kernel pipe objects.
        let mut input_data: Option<Vec<u8>> = None;

        for (i, segment) in segments.iter().enumerate() {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }

            let is_last = i == segments.len() - 1;

            if is_last {
                // Last command in pipeline: if we have piped input, provide it
                // via stdin redirection (for builtins that support it).
                // For now, just execute the command normally.
                let result = self.execute_command(segment);
                return result;
            }

            // Create a pipe for this stage
            match crate::fs::pipe::create_pipe() {
                Ok((reader, writer)) => {
                    // Execute the command — for builtins, output goes to serial.
                    // We capture what we can via the pipe.
                    let _result = self.execute_command(segment);

                    // Close the writer end
                    writer.close();

                    // Drain the pipe for the next stage
                    input_data = Some(crate::fs::pipe::drain_pipe(&reader));
                }
                Err(_) => {
                    return CommandResult::Error("vsh: pipe creation failed".to_string());
                }
            }
        }

        CommandResult::Success(0)
    }

    /// Get current working directory
    pub fn get_cwd(&self) -> String {
        self.cwd.read().clone()
    }

    /// Set current working directory
    pub fn set_cwd(&self, path: String) -> Result<(), crate::error::KernelError> {
        // Verify directory exists using VFS
        let vfs = crate::fs::get_vfs().read();
        let node = vfs.resolve_path(&path)?;
        let metadata = node.metadata()?;

        if metadata.node_type != crate::fs::NodeType::Directory {
            return Err(crate::error::KernelError::FsError(
                crate::error::FsError::NotADirectory,
            ));
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

        // Package management
        builtins.insert("pkg".into(), Box::new(PkgCommand));

        // Shell commands
        builtins.insert("history".into(), Box::new(HistoryCommand));
        builtins.insert("clear".into(), Box::new(ClearCommand));
        builtins.insert("exit".into(), Box::new(ExitCommand));
        builtins.insert("logout".into(), Box::new(ExitCommand));

        // Utility commands
        builtins.insert("true".into(), Box::new(TrueCommand));
        builtins.insert("false".into(), Box::new(FalseCommand));
        builtins.insert("test".into(), Box::new(TestCommand));
        builtins.insert("[".into(), Box::new(BracketTestCommand));

        // Text processing commands
        builtins.insert("wc".into(), Box::new(WcCommand));
        builtins.insert("head".into(), Box::new(HeadCommand));
        builtins.insert("tail".into(), Box::new(TailCommand));
        builtins.insert("grep".into(), Box::new(GrepCommand));
        builtins.insert("sort".into(), Box::new(SortCommand));
        builtins.insert("uniq".into(), Box::new(UniqCommand));
        builtins.insert("cut".into(), Box::new(CutCommand));
        builtins.insert("tr".into(), Box::new(TrCommand));
        builtins.insert("tee".into(), Box::new(TeeCommand));
        builtins.insert("printf".into(), Box::new(PrintfCommand));

        // I/O commands
        builtins.insert("read".into(), Box::new(ReadCommand));

        // File management commands
        builtins.insert("cp".into(), Box::new(CpCommand));
        builtins.insert("mv".into(), Box::new(MvCommand));
        builtins.insert("chmod".into(), Box::new(ChmodCommand));

        // System information commands
        builtins.insert("date".into(), Box::new(DateCommand));
        builtins.insert("uname".into(), Box::new(UnameCommand));
        builtins.insert("free".into(), Box::new(FreeCommand));
        builtins.insert("dmesg".into(), Box::new(DmesgCommand));
        builtins.insert("df".into(), Box::new(DfCommand));
        builtins.insert("sync".into(), Box::new(SyncCommand));

        // Shell control commands
        builtins.insert("set".into(), Box::new(SetCommand));
        builtins.insert("source".into(), Box::new(SourceCommand));
        builtins.insert(".".into(), Box::new(DotCommand));
        builtins.insert("alias".into(), Box::new(AliasCommand));
        builtins.insert("unalias".into(), Box::new(UnaliasCommand));
        builtins.insert("type".into(), Box::new(TypeCommand));
        builtins.insert("which".into(), Box::new(WhichCommand));

        // Job control commands
        builtins.insert("fg".into(), Box::new(FgCommand));
        builtins.insert("bg".into(), Box::new(BgCommand));
        builtins.insert("jobs".into(), Box::new(JobsCommand));

        // Performance commands
        builtins.insert("perf".into(), Box::new(PerfCommand));
        builtins.insert("trace".into(), Box::new(TraceCommand));
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
                crate::println!("[SHELL] Found executable: {}", full_path);
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
                        crate::println!("[SHELL] Process {} created, about to run", pid.0);
                        // Run the user process directly like bootstrap does.
                        // This transfers control to Ring 3 via iretq and returns
                        // when the process exits via sys_exit.
                        let exit_code = run_user_process_from_shell(pid);
                        crate::println!("[SHELL] Process {} exited with code {}", pid.0, exit_code);
                        return CommandResult::Success(exit_code);
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
        let mut editor = self.line_editor.write();
        editor.reset();

        loop {
            let ch = Self::read_char();

            match ch {
                Some(byte) => {
                    let history = self.history.read();
                    let result = editor.feed(byte, &history);
                    drop(history);

                    match result {
                        Some(line_editor::EditResult::Done) => {
                            crate::print!("\n");
                            crate::graphics::fbcon::flush();
                            return editor.line();
                        }
                        Some(line_editor::EditResult::Cancel) => {
                            crate::print!("^C\n");
                            crate::graphics::fbcon::flush();
                            return String::new();
                        }
                        Some(line_editor::EditResult::Eof) => {
                            crate::print!("\n");
                            crate::graphics::fbcon::flush();
                            return String::from("exit");
                        }
                        Some(line_editor::EditResult::ClearScreen) => {
                            // Clear screen and redraw prompt + current line
                            crate::print!("\x1b[2J\x1b[H");
                            let _prompt = self.expand_prompt();
                            crate::print!("{}", _prompt);
                            let line = editor.line();
                            crate::print!("{}", line);
                            // Reposition cursor if not at end
                            let pos = editor.cursor_pos();
                            let len = editor.len();
                            if pos < len {
                                crate::print!("\x1b[{}D", len - pos);
                            }
                            crate::graphics::fbcon::flush();
                        }
                        Some(line_editor::EditResult::TabComplete) => {
                            let line = editor.line();
                            let cursor = editor.cursor_pos();

                            // Collect builtin names
                            let builtins_guard = self.builtins.read();
                            let builtin_names: Vec<&str> =
                                builtins_guard.keys().map(|s| s.as_str()).collect();

                            // Collect environment variable names
                            let env_guard = self.environment.read();
                            let env_names: Vec<&str> =
                                env_guard.keys().map(|s| s.as_str()).collect();

                            let cwd = self.get_cwd();

                            let candidates = completion::complete(
                                &line,
                                cursor,
                                &builtin_names,
                                &env_names,
                                &cwd,
                            );

                            drop(builtins_guard);
                            drop(env_guard);

                            if candidates.is_empty() {
                                // No matches — audible bell
                                crate::print!("\x07");
                            } else {
                                // Find start of the word being completed
                                let before = &line[..cursor.min(line.len())];
                                let word_start = if before.ends_with(' ') || before.is_empty() {
                                    cursor
                                } else {
                                    before.rfind(' ').map_or(0, |p| p + 1)
                                };

                                if candidates.len() == 1 {
                                    // Single match — replace word directly
                                    let mut replacement = candidates[0].clone();
                                    if !replacement.ends_with('/') {
                                        replacement.push(' ');
                                    }
                                    editor.replace_word(word_start, &replacement);
                                } else {
                                    // Multiple matches — show candidates, insert
                                    // longest common prefix
                                    let lcp = completion::longest_common_prefix(&candidates);

                                    // Extend word to lcp if it's longer
                                    let current_word_len = cursor - word_start;
                                    if lcp.len() > current_word_len {
                                        editor.replace_word_silent(word_start, &lcp);
                                    }

                                    // Show candidates
                                    crate::print!("\n");
                                    for candidate in &candidates {
                                        crate::print!("{}  ", candidate);
                                    }
                                    crate::print!("\n");

                                    // Redraw prompt + current line
                                    let prompt = self.expand_prompt();
                                    crate::print!("{}", prompt);
                                    let updated_line = editor.line();
                                    crate::print!("{}", updated_line);
                                    let pos = editor.cursor_pos();
                                    let len = editor.len();
                                    if pos < len {
                                        crate::print!("\x1b[{}D", len - pos);
                                    }
                                }
                            }
                            crate::graphics::fbcon::flush();
                        }
                        Some(line_editor::EditResult::Suspend) => {
                            crate::print!("^Z\n");
                            crate::graphics::fbcon::flush();
                            // Attempt to suspend the foreground job. In the
                            // kernel shell there is no true foreground process
                            // to SIGTSTP, so we print the indicator and return
                            // an empty line (matching bash behavior when there
                            // is nothing to suspend).
                            self.suspend_foreground_job();
                            // Redraw prompt — the caller will see an empty
                            // command and re-prompt.
                            return String::new();
                        }
                        Some(line_editor::EditResult::Continue) => {
                            let row = crate::graphics::fbcon::cursor_row();
                            crate::graphics::fbcon::flush_row(row);
                            crate::graphics::fbcon::update_cursor();
                        }
                        None => {
                            // Incomplete escape sequence (e.g., first byte of
                            // arrow key).
                            // No visible change yet — skip MMIO update.
                        }
                    }
                }
                None => {
                    // No input available — yield CPU briefly.
                    // We use spin_loop() rather than hlt/wfi because the APIC
                    // is initialized and takes over interrupt routing from the
                    // PIC, so PIC-based timer/keyboard IRQs may not fire.
                    // Input is polled from serial + keyboard ring buffer.
                    // Multiple iterations (~1us delay) reduces idle CPU usage
                    // and gives QEMU's display thread more time to render.
                    for _ in 0..256 {
                        core::hint::spin_loop();
                    }
                }
            }
        }
    }

    /// Read a single byte from any available input source (keyboard + serial).
    fn read_char() -> Option<u8> {
        crate::drivers::input::read_char()
    }

    /// Handle a signal delivered to the shell or its foreground job.
    ///
    /// Dispatches to the appropriate handler based on the signal number.
    /// Uses POSIX signal constants from `crate::process::exit::signals`.
    #[allow(dead_code)] // Signal handling -- wired when job control is fully active
    fn handle_signal(&self, signum: i32) {
        use crate::process::exit::signals;

        match signum {
            signals::SIGINT => {
                // Interrupt: if there is a foreground job, send SIGINT to it.
                // Otherwise, just cancel the current input line (handled in
                // read_line via EditResult::Cancel).
                if let Some(job) = self.job_table.read().current_job() {
                    if job.is_running() {
                        for &pid in &job.pids {
                            let _ = crate::process::exit::kill_process(
                                crate::process::ProcessId(pid),
                                signals::SIGINT,
                            );
                        }
                    }
                }
            }
            signals::SIGTSTP => {
                // Terminal stop: suspend the foreground job.
                self.suspend_foreground_job();
            }
            signals::SIGCHLD => {
                // Child status changed: reap completed jobs and notify.
                self.notify_completed_jobs();
            }
            signals::SIGCONT => {
                // Continue: nothing special for the shell itself.
            }
            _ => {}
        }
    }

    /// Suspend the current foreground job (if any) by sending SIGTSTP to
    /// all of its processes and marking it as Stopped in the job table.
    fn suspend_foreground_job(&self) {
        use crate::process::exit::signals;

        let mut job_table = self.job_table.write();
        // The "current" job is the most recently added one.
        if let Some(job) = job_table.current_job() {
            if job.is_running() {
                let job_id = job.job_id;
                let _cmd = job.command_line.clone();

                // Send SIGTSTP to every process in the job's pipeline.
                for &pid in &job.pids {
                    let _ = crate::process::exit::kill_process(
                        crate::process::ProcessId(pid),
                        signals::SIGTSTP,
                    );
                }

                // Update the job table entry.
                job_table.update_status(job_id, jobs::JobStatus::Stopped);
                crate::println!("[{}]+  Stopped                 {}", job_id, _cmd);
            }
        }
        // If there is no running foreground job, Ctrl-Z is a no-op (bash
        // prints nothing and returns to the prompt).
    }

    /// Reap completed background jobs and print notifications.
    ///
    /// Called once per REPL iteration so the user sees "[N]+ Done ..." lines
    /// immediately before the next prompt, matching bash/zsh behavior.
    fn notify_completed_jobs(&self) {
        let reaped = self.job_table.write().reap_done();
        for job in &reaped {
            crate::println!(
                "[{}]+  Done                    {}",
                job.job_id,
                job.command_line
            );
        }
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

/// Run a user process directly from the shell, similar to bootstrap's approach.
///
/// This function transfers control to Ring 3 via iretq and returns when the
/// process exits. Unlike wait_for_child(), this doesn't block the shell's
/// process context or depend on scheduler wakeup semantics.
///
/// Returns the process's exit code.
fn run_user_process_from_shell(pid: crate::process::ProcessId) -> i32 {
    use crate::process::get_process;

    crate::println!("[SHELL] run_user_process_from_shell: pid={}", pid.0);

    let process = match get_process(pid) {
        Some(p) => p,
        None => {
            crate::println!("[SHELL] Error: Process {} not found", pid.0);
            return 1;
        }
    };

    // Architecture-specific user-mode entry
    #[cfg(target_arch = "x86_64")]
    {
        // Get the process's page table root (physical address for CR3)
        let vas = process.memory_space.lock();
        let pt_root = vas.get_page_table();
        if pt_root == 0 {
            drop(vas);
            crate::println!("[SHELL] Error: Process {} has no page table", pid.0);
            return 1;
        }

        // Get entry point and user stack from the process's first thread
        let threads = process.threads.lock();
        let thread = match threads.values().next() {
            Some(t) => t,
            None => {
                drop(threads);
                drop(vas);
                crate::println!("[SHELL] Error: Process {} has no threads", pid.0);
                return 1;
            }
        };

        let (entry_point, user_stack_ptr) = {
            use crate::arch::context::ThreadContext;
            let ctx = thread.context.lock();
            (
                ctx.get_instruction_pointer() as u64,
                ctx.get_stack_pointer() as u64,
            )
        };

        // Drop locks before entering user mode
        drop(threads);
        drop(vas);

        crate::println!(
            "[SHELL] Entering Ring 3: entry={:#x} stack={:#x}",
            entry_point,
            user_stack_ptr
        );

        // User CS and SS selectors (Ring 3)
        let user_cs: u64 = 0x33; // GDT index 6, RPL 3
        let user_ss: u64 = 0x2B; // GDT index 5, RPL 3

        // Enter Ring 3 via iretq with returnable context.
        // When the user process calls sys_exit, this call returns normally
        // with the exit code.
        //
        // SAFETY: All preconditions for enter_usermode_returnable are met:
        // - entry_point is in the process's user-space page tables
        // - user_stack_ptr points to the top of the user stack
        // - CS/SS are valid Ring 3 selectors from the GDT
        // - pt_root is a valid L4 page table with kernel mappings preserved
        // - kernel_rsp_ptr points to the per-CPU kernel_rsp field
        let kernel_rsp_ptr = crate::arch::x86_64::syscall::per_cpu_data_ptr() as u64;
        unsafe {
            crate::arch::x86_64::usermode::enter_usermode_returnable(
                entry_point,
                user_stack_ptr,
                user_cs,
                user_ss,
                pt_root,
                kernel_rsp_ptr,
            );
        }

        crate::println!("[SHELL] Returned from Ring 3");

        // When we return here, the process has exited. Get the exit code.
        match get_process(pid) {
            Some(p) => p.get_exit_code(),
            None => 0, // Process was cleaned up, assume successful exit
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        // For AArch64 and RISC-V, user-mode entry is not yet implemented in the shell.
        // Fall back to yielding and waiting (this will still freeze, but at least
        // it compiles and shows what needs to be done).
        crate::println!(
            "[SHELL] Warning: Direct user-mode entry not implemented on this architecture"
        );
        crate::println!("[SHELL] Attempting scheduler-based wait (may freeze)...");

        crate::sched::yield_cpu();

        match crate::process::wait_for_child(Some(pid)) {
            Ok((_child_pid, exit_code)) => exit_code,
            Err(e) => {
                crate::println!("[SHELL] Error waiting for process: {:?}", e);
                1
            }
        }
    }
}

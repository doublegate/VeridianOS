//! vsh -- VeridianOS Shell
//!
//! A user-space shell with Bash 5.3 feature parity, running as a Ring 3
//! `no_std` binary on VeridianOS.  Uses raw syscalls via inline assembly.

#![no_std]
#![no_main]

extern crate alloc;

mod builtin;
mod config;
mod error;
mod exec;
mod expand;
mod input;
mod jobs;
mod lexer;
mod output;
mod parser;
mod prompt;
mod readline;
mod syscall;
mod var;

use alloc::{collections::BTreeMap, string::String};
use core::{
    alloc::{GlobalAlloc, Layout},
    sync::atomic::{AtomicUsize, Ordering},
};

use config::ShellConfig;
use error::VshError;
use expand::parameter::SpecialVars;
use jobs::JobTable;
use prompt::PromptContext;
use readline::Readline;
use var::ShellEnv;

// ============================================================================
// Global allocator (mmap-based)
// ============================================================================

/// A simple bump allocator backed by anonymous mmap pages.
struct MmapAllocator {
    base: AtomicUsize,
    offset: AtomicUsize,
    capacity: AtomicUsize,
}

unsafe impl GlobalAlloc for MmapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();

        loop {
            let base = self.base.load(Ordering::SeqCst);
            let off = self.offset.load(Ordering::SeqCst);
            let cap = self.capacity.load(Ordering::SeqCst);

            if base == 0 || off + size + align > cap {
                // Need a new arena
                let arena_size = if size + align > 256 * 1024 {
                    // Large allocation: round up to page boundary
                    (size + align + 4095) & !4095
                } else {
                    256 * 1024 // 256 KB default arena
                };

                let ptr = syscall::sys_mmap(
                    0,
                    arena_size,
                    syscall::PROT_READ | syscall::PROT_WRITE,
                    syscall::MAP_PRIVATE | syscall::MAP_ANONYMOUS,
                );

                if ptr <= 0 {
                    return core::ptr::null_mut();
                }

                let new_base = ptr as usize;
                let aligned_off = (align - 1) & !(align - 1);

                self.base.store(new_base, Ordering::SeqCst);
                self.offset.store(aligned_off + size, Ordering::SeqCst);
                self.capacity.store(arena_size, Ordering::SeqCst);

                return (new_base + aligned_off) as *mut u8;
            }

            let aligned_off = (off + align - 1) & !(align - 1);
            if aligned_off + size > cap {
                // Try again -- will allocate new arena
                self.offset.store(cap + 1, Ordering::SeqCst);
                continue;
            }

            if self
                .offset
                .compare_exchange(off, aligned_off + size, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return (base + aligned_off) as *mut u8;
            }
            // CAS failed -- retry
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator: no individual deallocation.
        // Memory is reclaimed when the process exits.
    }
}

#[global_allocator]
static ALLOCATOR: MmapAllocator = MmapAllocator {
    base: AtomicUsize::new(0),
    offset: AtomicUsize::new(0),
    capacity: AtomicUsize::new(0),
};

// ============================================================================
// Panic handler
// ============================================================================

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let err = output::Writer::stderr();
    err.write_str("vsh: panic: ");
    if let Some(msg) = info.message().as_str() {
        err.write_str(msg);
    }
    err.write_str("\n");
    syscall::sys_exit(127)
}

// ============================================================================
// Shell state
// ============================================================================

/// Top-level shell state, passed to the executor and builtins.
pub struct Shell {
    /// Variable/environment manager.
    pub env: ShellEnv,
    /// Shell configuration (set -o, shopt).
    pub config: ShellConfig,
    /// Job table.
    pub jobs: JobTable,
    /// Line editor with history.
    pub readline: Readline,
    /// Whether the shell is interactive.
    pub interactive: bool,
    /// Whether the shell should keep running.
    pub running: bool,
}

impl Shell {
    /// Create a new interactive shell.
    fn new_interactive() -> Self {
        let pid = syscall::sys_getpid();
        let mut env = ShellEnv::new();
        env.shell_pid = pid;

        // Set up default environment
        let _ = env.set_global("SHELL", "/bin/vsh");

        // Read current working directory
        let mut cwd_buf = [0u8; 512];
        let cwd_len = syscall::sys_getcwd(&mut cwd_buf);
        if cwd_len > 0 {
            if let Ok(cwd_str) = core::str::from_utf8(&cwd_buf[..cwd_len as usize]) {
                let trimmed = cwd_str.trim_end_matches('\0');
                let _ = env.set_global("PWD", trimmed);
            }
        }

        // Default PS1
        let _ = env.set_global("PS1", &prompt::default_ps1());

        Shell {
            env,
            config: ShellConfig::interactive(),
            jobs: JobTable::new(),
            readline: Readline::new(),
            interactive: true,
            running: true,
        }
    }

    /// Build the prompt context from current environment state.
    fn prompt_context(&self) -> PromptContext<'_> {
        let user = self.env.get_str("USER");
        let hostname = self.env.get_str("HOSTNAME");
        let cwd = self.env.get_str("PWD");
        let home = self.env.get_str("HOME");
        // Use static references -- safe because ShellEnv lives on the stack
        // for the duration of the shell.  We copy into owned strings for
        // the PromptContext.
        PromptContext {
            user: if user.is_empty() { "user" } else { user },
            hostname: if hostname.is_empty() {
                "veridian"
            } else {
                hostname
            },
            cwd: if cwd.is_empty() { "/" } else { cwd },
            home: if home.is_empty() { "/" } else { home },
            is_root: self.env.get_str("EUID") == "0",
            shell_name: "vsh",
        }
    }

    /// Build the SpecialVars snapshot for the expansion engine.
    pub fn special_vars(&self) -> SpecialVars {
        SpecialVars {
            exit_status: self.env.last_status,
            pid: self.env.shell_pid as u32,
            last_bg_pid: self.env.last_bg_pid as u32,
            argc: self.env.positional.len(),
            arg0: self.env.arg0.clone(),
            flags: self.env.option_flags.clone(),
            last_arg: self.env.last_arg.clone(),
            positional: self.env.positional.clone(),
        }
    }

    /// Collect variables as a flat BTreeMap for the expansion engine.
    pub fn vars_map(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for name in self.env.all_var_names() {
            let val = self.env.get_str(&name);
            map.insert(name, String::from(val));
        }
        map
    }

    /// Update the PWD variable after a chdir.
    pub fn update_cwd(&mut self) {
        let mut buf = [0u8; 512];
        let n = syscall::sys_getcwd(&mut buf);
        if n > 0 {
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                let trimmed = s.trim_end_matches('\0');
                let old = String::from(self.env.get_str("PWD"));
                let _ = self.env.set_global("OLDPWD", &old);
                let _ = self.env.set_global("PWD", trimmed);
            }
        }
    }
}

// ============================================================================
// Main REPL
// ============================================================================

fn run_interactive(shell: &mut Shell) {
    while shell.running {
        // Report any background job completions
        shell.jobs.update_status();
        let messages = shell.jobs.report_and_clean();
        for msg in &messages {
            println!("{}", msg);
        }

        // Build prompt
        let ps1 = String::from(shell.env.get_str("PS1"));
        let ps1_text = if ps1.is_empty() {
            String::from("$ ")
        } else {
            let ctx = shell.prompt_context();
            prompt::expand_prompt(&ps1, &ctx)
        };

        // Read a line
        let line = match shell.readline.readline(&ps1_text) {
            Some(line) => line,
            None => {
                // EOF
                println!("exit");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Add to history
        shell.readline.history.add(trimmed);

        // Execute
        match exec::eval::eval_string(shell, trimmed) {
            Ok(status) => {
                shell.env.last_status = status;
            }
            Err(VshError::Exit(code)) => {
                shell.env.last_status = code;
                shell.running = false;
            }
            Err(e) => {
                eprintln!("vsh: {}", e);
                shell.env.last_status = 1;
            }
        }
    }
}

#[allow(dead_code)] // Used when argv parsing is implemented
fn run_script(shell: &mut Shell, path: &str, args: &[String]) {
    shell.interactive = false;
    shell.config.interactive = false;
    shell.env.arg0 = String::from(path);
    shell.env.positional = args.to_vec();

    match exec::script::run_script_file(shell, path) {
        Ok(status) => {
            shell.env.last_status = status;
        }
        Err(VshError::Exit(code)) => {
            shell.env.last_status = code;
        }
        Err(e) => {
            eprintln!("vsh: {}: {}", path, e);
            shell.env.last_status = 1;
        }
    }
}

#[allow(dead_code)] // Used when argv parsing is implemented (-c flag)
fn run_command_string(shell: &mut Shell, cmd: &str) {
    shell.interactive = false;
    shell.config.interactive = false;

    match exec::eval::eval_string(shell, cmd) {
        Ok(status) => {
            shell.env.last_status = status;
        }
        Err(VshError::Exit(code)) => {
            shell.env.last_status = code;
        }
        Err(e) => {
            eprintln!("vsh: {}", e);
            shell.env.last_status = 1;
        }
    }
}

// ============================================================================
// Entry point
// ============================================================================

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut shell = Shell::new_interactive();

    // Parse arguments from argv (passed on the stack by the kernel).
    // For now, start in interactive mode.
    // TODO: read argc/argv from the stack or via a syscall.

    run_interactive(&mut shell);
    syscall::sys_exit(shell.env.last_status)
}

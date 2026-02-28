//! Process operations for VeridianOS.
//!
//! Provides both low-level syscall wrappers and higher-level types:
//!
//! - Low-level: `exit`, `fork`, `execve`, `waitpid`, `getpid`, `getppid`,
//!   `getcwd`, `chdir`, `kill`, `sched_yield`
//! - High-level: `Command` builder, `Child`, `ExitStatus`
//!
//! Syscall mappings:
//! - `exit`    -> SYS_PROCESS_EXIT (11)
//! - `fork`    -> SYS_PROCESS_FORK (12)
//! - `exec`    -> SYS_PROCESS_EXEC (13)
//! - `waitpid` -> SYS_PROCESS_WAIT (14)
//! - `getpid`  -> SYS_PROCESS_GETPID (15)
//! - `getppid` -> SYS_PROCESS_GETPPID (16)
//! - `getcwd`  -> SYS_PROCESS_GETCWD (110)
//! - `chdir`   -> SYS_PROCESS_CHDIR (111)
//! - `kill`    -> SYS_PROCESS_KILL (113)

extern crate alloc;
use alloc::vec::Vec;

use super::{
    fd::OwnedFd,
    io::AnonPipe,
    path::{OsStr, OsString, Path, PathBuf},
    syscall0, syscall1, syscall2, syscall3, syscall_result, SyscallError, SYS_PROCESS_CHDIR,
    SYS_PROCESS_EXEC, SYS_PROCESS_EXIT, SYS_PROCESS_FORK, SYS_PROCESS_GETCWD, SYS_PROCESS_GETPID,
    SYS_PROCESS_GETPPID, SYS_PROCESS_KILL, SYS_PROCESS_WAIT, SYS_PROCESS_YIELD,
};

// ============================================================================
// Low-level syscall wrappers (preserved from original API)
// ============================================================================

/// Exit the current process with the given status code.
///
/// This function never returns.
pub fn exit(status: i32) -> ! {
    unsafe {
        syscall1(SYS_PROCESS_EXIT, status as usize);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Fork the current process.
///
/// Returns child PID in the parent, 0 in the child.
pub fn fork() -> Result<usize, SyscallError> {
    let ret = unsafe { syscall0(SYS_PROCESS_FORK) };
    syscall_result(ret)
}

/// Replace the current process image with a new program.
///
/// On success, this function does not return.
pub fn execve(
    path: *const u8,
    argv: *const *const u8,
    envp: *const *const u8,
) -> Result<usize, SyscallError> {
    let ret = unsafe {
        syscall3(
            SYS_PROCESS_EXEC,
            path as usize,
            argv as usize,
            envp as usize,
        )
    };
    syscall_result(ret)
}

/// Wait for a child process to change state.
pub fn waitpid(pid: isize, wstatus: *mut i32, options: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall3(SYS_PROCESS_WAIT, pid as usize, wstatus as usize, options) };
    syscall_result(ret)
}

/// Get the current process ID.
pub fn getpid() -> usize {
    unsafe { syscall0(SYS_PROCESS_GETPID) as usize }
}

/// Get the parent process ID.
pub fn getppid() -> usize {
    unsafe { syscall0(SYS_PROCESS_GETPPID) as usize }
}

/// Yield the CPU to another process.
pub fn sched_yield() -> Result<usize, SyscallError> {
    let ret = unsafe { syscall0(SYS_PROCESS_YIELD) };
    syscall_result(ret)
}

/// Get the current working directory.
pub fn getcwd(buf: *mut u8, size: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_PROCESS_GETCWD, buf as usize, size) };
    syscall_result(ret)
}

/// Change the current working directory.
pub fn chdir(path: *const u8) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall1(SYS_PROCESS_CHDIR, path as usize) };
    syscall_result(ret)
}

/// Send a signal to a process.
pub fn kill(pid: usize, sig: usize) -> Result<usize, SyscallError> {
    let ret = unsafe { syscall2(SYS_PROCESS_KILL, pid, sig) };
    syscall_result(ret)
}

// ============================================================================
// High-level: current_dir / set_current_dir
// ============================================================================

/// Get the current working directory as a `PathBuf`.
pub fn current_dir() -> Result<PathBuf, SyscallError> {
    let mut buf = [0u8; 4096];
    let len = getcwd(buf.as_mut_ptr(), buf.len())?;
    Ok(PathBuf::from_vec(buf[..len].to_vec()))
}

/// Set the current working directory.
pub fn set_current_dir(path: &Path) -> Result<(), SyscallError> {
    let c_path = path.to_cstring();
    chdir(c_path.as_ptr())?;
    Ok(())
}

// ============================================================================
// Signal constants
// ============================================================================

pub const SIGTERM: usize = 15;
pub const SIGKILL: usize = 9;
pub const SIGINT: usize = 2;
pub const SIGHUP: usize = 1;
pub const SIGCHLD: usize = 17;
pub const SIGPIPE: usize = 13;

// ============================================================================
// Wait options
// ============================================================================

/// Do not block if no child has exited.
pub const WNOHANG: usize = 1;
/// Also report stopped (not traced) children.
pub const WUNTRACED: usize = 2;

// ============================================================================
// ExitStatus
// ============================================================================

/// The exit status of a completed child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    /// Raw wait status from `waitpid`.
    raw: i32,
}

impl ExitStatus {
    /// Create from a raw wait status value.
    pub fn from_raw(raw: i32) -> Self {
        ExitStatus { raw }
    }

    /// Did the process exit successfully (exit code 0)?
    pub fn success(&self) -> bool {
        self.code() == Some(0)
    }

    /// Get the exit code, if the process exited normally.
    ///
    /// Returns `None` if the process was killed by a signal.
    pub fn code(&self) -> Option<i32> {
        // Standard encoding: if low 7 bits == 0, bits 8-15 are exit code.
        if self.raw & 0x7f == 0 {
            Some((self.raw >> 8) & 0xff)
        } else {
            None
        }
    }

    /// Get the signal that killed the process, if any.
    pub fn signal(&self) -> Option<i32> {
        let sig = self.raw & 0x7f;
        if sig != 0 && sig != 0x7f {
            Some(sig)
        } else {
            None
        }
    }

    /// Return the raw status value.
    pub fn raw(&self) -> i32 {
        self.raw
    }
}

impl core::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(code) = self.code() {
            write!(f, "exit code: {}", code)
        } else if let Some(sig) = self.signal() {
            write!(f, "killed by signal {}", sig)
        } else {
            write!(f, "unknown status: {}", self.raw)
        }
    }
}

// ============================================================================
// Stdio configuration for Command
// ============================================================================

/// How to configure a child process stdio stream.
#[derive(Debug)]
pub enum Stdio {
    /// Inherit the parent's fd.
    Inherit,
    /// Connect via a pipe.
    Piped,
    /// Discard output (open /dev/null equivalent).
    Null,
    /// Use a specific fd (caller-owned).
    Fd(usize),
}

// ============================================================================
// Child
// ============================================================================

/// A running or completed child process.
pub struct Child {
    /// The child's process ID.
    pid: usize,
    /// Pipe connected to the child's stdin (if piped).
    pub stdin: Option<OwnedFd>,
    /// Pipe connected to the child's stdout (if piped).
    pub stdout: Option<OwnedFd>,
    /// Pipe connected to the child's stderr (if piped).
    pub stderr: Option<OwnedFd>,
}

impl Child {
    /// Get the child's PID.
    pub fn id(&self) -> usize {
        self.pid
    }

    /// Wait for the child to exit and return its status.
    pub fn wait(&mut self) -> Result<ExitStatus, SyscallError> {
        // Close our end of stdin pipe so the child sees EOF.
        if let Some(fd) = self.stdin.take() {
            drop(fd);
        }
        let mut wstatus: i32 = 0;
        waitpid(self.pid as isize, &mut wstatus, 0)?;
        Ok(ExitStatus::from_raw(wstatus))
    }

    /// Check if the child has exited without blocking.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, SyscallError> {
        let mut wstatus: i32 = 0;
        match waitpid(self.pid as isize, &mut wstatus, WNOHANG) {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(ExitStatus::from_raw(wstatus))),
            Err(e) => Err(e),
        }
    }

    /// Send SIGKILL to the child.
    pub fn kill(&self) -> Result<(), SyscallError> {
        kill(self.pid, SIGKILL)?;
        Ok(())
    }
}

impl core::fmt::Debug for Child {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Child")
            .field("pid", &self.pid)
            .field("stdin", &self.stdin)
            .field("stdout", &self.stdout)
            .field("stderr", &self.stderr)
            .finish()
    }
}

// ============================================================================
// Command
// ============================================================================

/// A builder for spawning child processes.
///
/// Mirrors `std::process::Command`.
pub struct Command {
    /// Program path.
    program: OsString,
    /// Arguments (including argv[0] = program).
    args: Vec<OsString>,
    /// Environment variables (KEY=VALUE).
    env: Option<Vec<OsString>>,
    /// Working directory for the child.
    cwd: Option<PathBuf>,
    /// Stdin configuration.
    stdin_cfg: Stdio,
    /// Stdout configuration.
    stdout_cfg: Stdio,
    /// Stderr configuration.
    stderr_cfg: Stdio,
}

impl Command {
    /// Create a new command for the given program.
    pub fn new(program: &OsStr) -> Self {
        Command {
            program: program.to_os_string(),
            args: Vec::new(),
            env: None,
            cwd: None,
            stdin_cfg: Stdio::Inherit,
            stdout_cfg: Stdio::Inherit,
            stderr_cfg: Stdio::Inherit,
        }
    }

    /// Create from a string path.
    pub fn new_str(program: &str) -> Self {
        Self::new(OsStr::new(program))
    }

    /// Add an argument.
    pub fn arg(&mut self, arg: &OsStr) -> &mut Self {
        self.args.push(arg.to_os_string());
        self
    }

    /// Add an argument from a string.
    pub fn arg_str(&mut self, arg: &str) -> &mut Self {
        self.arg(OsStr::new(arg))
    }

    /// Add multiple arguments.
    pub fn args(&mut self, args: &[&OsStr]) -> &mut Self {
        for a in args {
            self.args.push(a.to_os_string());
        }
        self
    }

    /// Set an environment variable.
    pub fn env(&mut self, key: &OsStr, val: &OsStr) -> &mut Self {
        let env = self.env.get_or_insert_with(Vec::new);
        let mut entry = key.to_os_string();
        entry.push(OsStr::from_bytes(b"="));
        entry.push(val);
        env.push(entry);
        self
    }

    /// Set an environment variable from strings.
    pub fn env_str(&mut self, key: &str, val: &str) -> &mut Self {
        self.env(OsStr::new(key), OsStr::new(val))
    }

    /// Clear all environment variables for the child.
    pub fn env_clear(&mut self) -> &mut Self {
        self.env = Some(Vec::new());
        self
    }

    /// Set the working directory for the child.
    pub fn current_dir(&mut self, dir: &Path) -> &mut Self {
        self.cwd = Some(dir.to_path_buf());
        self
    }

    /// Configure stdin.
    pub fn stdin(&mut self, cfg: Stdio) -> &mut Self {
        self.stdin_cfg = cfg;
        self
    }

    /// Configure stdout.
    pub fn stdout(&mut self, cfg: Stdio) -> &mut Self {
        self.stdout_cfg = cfg;
        self
    }

    /// Configure stderr.
    pub fn stderr(&mut self, cfg: Stdio) -> &mut Self {
        self.stderr_cfg = cfg;
        self
    }

    /// Spawn the child process.
    pub fn spawn(&self) -> Result<Child, SyscallError> {
        // Build null-terminated program path.
        let mut prog_c = self.program.as_bytes().to_vec();
        prog_c.push(0);

        // Build argv: [program, args..., NULL]
        let mut argv_storage: Vec<Vec<u8>> = Vec::new();
        argv_storage.push(prog_c.clone());
        for a in &self.args {
            let mut s = a.as_bytes().to_vec();
            s.push(0);
            argv_storage.push(s);
        }
        let mut argv_ptrs: Vec<*const u8> = Vec::with_capacity(argv_storage.len() + 1);
        for s in &argv_storage {
            argv_ptrs.push(s.as_ptr());
        }
        argv_ptrs.push(core::ptr::null());

        // Build envp.
        let mut envp_storage: Vec<Vec<u8>> = Vec::new();
        let mut envp_ptrs: Vec<*const u8> = Vec::new();
        if let Some(env) = &self.env {
            for e in env {
                let mut s = e.as_bytes().to_vec();
                s.push(0);
                envp_storage.push(s);
            }
            for s in &envp_storage {
                envp_ptrs.push(s.as_ptr());
            }
        }
        envp_ptrs.push(core::ptr::null());

        // Create pipes if needed.
        let stdin_pipe = match &self.stdin_cfg {
            Stdio::Piped => Some(AnonPipe::new()?),
            _ => None,
        };
        let stdout_pipe = match &self.stdout_cfg {
            Stdio::Piped => Some(AnonPipe::new()?),
            _ => None,
        };
        let stderr_pipe = match &self.stderr_cfg {
            Stdio::Piped => Some(AnonPipe::new()?),
            _ => None,
        };

        // Fork.
        let child_pid = fork()?;
        if child_pid == 0 {
            // --- Child process ---
            // Set up stdio redirections.
            if let Some(ref pipe) = stdin_pipe {
                let _ = super::fs::dup2(pipe.read_fd(), 0);
                let _ = super::fs::close(pipe.write_fd());
                let _ = super::fs::close(pipe.read_fd());
            }
            if let Some(ref pipe) = stdout_pipe {
                let _ = super::fs::dup2(pipe.write_fd(), 1);
                let _ = super::fs::close(pipe.read_fd());
                let _ = super::fs::close(pipe.write_fd());
            }
            if let Some(ref pipe) = stderr_pipe {
                let _ = super::fs::dup2(pipe.write_fd(), 2);
                let _ = super::fs::close(pipe.read_fd());
                let _ = super::fs::close(pipe.write_fd());
            }

            // Change directory if requested.
            if let Some(ref dir) = self.cwd {
                let c_dir = dir.to_cstring();
                let _ = chdir(c_dir.as_ptr());
            }

            // Exec.
            let _ = execve(prog_c.as_ptr(), argv_ptrs.as_ptr(), envp_ptrs.as_ptr());

            // If exec failed, exit with 127 (command not found convention).
            exit(127);
        }

        // --- Parent process ---
        // Close the child's end of pipes and return parent's end.
        let parent_stdin = stdin_pipe.map(|mut p| p.take_write_fd());
        let parent_stdout = stdout_pipe.map(|mut p| p.take_read_fd());
        let parent_stderr = stderr_pipe.map(|mut p| p.take_read_fd());

        Ok(Child {
            pid: child_pid,
            stdin: parent_stdin,
            stdout: parent_stdout,
            stderr: parent_stderr,
        })
    }

    /// Spawn the child and wait for it to complete, returning its exit status.
    pub fn status(&self) -> Result<ExitStatus, SyscallError> {
        let mut child = self.spawn()?;
        child.wait()
    }

    /// Spawn the child, wait for it to complete, and capture its stdout.
    pub fn output(&self) -> Result<Output, SyscallError> {
        let mut cmd = Command::new(self.program.as_os_str());
        for a in &self.args {
            cmd.arg(a.as_os_str());
        }
        if let Some(ref env) = self.env {
            cmd.env = Some(env.clone());
        }
        cmd.cwd = self.cwd.clone();
        cmd.stdin(Stdio::Inherit);
        cmd.stdout(Stdio::Piped);
        cmd.stderr(Stdio::Piped);

        let mut child = cmd.spawn()?;

        // Read stdout and stderr.
        let mut stdout_buf = Vec::new();
        if let Some(ref fd) = child.stdout {
            let mut tmp = [0u8; 4096];
            loop {
                let n = fd.read(&mut tmp)?;
                if n == 0 {
                    break;
                }
                stdout_buf.extend_from_slice(&tmp[..n]);
            }
        }

        let mut stderr_buf = Vec::new();
        if let Some(ref fd) = child.stderr {
            let mut tmp = [0u8; 4096];
            loop {
                let n = fd.read(&mut tmp)?;
                if n == 0 {
                    break;
                }
                stderr_buf.extend_from_slice(&tmp[..n]);
            }
        }

        let status = child.wait()?;

        Ok(Output {
            status,
            stdout: stdout_buf,
            stderr: stderr_buf,
        })
    }
}

impl core::fmt::Debug for Command {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Command")
            .field("program", &self.program)
            .field("args", &self.args)
            .finish()
    }
}

// ============================================================================
// Output
// ============================================================================

/// The output of a finished child process (status + captured stdout/stderr).
#[derive(Debug)]
pub struct Output {
    /// Exit status.
    pub status: ExitStatus,
    /// Captured stdout bytes.
    pub stdout: Vec<u8>,
    /// Captured stderr bytes.
    pub stderr: Vec<u8>,
}

// ============================================================================
// Process info
// ============================================================================

/// Get the current process ID (typed).
pub fn id() -> u32 {
    getpid() as u32
}

/// Get the current user ID.
pub fn getuid() -> u32 {
    super::os::getuid() as u32
}

/// Get the current group ID.
pub fn getgid() -> u32 {
    super::os::getgid() as u32
}

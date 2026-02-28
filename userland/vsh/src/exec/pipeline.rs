//! Pipeline execution.
//!
//! Connects multiple commands with pipes: `cmd1 | cmd2 | cmd3`.
//! Each command in the pipeline runs in its own forked process (except
//! optionally the last command with `lastpipe` shopt).

extern crate alloc;

use alloc::vec::Vec;

use crate::{
    error::{Result, VshError},
    parser::ast::Command,
    syscall, Shell,
};

/// Execute a multi-command pipeline.
///
/// Creates pipes between adjacent commands, forks processes for each
/// command, and waits for all to complete.  Returns the exit status of
/// the last command in the pipeline.
pub fn execute_pipeline_multi(
    shell: &mut Shell,
    commands: &[Command],
    _pipe_stderr: bool,
) -> Result<i32> {
    let n = commands.len();
    if n == 0 {
        return Ok(0);
    }
    if n == 1 {
        return super::execute_command(shell, &commands[0]);
    }

    // Create n-1 pipes
    let mut pipes: Vec<[i32; 2]> = Vec::with_capacity(n - 1);
    for _ in 0..n - 1 {
        let mut pipefd = [0i32; 2];
        let ret = syscall::sys_pipe(&mut pipefd);
        if ret < 0 {
            // Close any pipes we already created
            for p in &pipes {
                syscall::sys_close(p[0]);
                syscall::sys_close(p[1]);
            }
            return Err(VshError::PipeFailed);
        }
        pipes.push(pipefd);
    }

    // Fork children for each command
    let mut child_pids: Vec<i32> = Vec::with_capacity(n);

    for i in 0..n {
        let pid = syscall::sys_fork();
        if pid < 0 {
            // Fork failed -- kill any children we already started
            for &cpid in &child_pids {
                let _ = unsafe { syscall::syscall2(syscall::SYS_PROCESS_KILL, cpid as usize, 9) };
            }
            for p in &pipes {
                syscall::sys_close(p[0]);
                syscall::sys_close(p[1]);
            }
            return Err(VshError::ForkFailed);
        }

        if pid == 0 {
            // Child process

            // Set up stdin from previous pipe
            if i > 0 {
                syscall::sys_dup2(pipes[i - 1][0], 0);
            }

            // Set up stdout to next pipe
            if i < n - 1 {
                syscall::sys_dup2(pipes[i][1], 1);
            }

            // Close all pipe fds (they've been dup'd where needed)
            for p in &pipes {
                syscall::sys_close(p[0]);
                syscall::sys_close(p[1]);
            }

            // Execute the command
            let status = super::execute_command(shell, &commands[i]).unwrap_or(1);
            syscall::sys_exit(status);
        }

        child_pids.push(pid as i32);
    }

    // Parent: close all pipe fds
    for p in &pipes {
        syscall::sys_close(p[0]);
        syscall::sys_close(p[1]);
    }

    // Wait for all children
    let mut last_status = 0;
    let mut pipe_statuses: Vec<i32> = Vec::with_capacity(n);

    for &cpid in &child_pids {
        let (ret, status) = syscall::sys_waitpid(cpid, 0);
        let exit_code = if ret > 0 {
            if status & 0x7f == 0 {
                (status >> 8) & 0xff
            } else {
                128 + (status & 0x7f)
            }
        } else {
            127
        };
        pipe_statuses.push(exit_code);
    }

    // The exit status of a pipeline is the exit status of the last command
    if let Some(&s) = pipe_statuses.last() {
        last_status = s;
    }

    // If pipefail is set, return the rightmost non-zero exit status
    if shell.config.set_opts.pipefail {
        for &s in pipe_statuses.iter().rev() {
            if s != 0 {
                last_status = s;
                break;
            }
        }
    }

    // Store PIPESTATUS array
    // (In a full implementation, set PIPESTATUS array variable)

    Ok(last_status)
}

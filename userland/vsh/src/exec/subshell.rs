//! Subshell execution.
//!
//! Runs a list of commands in a forked child process: `( list )`.

extern crate alloc;

use crate::{
    error::{Result, VshError},
    parser::ast::CompleteCommand,
    syscall, Shell,
};

/// Execute a list of commands in a subshell.
pub fn execute_subshell(shell: &mut Shell, body: &[CompleteCommand]) -> Result<i32> {
    let pid = syscall::sys_fork();
    if pid < 0 {
        return Err(VshError::ForkFailed);
    }

    if pid == 0 {
        // Child: execute the body
        let mut status = 0;
        for cmd in body {
            match super::execute_complete_command(shell, cmd) {
                Ok(s) => {
                    status = s;
                    shell.env.last_status = s;
                }
                Err(VshError::Exit(code)) => {
                    syscall::sys_exit(code);
                }
                Err(_) => {
                    status = 1;
                }
            }
        }
        syscall::sys_exit(status);
    }

    // Parent: wait for subshell
    let (ret, wait_status) = syscall::sys_waitpid(pid as i32, 0);
    if ret < 0 {
        return Ok(127);
    }

    let exit_code = if wait_status & 0x7f == 0 {
        (wait_status >> 8) & 0xff
    } else {
        128 + (wait_status & 0x7f)
    };

    Ok(exit_code)
}

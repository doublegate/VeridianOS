//! Coprocess management.
//!
//! Implements `coproc [NAME] command`: runs a command in the background
//! with stdin/stdout connected to the shell via file descriptors stored
//! in `${NAME[0]}` (read from coproc) and `${NAME[1]}` (write to coproc).

extern crate alloc;

use alloc::format;

use crate::{
    error::{Result, VshError},
    parser::ast::CoprocCommand,
    println, syscall, Shell,
};

/// Execute a coprocess command.
pub fn execute_coproc(shell: &mut Shell, coproc: &CoprocCommand) -> Result<i32> {
    let name = coproc.name.as_deref().unwrap_or("COPROC");

    // Create two pipes: one for stdin, one for stdout
    let mut stdin_pipe = [0i32; 2]; // shell writes to [1], coproc reads from [0]
    let mut stdout_pipe = [0i32; 2]; // coproc writes to [1], shell reads from [0]

    if syscall::sys_pipe(&mut stdin_pipe) < 0 {
        return Err(VshError::PipeFailed);
    }
    if syscall::sys_pipe(&mut stdout_pipe) < 0 {
        syscall::sys_close(stdin_pipe[0]);
        syscall::sys_close(stdin_pipe[1]);
        return Err(VshError::PipeFailed);
    }

    let pid = syscall::sys_fork();
    if pid < 0 {
        syscall::sys_close(stdin_pipe[0]);
        syscall::sys_close(stdin_pipe[1]);
        syscall::sys_close(stdout_pipe[0]);
        syscall::sys_close(stdout_pipe[1]);
        return Err(VshError::ForkFailed);
    }

    if pid == 0 {
        // Child: set up stdin from pipe, stdout to pipe
        syscall::sys_dup2(stdin_pipe[0], 0);
        syscall::sys_dup2(stdout_pipe[1], 1);

        // Close unused pipe ends
        syscall::sys_close(stdin_pipe[0]);
        syscall::sys_close(stdin_pipe[1]);
        syscall::sys_close(stdout_pipe[0]);
        syscall::sys_close(stdout_pipe[1]);

        // Execute the command
        let status = super::execute_command(shell, &coproc.body).unwrap_or(1);
        syscall::sys_exit(status);
    }

    // Parent: close unused pipe ends
    syscall::sys_close(stdin_pipe[0]);
    syscall::sys_close(stdout_pipe[1]);

    // Store fd numbers in the COPROC array variable
    // ${COPROC[0]} = fd to read from (stdout of coproc)
    // ${COPROC[1]} = fd to write to (stdin of coproc)
    let _ = shell
        .env
        .set_array_element(name, 0, &format!("{}", stdout_pipe[0]));
    let _ = shell
        .env
        .set_array_element(name, 1, &format!("{}", stdin_pipe[1]));

    // Add to job table
    let pid_i32 = pid as i32;
    let job_id = shell.jobs.add_job(
        pid_i32,
        alloc::vec![pid_i32],
        format!("coproc {}", name),
        true,
    );
    shell.env.last_bg_pid = pid_i32;
    println!("[{}] {}", job_id, pid_i32);

    Ok(0)
}

//! Command execution engine.
//!
//! Executes parsed AST nodes by dispatching to builtins, external commands,
//! pipelines, compound commands, and function calls.  Uses fork/exec for
//! external command execution and pipe/dup2 for pipeline setup.

extern crate alloc;

use alloc::{string::String, vec::Vec};

use crate::{error::Result, expand, parser::ast::*, println, Shell};

pub mod compound;
pub mod coproc;
pub mod eval;
pub mod function;
pub mod pipeline;
pub mod redirect;
pub mod script;
pub mod simple;
pub mod source;
pub mod subshell;

// ---------------------------------------------------------------------------
// Top-level execution entry points
// ---------------------------------------------------------------------------

/// Execute a parsed program (a list of complete commands).
pub fn execute_program(shell: &mut Shell, program: &Program) -> Result<i32> {
    let mut status = 0;
    for cmd in &program.commands {
        status = execute_complete_command(shell, cmd)?;
        shell.env.last_status = status;
        if shell.config.set_opts.errexit && status != 0 {
            break;
        }
    }
    Ok(status)
}

/// Execute a single complete command (and-or list, possibly backgrounded).
pub fn execute_complete_command(shell: &mut Shell, cmd: &CompleteCommand) -> Result<i32> {
    if cmd.background {
        execute_background(shell, &cmd.list)
    } else {
        execute_and_or_list(shell, &cmd.list)
    }
}

/// Execute an and-or list: `cmd1 && cmd2 || cmd3`.
pub fn execute_and_or_list(shell: &mut Shell, list: &AndOrList) -> Result<i32> {
    let mut status = execute_pipeline(shell, &list.first)?;
    shell.env.last_status = status;

    for (op, pipeline) in &list.rest {
        match op {
            AndOrOp::And => {
                if status == 0 {
                    status = execute_pipeline(shell, pipeline)?;
                    shell.env.last_status = status;
                }
            }
            AndOrOp::Or => {
                if status != 0 {
                    status = execute_pipeline(shell, pipeline)?;
                    shell.env.last_status = status;
                }
            }
        }
    }
    Ok(status)
}

/// Execute a pipeline: `cmd1 | cmd2 | cmd3`.
pub fn execute_pipeline(shell: &mut Shell, pipe: &Pipeline) -> Result<i32> {
    let cmds = &pipe.commands;
    if cmds.is_empty() {
        return Ok(0);
    }

    // Single command: no pipe needed
    if cmds.len() == 1 {
        let status = execute_command(shell, &cmds[0])?;
        return Ok(if pipe.negated {
            if status == 0 {
                1
            } else {
                0
            }
        } else {
            status
        });
    }

    // Multi-command pipeline
    let status = pipeline::execute_pipeline_multi(shell, cmds, pipe.pipe_stderr)?;
    Ok(if pipe.negated {
        if status == 0 {
            1
        } else {
            0
        }
    } else {
        status
    })
}

/// Execute a single command (simple, compound, function def, or coproc).
pub fn execute_command(shell: &mut Shell, cmd: &Command) -> Result<i32> {
    match cmd {
        Command::Simple(simple) => simple::execute_simple(shell, simple),
        Command::Compound(compound, redirects) => {
            let saved = redirect::setup_redirects(shell, redirects)?;
            let status = compound::execute_compound(shell, compound);
            redirect::restore_redirects(&saved);
            status
        }
        Command::FunctionDef(func) => {
            function::define_function(shell, func);
            Ok(0)
        }
        Command::Coproc(cp) => coproc::execute_coproc(shell, cp),
    }
}

/// Execute a command in the background.
fn execute_background(shell: &mut Shell, list: &AndOrList) -> Result<i32> {
    let pid = crate::syscall::sys_fork();
    if pid < 0 {
        return Err(crate::error::VshError::ForkFailed);
    }

    if pid == 0 {
        let status = execute_and_or_list(shell, list).unwrap_or(1);
        crate::syscall::sys_exit(status);
    }

    let pid_i32 = pid as i32;
    let job_id = shell
        .jobs
        .add_job(pid_i32, alloc::vec![pid_i32], String::from("&"), true);
    shell.env.last_bg_pid = pid_i32;
    println!("[{}] {}", job_id, pid_i32);
    Ok(0)
}

// ---------------------------------------------------------------------------
// Word expansion helpers (used by submodules)
// ---------------------------------------------------------------------------

/// Expand a single AST Word using the shell's current state.
pub fn expand_word(shell: &Shell, word: &Word) -> Vec<String> {
    let vars = shell.vars_map();
    let special = shell.special_vars();
    let do_glob = !shell.config.set_opts.noglob;
    expand::expand_word(&word.raw, &vars, &special, do_glob)
}

/// Expand a list of AST Words.
pub fn expand_words(shell: &Shell, words: &[Word]) -> Vec<String> {
    let vars = shell.vars_map();
    let special = shell.special_vars();
    let do_glob = !shell.config.set_opts.noglob;
    expand::expand_words(words, &vars, &special, do_glob)
}

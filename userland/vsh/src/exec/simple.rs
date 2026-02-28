//! Simple command execution.
//!
//! Handles commands of the form: `[VAR=val...] cmd [args...] [redirects...]`
//! Executes builtins in-process, external commands via fork/exec.

extern crate alloc;

use alloc::{string::String, vec::Vec};

use super::redirect;
use crate::{
    builtin, eprintln,
    error::{Result, VshError},
    parser::ast::SimpleCommand,
    syscall, Shell,
};

/// Execute a simple command.
pub fn execute_simple(shell: &mut Shell, cmd: &SimpleCommand) -> Result<i32> {
    // Expand words
    let expanded_words = super::expand_words(shell, &cmd.words);

    // Handle empty command (assignments only)
    if expanded_words.is_empty() {
        // Apply variable assignments to the current environment
        for assign in &cmd.assignments {
            let value_parts = super::expand_word(shell, &assign.value);
            let value = if value_parts.is_empty() {
                String::new()
            } else {
                value_parts.join(" ")
            };

            if assign.append {
                let old = String::from(shell.env.get_str(&assign.name));
                let mut new = old;
                new.push_str(&value);
                let _ = shell.env.set(&assign.name, &new);
            } else {
                let _ = shell.env.set(&assign.name, &value);
            }
        }
        return Ok(0);
    }

    let cmd_name = &expanded_words[0];
    let args = &expanded_words[1..];

    // Check for alias expansion (if enabled)
    // For simplicity, aliases are expanded during parsing/lexing in a
    // full implementation. Here we do a single-level expansion.

    // Source and eval are special: they need to run in the current shell.
    if cmd_name == "source" || cmd_name == "." {
        if args.is_empty() {
            eprintln!("vsh: {}: filename argument required", cmd_name);
            return Ok(2);
        }
        return super::source::source_file(shell, &args[0]);
    }

    if cmd_name == "eval" {
        let eval_str = args.join(" ");
        return super::eval::eval_string(shell, &eval_str);
    }

    if cmd_name == "exec" {
        return exec_replace(shell, args, &cmd.redirects);
    }

    // Set up redirections
    let saved = redirect::setup_redirects(shell, &cmd.redirects)?;

    // Check for builtins
    if builtin::is_builtin(cmd_name) {
        let args_vec: Vec<String> = args.to_vec();
        let result = execute_builtin_with_env(shell, cmd_name, &args_vec, &cmd.assignments);
        redirect::restore_redirects(&saved);
        return result;
    }

    // Check for shell functions
    if shell.env.functions.contains_key(cmd_name.as_str()) {
        let args_vec: Vec<String> = args.to_vec();
        let result = super::function::call_function(shell, cmd_name, &args_vec);
        redirect::restore_redirects(&saved);
        return result;
    }

    // External command: fork/exec
    let result = execute_external(shell, cmd_name, args, &cmd.assignments);
    redirect::restore_redirects(&saved);
    result
}

/// Execute a builtin with optional prefix assignments.
fn execute_builtin_with_env(
    shell: &mut Shell,
    name: &str,
    args: &[String],
    assignments: &[crate::parser::ast::Assignment],
) -> Result<i32> {
    // Temporarily set assignment variables
    let mut saved_vals: Vec<(String, Option<String>)> = Vec::new();

    for assign in assignments {
        let value_parts = super::expand_word(shell, &assign.value);
        let value = value_parts.join(" ");
        let old = if shell.env.is_set(&assign.name) {
            Some(String::from(shell.env.get_str(&assign.name)))
        } else {
            None
        };
        saved_vals.push((assign.name.clone(), old));
        let _ = shell.env.set(&assign.name, &value);
    }

    let result = builtin::run_builtin(shell, name, args);

    // Restore original values (assignments are only for the command duration
    // unless the command is a builtin -- in Bash, builtins keep the assignments.
    // We follow this behavior, so we do NOT restore.)
    // However, if there was no command (assignments-only), they persist.
    // For builtins with prefix assignments, we keep them.
    let _ = saved_vals;

    result
}

/// Execute an external command via fork/exec.
fn execute_external(
    shell: &mut Shell,
    cmd: &str,
    args: &[String],
    assignments: &[crate::parser::ast::Assignment],
) -> Result<i32> {
    // Resolve the command path
    let path = match builtin::find_in_path(cmd, shell.env.get_str("PATH")) {
        Some(p) => p,
        None => {
            eprintln!("vsh: {}: command not found", cmd);
            return Ok(127);
        }
    };

    // Cache in hash table
    shell.env.hash_table.insert(String::from(cmd), path.clone());

    // Build argv: [path, args...]
    let mut argv_strs: Vec<Vec<u8>> = Vec::with_capacity(1 + args.len());
    let mut path_nul = Vec::with_capacity(path.len() + 1);
    path_nul.extend_from_slice(path.as_bytes());
    path_nul.push(0);
    argv_strs.push(path_nul);

    for arg in args {
        let mut a = Vec::with_capacity(arg.len() + 1);
        a.extend_from_slice(arg.as_bytes());
        a.push(0);
        argv_strs.push(a);
    }

    let argv_ptrs: Vec<*const u8> = argv_strs.iter().map(|s| s.as_ptr()).collect();
    let mut argv_with_null: Vec<*const u8> = argv_ptrs;
    argv_with_null.push(core::ptr::null());

    // Build envp with prefix assignments
    let mut env_strings = shell.env.collect_env();
    for assign in assignments {
        let value_parts = super::expand_word(shell, &assign.value);
        let value = value_parts.join(" ");
        env_strings.push(alloc::format!("{}={}", assign.name, value));
    }

    let mut env_nul: Vec<Vec<u8>> = Vec::with_capacity(env_strings.len());
    for e in &env_strings {
        let mut buf = Vec::with_capacity(e.len() + 1);
        buf.extend_from_slice(e.as_bytes());
        buf.push(0);
        env_nul.push(buf);
    }
    let envp_ptrs: Vec<*const u8> = env_nul.iter().map(|s| s.as_ptr()).collect();
    let mut envp_with_null: Vec<*const u8> = envp_ptrs;
    envp_with_null.push(core::ptr::null());

    // Fork
    let pid = syscall::sys_fork();
    if pid < 0 {
        return Err(VshError::ForkFailed);
    }

    if pid == 0 {
        // Child: exec
        let ret = syscall::sys_execve(
            argv_strs[0].as_ptr(),
            argv_with_null.as_ptr(),
            envp_with_null.as_ptr(),
        );
        // If we get here, exec failed
        eprintln!("vsh: {}: exec failed ({})", cmd, ret);
        syscall::sys_exit(126);
    }

    // Parent: wait for child
    let (ret, status) = syscall::sys_waitpid(pid as i32, 0);
    if ret < 0 {
        return Ok(127);
    }

    // Decode exit status
    let exit_code = if status & 0x7f == 0 {
        (status >> 8) & 0xff
    } else {
        128 + (status & 0x7f)
    };

    // Update $_ to the last argument
    if let Some(last) = args.last() {
        shell.env.last_arg = last.clone();
    } else {
        shell.env.last_arg = String::from(cmd);
    }

    Ok(exit_code)
}

/// exec builtin: replace the current process with the given command.
fn exec_replace(
    shell: &mut Shell,
    args: &[String],
    redirects: &[crate::parser::ast::Redirect],
) -> Result<i32> {
    // If no arguments, exec just applies redirections permanently
    if args.is_empty() {
        let _saved = redirect::setup_redirects(shell, redirects)?;
        // Don't restore -- these redirections are permanent with exec
        return Ok(0);
    }

    let cmd = &args[0];
    let path = match builtin::find_in_path(cmd, shell.env.get_str("PATH")) {
        Some(p) => p,
        None => {
            eprintln!("vsh: exec: {}: not found", cmd);
            return Ok(127);
        }
    };

    // Set up redirections before exec (they'll be inherited)
    let _saved = redirect::setup_redirects(shell, redirects)?;

    // Build argv
    let mut argv_strs: Vec<Vec<u8>> = Vec::new();
    let mut path_nul = Vec::with_capacity(path.len() + 1);
    path_nul.extend_from_slice(path.as_bytes());
    path_nul.push(0);
    argv_strs.push(path_nul);

    for arg in &args[1..] {
        let mut a = Vec::with_capacity(arg.len() + 1);
        a.extend_from_slice(arg.as_bytes());
        a.push(0);
        argv_strs.push(a);
    }

    let argv_ptrs: Vec<*const u8> = argv_strs.iter().map(|s| s.as_ptr()).collect();
    let mut argv_with_null: Vec<*const u8> = argv_ptrs;
    argv_with_null.push(core::ptr::null());

    let env_strings = shell.env.collect_env();
    let mut env_nul: Vec<Vec<u8>> = Vec::new();
    for e in &env_strings {
        let mut buf = Vec::with_capacity(e.len() + 1);
        buf.extend_from_slice(e.as_bytes());
        buf.push(0);
        env_nul.push(buf);
    }
    let envp_ptrs: Vec<*const u8> = env_nul.iter().map(|s| s.as_ptr()).collect();
    let mut envp_with_null: Vec<*const u8> = envp_ptrs;
    envp_with_null.push(core::ptr::null());

    let ret = syscall::sys_execve(
        argv_strs[0].as_ptr(),
        argv_with_null.as_ptr(),
        envp_with_null.as_ptr(),
    );

    // If we get here, exec failed
    Err(VshError::ExecFailed(alloc::format!("{} ({})", cmd, ret)))
}

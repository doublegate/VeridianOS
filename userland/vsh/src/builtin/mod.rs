//! Shell builtin commands.
//!
//! Implements the standard set of Bash builtins: cd, echo, exit, export,
//! unset, set, shopt, alias, unalias, type, hash, source/`.`, eval,
//! exec, read, printf, test/`[`, true, false, colon, pwd, jobs, fg, bg,
//! wait, kill, trap, return, break, continue, shift, declare/typeset,
//! local, readonly, let, history.

extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use crate::{eprintln, error::Result, print, println, syscall, Shell};

/// Check if a command name is a builtin.
pub fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "cd" | "echo"
            | "exit"
            | "export"
            | "unset"
            | "set"
            | "shopt"
            | "alias"
            | "unalias"
            | "type"
            | "hash"
            | "source"
            | "."
            | "eval"
            | "exec"
            | "read"
            | "printf"
            | "test"
            | "["
            | "true"
            | "false"
            | ":"
            | "pwd"
            | "jobs"
            | "fg"
            | "bg"
            | "wait"
            | "kill"
            | "trap"
            | "return"
            | "break"
            | "continue"
            | "shift"
            | "declare"
            | "typeset"
            | "local"
            | "readonly"
            | "let"
            | "history"
    )
}

/// Run a builtin command. Returns the exit status.
pub fn run_builtin(shell: &mut Shell, name: &str, args: &[String]) -> Result<i32> {
    match name {
        "cd" => builtin_cd(shell, args),
        "echo" => builtin_echo(args),
        "exit" => builtin_exit(shell, args),
        "export" => builtin_export(shell, args),
        "unset" => builtin_unset(shell, args),
        "set" => builtin_set(shell, args),
        "pwd" => builtin_pwd(),
        "true" | ":" => Ok(0),
        "false" => Ok(1),
        "type" => builtin_type(shell, args),
        "alias" => builtin_alias(shell, args),
        "unalias" => builtin_unalias(shell, args),
        "shift" => builtin_shift(shell, args),
        "return" => builtin_return(shell, args),
        "break" => builtin_break(args),
        "continue" => builtin_continue(args),
        "test" | "[" => builtin_test(args),
        "declare" | "typeset" => builtin_declare(shell, args),
        "local" => builtin_local(shell, args),
        "readonly" => builtin_readonly(shell, args),
        "read" => builtin_read(shell, args),
        "printf" => builtin_printf(args),
        "jobs" => builtin_jobs(shell),
        "fg" => builtin_fg(shell, args),
        "bg" => builtin_bg(shell, args),
        "wait" => builtin_wait(shell, args),
        "history" => builtin_history(shell, args),
        "hash" => builtin_hash(shell, args),
        "let" => builtin_let(shell, args),
        "source" | "." => {
            // Handled by the exec layer (needs to evaluate in current shell)
            Ok(0)
        }
        "eval" => {
            // Handled by the exec layer
            Ok(0)
        }
        "exec" => {
            // Handled by the exec layer (replaces the process)
            Ok(0)
        }
        "kill" => builtin_kill(args),
        "trap" => builtin_trap(shell, args),
        "shopt" => builtin_shopt(shell, args),
        _ => {
            eprintln!("vsh: {}: not a builtin", name);
            Ok(127)
        }
    }
}

// ---------------------------------------------------------------------------
// cd
// ---------------------------------------------------------------------------

fn builtin_cd(shell: &mut Shell, args: &[String]) -> Result<i32> {
    let target = if args.is_empty() {
        let home = String::from(shell.env.get_str("HOME"));
        if home.is_empty() {
            eprintln!("vsh: cd: HOME not set");
            return Ok(1);
        }
        home
    } else if args[0] == "-" {
        let old = String::from(shell.env.get_str("OLDPWD"));
        if old.is_empty() {
            eprintln!("vsh: cd: OLDPWD not set");
            return Ok(1);
        }
        println!("{}", old);
        old
    } else {
        args[0].clone()
    };

    let mut path_buf = Vec::with_capacity(target.len() + 1);
    path_buf.extend_from_slice(target.as_bytes());
    path_buf.push(0);

    let ret = syscall::sys_chdir(path_buf.as_ptr());
    if ret < 0 {
        eprintln!("vsh: cd: {}: No such file or directory", target);
        return Ok(1);
    }

    shell.update_cwd();
    Ok(0)
}

// ---------------------------------------------------------------------------
// echo
// ---------------------------------------------------------------------------

fn builtin_echo(args: &[String]) -> Result<i32> {
    let mut newline = true;
    let mut interpret_escapes = false;
    let mut start = 0;

    // Process flags
    while start < args.len() {
        if args[start] == "-n" {
            newline = false;
            start += 1;
        } else if args[start] == "-e" {
            interpret_escapes = true;
            start += 1;
        } else if args[start] == "-E" {
            interpret_escapes = false;
            start += 1;
        } else if args[start] == "-ne" || args[start] == "-en" {
            newline = false;
            interpret_escapes = true;
            start += 1;
        } else {
            break;
        }
    }

    let out = crate::output::Writer::stdout();
    for (i, arg) in args[start..].iter().enumerate() {
        if i > 0 {
            out.write_str(" ");
        }
        if interpret_escapes {
            out.write_str(&interpret_echo_escapes(arg));
        } else {
            out.write_str(arg);
        }
    }
    if newline {
        out.write_str("\n");
    }
    Ok(0)
}

fn interpret_echo_escapes(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'n' => {
                    result.push('\n');
                    i += 2;
                }
                b't' => {
                    result.push('\t');
                    i += 2;
                }
                b'r' => {
                    result.push('\r');
                    i += 2;
                }
                b'a' => {
                    result.push('\x07');
                    i += 2;
                }
                b'b' => {
                    result.push('\x08');
                    i += 2;
                }
                b'e' => {
                    result.push('\x1B');
                    i += 2;
                }
                b'f' => {
                    result.push('\x0C');
                    i += 2;
                }
                b'v' => {
                    result.push('\x0B');
                    i += 2;
                }
                b'\\' => {
                    result.push('\\');
                    i += 2;
                }
                b'0' => {
                    // Octal
                    i += 2;
                    let mut val: u8 = 0;
                    let mut count = 0;
                    while i < bytes.len() && count < 3 && bytes[i] >= b'0' && bytes[i] <= b'7' {
                        val = val * 8 + (bytes[i] - b'0');
                        i += 1;
                        count += 1;
                    }
                    result.push(val as char);
                }
                b'x' => {
                    // Hex
                    i += 2;
                    let mut val: u8 = 0;
                    let mut count = 0;
                    while i < bytes.len() && count < 2 {
                        let d = bytes[i];
                        let v = if d.is_ascii_digit() {
                            Some(d - b'0')
                        } else if d.is_ascii_lowercase() && d <= b'f' {
                            Some(d - b'a' + 10)
                        } else if d.is_ascii_uppercase() && d <= b'F' {
                            Some(d - b'A' + 10)
                        } else {
                            None
                        };
                        match v {
                            Some(digit) => {
                                val = val * 16 + digit;
                                i += 1;
                                count += 1;
                            }
                            None => break,
                        }
                    }
                    result.push(val as char);
                }
                _ => {
                    result.push('\\');
                    result.push(bytes[i + 1] as char);
                    i += 2;
                }
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

// ---------------------------------------------------------------------------
// exit
// ---------------------------------------------------------------------------

fn builtin_exit(shell: &mut Shell, args: &[String]) -> Result<i32> {
    let code = if args.is_empty() {
        shell.env.last_status
    } else {
        parse_i32(&args[0]).unwrap_or(2)
    };
    Err(crate::error::VshError::Exit(code))
}

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

fn builtin_export(shell: &mut Shell, args: &[String]) -> Result<i32> {
    if args.is_empty() {
        // List all exported variables
        for name in shell.env.all_var_names() {
            if let Some(var) = shell.env.get(&name) {
                if var.attrs.exported {
                    println!("declare -x {}=\"{}\"", name, var.value.as_str());
                }
            }
        }
        return Ok(0);
    }

    for arg in args {
        if let Some(eq) = arg.find('=') {
            let name = &arg[..eq];
            let value = &arg[eq + 1..];
            shell.env.export(name, Some(value));
        } else {
            shell.env.export(arg, None);
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// unset
// ---------------------------------------------------------------------------

fn builtin_unset(shell: &mut Shell, args: &[String]) -> Result<i32> {
    let mut unset_func = false;
    let start = if !args.is_empty() && args[0] == "-f" {
        unset_func = true;
        1
    } else if !args.is_empty() && args[0] == "-v" {
        1
    } else {
        0
    };

    for name in &args[start..] {
        if unset_func {
            shell.env.functions.remove(name.as_str());
        } else {
            let _ = shell.env.unset(name);
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// set
// ---------------------------------------------------------------------------

fn builtin_set(shell: &mut Shell, args: &[String]) -> Result<i32> {
    if args.is_empty() {
        // Print all variables
        for name in shell.env.all_var_names() {
            let val = shell.env.get_str(&name);
            println!("{}={}", name, val);
        }
        return Ok(0);
    }

    if args[0] == "--" {
        // Reassign positional parameters
        shell.env.positional = args[1..].to_vec();
        return Ok(0);
    }

    if args[0].starts_with('-') || args[0].starts_with('+') {
        let enable = args[0].starts_with('-');
        for ch in args[0][1..].chars() {
            shell.config.set_opts.apply(ch, enable);
        }
    }

    Ok(0)
}

// ---------------------------------------------------------------------------
// pwd
// ---------------------------------------------------------------------------

fn builtin_pwd() -> Result<i32> {
    let mut buf = [0u8; 512];
    let n = syscall::sys_getcwd(&mut buf);
    if n > 0 {
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            println!("{}", s.trim_end_matches('\0'));
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// type
// ---------------------------------------------------------------------------

fn builtin_type(shell: &mut Shell, args: &[String]) -> Result<i32> {
    let mut status = 0;
    for arg in args {
        if is_builtin(arg) {
            println!("{} is a shell builtin", arg);
        } else if shell.env.functions.contains_key(arg.as_str()) {
            println!("{} is a function", arg);
        } else if shell.env.aliases.contains_key(arg.as_str()) {
            let val = shell.env.aliases.get(arg.as_str()).unwrap();
            println!("{} is aliased to `{}'", arg, val);
        } else {
            // Try to find in PATH
            match find_in_path(arg, shell.env.get_str("PATH")) {
                Some(path) => println!("{} is {}", arg, path),
                None => {
                    eprintln!("vsh: type: {}: not found", arg);
                    status = 1;
                }
            }
        }
    }
    Ok(status)
}

/// Search for a command in the PATH.
pub fn find_in_path(cmd: &str, path_var: &str) -> Option<String> {
    if cmd.contains('/') {
        // Absolute or relative path -- check directly
        let mut buf = Vec::with_capacity(cmd.len() + 1);
        buf.extend_from_slice(cmd.as_bytes());
        buf.push(0);
        let ret = syscall::sys_access(buf.as_ptr(), syscall::X_OK);
        if ret >= 0 {
            return Some(String::from(cmd));
        }
        return None;
    }

    for dir in path_var.split(':') {
        if dir.is_empty() {
            continue;
        }
        let full = format!("{}/{}", dir, cmd);
        let mut buf = Vec::with_capacity(full.len() + 1);
        buf.extend_from_slice(full.as_bytes());
        buf.push(0);
        let ret = syscall::sys_access(buf.as_ptr(), syscall::X_OK);
        if ret >= 0 {
            return Some(full);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// alias / unalias
// ---------------------------------------------------------------------------

fn builtin_alias(shell: &mut Shell, args: &[String]) -> Result<i32> {
    if args.is_empty() {
        for (name, val) in &shell.env.aliases {
            println!("alias {}='{}'", name, val);
        }
        return Ok(0);
    }
    for arg in args {
        if let Some(eq) = arg.find('=') {
            let name = &arg[..eq];
            let value = &arg[eq + 1..];
            shell
                .env
                .aliases
                .insert(String::from(name), String::from(value));
        } else {
            match shell.env.aliases.get(arg.as_str()) {
                Some(val) => println!("alias {}='{}'", arg, val),
                None => {
                    eprintln!("vsh: alias: {}: not found", arg);
                    return Ok(1);
                }
            }
        }
    }
    Ok(0)
}

fn builtin_unalias(shell: &mut Shell, args: &[String]) -> Result<i32> {
    if !args.is_empty() && args[0] == "-a" {
        shell.env.aliases.clear();
        return Ok(0);
    }
    for arg in args {
        shell.env.aliases.remove(arg.as_str());
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// shift
// ---------------------------------------------------------------------------

fn builtin_shift(shell: &mut Shell, args: &[String]) -> Result<i32> {
    let n = if args.is_empty() {
        1
    } else {
        parse_usize(&args[0]).unwrap_or(1)
    };

    if n > shell.env.positional.len() {
        eprintln!("vsh: shift: shift count out of range");
        return Ok(1);
    }

    shell.env.positional = shell.env.positional[n..].to_vec();
    Ok(0)
}

// ---------------------------------------------------------------------------
// return
// ---------------------------------------------------------------------------

fn builtin_return(_shell: &mut Shell, args: &[String]) -> Result<i32> {
    let code = if args.is_empty() {
        0
    } else {
        parse_i32(&args[0]).unwrap_or(2)
    };
    // `return` uses the Exit error to unwind to the function call boundary.
    // The exec layer distinguishes between return-from-function and exit.
    Ok(code)
}

// ---------------------------------------------------------------------------
// break / continue
// ---------------------------------------------------------------------------

fn builtin_break(args: &[String]) -> Result<i32> {
    let _n = if args.is_empty() {
        1
    } else {
        parse_usize(&args[0]).unwrap_or(1)
    };
    // The exec layer handles break by checking the command result.
    Ok(0)
}

fn builtin_continue(args: &[String]) -> Result<i32> {
    let _n = if args.is_empty() {
        1
    } else {
        parse_usize(&args[0]).unwrap_or(1)
    };
    Ok(0)
}

// ---------------------------------------------------------------------------
// test / [
// ---------------------------------------------------------------------------

fn builtin_test(args: &[String]) -> Result<i32> {
    let actual_args = if !args.is_empty() && args.last().map(|s| s.as_str()) == Some("]") {
        &args[..args.len() - 1]
    } else {
        args
    };
    let result = crate::parser::test_expr::eval_test(actual_args);
    Ok(if result { 0 } else { 1 })
}

// ---------------------------------------------------------------------------
// declare / typeset
// ---------------------------------------------------------------------------

fn builtin_declare(shell: &mut Shell, args: &[String]) -> Result<i32> {
    if args.is_empty() {
        // Print all variables with attributes
        for name in shell.env.all_var_names() {
            if let Some(var) = shell.env.get(&name) {
                let mut flags = String::from("declare ");
                if var.attrs.exported {
                    flags.push_str("-x ");
                }
                if var.attrs.readonly {
                    flags.push_str("-r ");
                }
                if var.attrs.integer {
                    flags.push_str("-i ");
                }
                if var.attrs.is_array {
                    flags.push_str("-a ");
                }
                if var.attrs.is_assoc {
                    flags.push_str("-A ");
                }
                if var.attrs.lowercase {
                    flags.push_str("-l ");
                }
                if var.attrs.uppercase {
                    flags.push_str("-u ");
                }
                if var.attrs.nameref {
                    flags.push_str("-n ");
                }
                println!("{}{}=\"{}\"", flags, name, var.value.as_str());
            }
        }
        return Ok(0);
    }

    // Parse flags and assignments
    let mut export = false;
    let mut readonly = false;
    let mut integer = false;
    let mut array = false;
    let mut assoc = false;
    let mut names_start = 0;

    for (i, arg) in args.iter().enumerate() {
        if let Some(stripped) = arg.strip_prefix('-') {
            for ch in stripped.chars() {
                match ch {
                    'x' => export = true,
                    'r' => readonly = true,
                    'i' => integer = true,
                    'a' => array = true,
                    'A' => assoc = true,
                    _ => {}
                }
            }
            names_start = i + 1;
        } else {
            break;
        }
    }

    for arg in &args[names_start..] {
        if let Some(eq) = arg.find('=') {
            let name = &arg[..eq];
            let value = &arg[eq + 1..];
            let _ = shell.env.set(name, value);
        } else {
            // Just set attributes (set with empty value if not exists)
            if !shell.env.is_set(arg) {
                let _ = shell.env.set(arg, "");
            }
        }
        // Apply attributes
        if export {
            shell.env.export(arg.split('=').next().unwrap_or(arg), None);
        }
        if readonly {
            shell.env.set_readonly(arg.split('=').next().unwrap_or(arg));
        }
        let _ = (integer, array, assoc); // Attribute application placeholder
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// local
// ---------------------------------------------------------------------------

fn builtin_local(shell: &mut Shell, args: &[String]) -> Result<i32> {
    for arg in args {
        if let Some(eq) = arg.find('=') {
            let name = &arg[..eq];
            let value = &arg[eq + 1..];
            let _ = shell.env.set_local(name, value);
        } else {
            let _ = shell.env.set_local(arg, "");
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// readonly
// ---------------------------------------------------------------------------

fn builtin_readonly(shell: &mut Shell, args: &[String]) -> Result<i32> {
    if args.is_empty() {
        for name in shell.env.all_var_names() {
            if let Some(var) = shell.env.get(&name) {
                if var.attrs.readonly {
                    println!("declare -r {}=\"{}\"", name, var.value.as_str());
                }
            }
        }
        return Ok(0);
    }
    for arg in args {
        if let Some(eq) = arg.find('=') {
            let name = &arg[..eq];
            let value = &arg[eq + 1..];
            let _ = shell.env.set(name, value);
            shell.env.set_readonly(name);
        } else {
            shell.env.set_readonly(arg);
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// read
// ---------------------------------------------------------------------------

fn builtin_read(shell: &mut Shell, args: &[String]) -> Result<i32> {
    let mut prompt_str = None;
    let mut var_names = Vec::new();
    let mut i = 0;

    while i < args.len() {
        if args[i] == "-p" && i + 1 < args.len() {
            prompt_str = Some(args[i + 1].clone());
            i += 2;
        } else if args[i] == "-r" {
            // Raw mode (no backslash interpretation) -- default for simplicity
            i += 1;
        } else {
            var_names.push(args[i].clone());
            i += 1;
        }
    }

    if var_names.is_empty() {
        var_names.push(String::from("REPLY"));
    }

    if let Some(p) = &prompt_str {
        let out = crate::output::Writer::stderr();
        out.write_str(p);
    }

    let line = match crate::input::read_line("") {
        Some(l) => l,
        None => return Ok(1),
    };

    let ifs = String::from(shell.env.get_str("IFS"));
    let ifs_chars = if ifs.is_empty() { " \t\n" } else { &ifs };

    let fields: Vec<&str> = line
        .split(|c: char| ifs_chars.contains(c))
        .filter(|s| !s.is_empty())
        .collect();

    for (idx, name) in var_names.iter().enumerate() {
        if idx == var_names.len() - 1 {
            // Last variable gets remaining fields
            let val = if idx < fields.len() {
                fields[idx..].join(" ")
            } else {
                String::new()
            };
            let _ = shell.env.set(name, &val);
        } else if idx < fields.len() {
            let _ = shell.env.set(name, fields[idx]);
        } else {
            let _ = shell.env.set(name, "");
        }
    }

    Ok(0)
}

// ---------------------------------------------------------------------------
// printf
// ---------------------------------------------------------------------------

fn builtin_printf(args: &[String]) -> Result<i32> {
    if args.is_empty() {
        eprintln!("vsh: printf: usage: printf format [arguments]");
        return Ok(2);
    }

    let fmt_str = &args[0];
    let mut arg_idx = 1;
    let out = crate::output::Writer::stdout();

    let bytes = fmt_str.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b's' => {
                    let val = if arg_idx < args.len() {
                        &args[arg_idx]
                    } else {
                        ""
                    };
                    out.write_str(val);
                    arg_idx += 1;
                    i += 1;
                }
                b'd' | b'i' => {
                    let val = if arg_idx < args.len() {
                        parse_i64(&args[arg_idx]).unwrap_or(0)
                    } else {
                        0
                    };
                    print!("{}", val);
                    arg_idx += 1;
                    i += 1;
                }
                b'%' => {
                    out.write_str("%");
                    i += 1;
                }
                b'n' => {
                    out.write_str("\n");
                    i += 1;
                }
                _ => {
                    out.write_str("%");
                    out.write_bytes(&[bytes[i]]);
                    i += 1;
                }
            }
        } else if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'n' => {
                    out.write_str("\n");
                    i += 2;
                }
                b't' => {
                    out.write_str("\t");
                    i += 2;
                }
                b'r' => {
                    out.write_str("\r");
                    i += 2;
                }
                b'\\' => {
                    out.write_str("\\");
                    i += 2;
                }
                _ => {
                    out.write_bytes(&[bytes[i]]);
                    i += 1;
                }
            }
        } else {
            out.write_bytes(&[bytes[i]]);
            i += 1;
        }
    }

    Ok(0)
}

// ---------------------------------------------------------------------------
// jobs / fg / bg / wait
// ---------------------------------------------------------------------------

fn builtin_jobs(shell: &mut Shell) -> Result<i32> {
    shell.jobs.update_status();
    for line in shell.jobs.format_jobs() {
        println!("{}", line);
    }
    Ok(0)
}

fn builtin_fg(shell: &mut Shell, args: &[String]) -> Result<i32> {
    let job_id = if args.is_empty() {
        match shell.jobs.current() {
            Some(id) => id,
            None => {
                eprintln!("vsh: fg: no current job");
                return Ok(1);
            }
        }
    } else {
        match shell.jobs.resolve_job_spec(&args[0]) {
            Some(id) => id,
            None => {
                eprintln!("vsh: fg: {}: no such job", args[0]);
                return Ok(1);
            }
        }
    };

    let status = shell.jobs.wait_for_job(job_id);
    Ok(status)
}

fn builtin_bg(shell: &mut Shell, args: &[String]) -> Result<i32> {
    let job_id = if args.is_empty() {
        match shell.jobs.current() {
            Some(id) => id,
            None => {
                eprintln!("vsh: bg: no current job");
                return Ok(1);
            }
        }
    } else {
        match shell.jobs.resolve_job_spec(&args[0]) {
            Some(id) => id,
            None => {
                eprintln!("vsh: bg: {}: no such job", args[0]);
                return Ok(1);
            }
        }
    };

    // In a full implementation, we would send SIGCONT to the process group.
    let _ = job_id;
    Ok(0)
}

fn builtin_wait(shell: &mut Shell, args: &[String]) -> Result<i32> {
    if args.is_empty() {
        // Wait for all background jobs
        loop {
            let (pid, status) = syscall::sys_waitpid(-1, 0);
            if pid <= 0 {
                break;
            }
            let _ = status;
        }
        return Ok(0);
    }

    for arg in args {
        if let Some(job_id) = shell.jobs.resolve_job_spec(arg) {
            let status = shell.jobs.wait_for_job(job_id);
            shell.env.last_status = status;
        } else if let Some(pid) = parse_i32(arg) {
            let (ret, status) = syscall::sys_waitpid(pid, 0);
            if ret > 0 {
                shell.env.last_status = (status >> 8) & 0xff;
            }
        }
    }
    Ok(shell.env.last_status)
}

// ---------------------------------------------------------------------------
// history
// ---------------------------------------------------------------------------

fn builtin_history(shell: &mut Shell, args: &[String]) -> Result<i32> {
    if !args.is_empty() && args[0] == "-c" {
        shell.readline.history.clear();
        return Ok(0);
    }

    for (i, entry) in shell.readline.history.entries().iter().enumerate() {
        println!("{:5}  {}", i + 1, entry);
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// hash
// ---------------------------------------------------------------------------

fn builtin_hash(shell: &mut Shell, args: &[String]) -> Result<i32> {
    if args.is_empty() {
        for (name, path) in &shell.env.hash_table {
            println!("{}={}", name, path);
        }
        return Ok(0);
    }
    if args[0] == "-r" {
        shell.env.hash_table.clear();
        return Ok(0);
    }
    for arg in args {
        if let Some(path) = find_in_path(arg, shell.env.get_str("PATH")) {
            shell.env.hash_table.insert(arg.clone(), path);
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// kill
// ---------------------------------------------------------------------------

fn builtin_kill(args: &[String]) -> Result<i32> {
    if args.is_empty() {
        eprintln!("vsh: kill: usage: kill [-signal] pid ...");
        return Ok(2);
    }

    let mut signal = 15i32; // SIGTERM
    let mut start = 0;

    if !args.is_empty() && args[0].starts_with('-') {
        if let Some(sig) = parse_i32(&args[0][1..]) {
            signal = sig;
            start = 1;
        }
    }

    for arg in &args[start..] {
        if let Some(pid) = parse_i32(arg) {
            let ret = unsafe {
                syscall::syscall2(syscall::SYS_PROCESS_KILL, pid as usize, signal as usize)
            };
            if ret < 0 {
                eprintln!("vsh: kill: ({}) - No such process", pid);
            }
        } else {
            eprintln!("vsh: kill: {}: invalid signal specification", arg);
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// trap
// ---------------------------------------------------------------------------

fn builtin_trap(_shell: &mut Shell, args: &[String]) -> Result<i32> {
    if args.is_empty() {
        // List traps -- not yet implemented
        return Ok(0);
    }
    // trap 'action' SIGNAL...
    // Simplified: store action strings for signals.
    // Full implementation deferred to exec/trap.rs.
    let _ = args;
    Ok(0)
}

// ---------------------------------------------------------------------------
// let
// ---------------------------------------------------------------------------

fn builtin_let(shell: &mut Shell, args: &[String]) -> Result<i32> {
    let vars = shell.vars_map();
    let mut result = 0i64;
    for arg in args {
        result = crate::parser::arithmetic::eval_arithmetic(arg, &vars).unwrap_or(0);
    }
    // `let` returns 1 if result is 0, 0 otherwise
    Ok(if result == 0 { 1 } else { 0 })
}

// ---------------------------------------------------------------------------
// shopt
// ---------------------------------------------------------------------------

fn builtin_shopt(shell: &mut Shell, args: &[String]) -> Result<i32> {
    if args.is_empty() {
        // List all shopt options
        println!(
            "extglob        {}",
            if shell.config.shopt_opts.extglob {
                "on"
            } else {
                "off"
            }
        );
        println!(
            "globstar       {}",
            if shell.config.shopt_opts.globstar {
                "on"
            } else {
                "off"
            }
        );
        println!(
            "dotglob        {}",
            if shell.config.shopt_opts.dotglob {
                "on"
            } else {
                "off"
            }
        );
        println!(
            "nullglob       {}",
            if shell.config.shopt_opts.nullglob {
                "on"
            } else {
                "off"
            }
        );
        println!(
            "failglob       {}",
            if shell.config.shopt_opts.failglob {
                "on"
            } else {
                "off"
            }
        );
        println!(
            "nocaseglob     {}",
            if shell.config.shopt_opts.nocaseglob {
                "on"
            } else {
                "off"
            }
        );
        println!(
            "expand_aliases {}",
            if shell.config.shopt_opts.expand_aliases {
                "on"
            } else {
                "off"
            }
        );
        return Ok(0);
    }

    let mut enable = true;
    let mut start = 0;
    if args[0] == "-s" {
        enable = true;
        start = 1;
    } else if args[0] == "-u" {
        enable = false;
        start = 1;
    }

    for arg in &args[start..] {
        match arg.as_str() {
            "extglob" => shell.config.shopt_opts.extglob = enable,
            "globstar" => shell.config.shopt_opts.globstar = enable,
            "dotglob" => shell.config.shopt_opts.dotglob = enable,
            "nullglob" => shell.config.shopt_opts.nullglob = enable,
            "failglob" => shell.config.shopt_opts.failglob = enable,
            "nocaseglob" => shell.config.shopt_opts.nocaseglob = enable,
            "nocasematch" => shell.config.shopt_opts.nocasematch = enable,
            "expand_aliases" => shell.config.shopt_opts.expand_aliases = enable,
            "sourcepath" => shell.config.shopt_opts.sourcepath = enable,
            "lastpipe" => shell.config.shopt_opts.lastpipe = enable,
            "autocd" => shell.config.shopt_opts.autocd = enable,
            "cdspell" => shell.config.shopt_opts.cdspell = enable,
            _ => {
                eprintln!("vsh: shopt: {}: invalid shell option name", arg);
                return Ok(1);
            }
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_i32(s: &str) -> Option<i32> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut n: i32 = 0;
    let mut neg = false;
    let mut i = 0;
    if bytes[0] == b'-' {
        neg = true;
        i = 1;
    }
    if i >= bytes.len() {
        return None;
    }
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((bytes[i] - b'0') as i32)?;
        i += 1;
    }
    Some(if neg { -n } else { n })
}

fn parse_usize(s: &str) -> Option<usize> {
    let mut n: usize = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(n)
}

fn parse_i64(s: &str) -> Option<i64> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut n: i64 = 0;
    let mut neg = false;
    let mut i = 0;
    if bytes[0] == b'-' {
        neg = true;
        i = 1;
    }
    if i >= bytes.len() {
        return None;
    }
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((bytes[i] - b'0') as i64)?;
        i += 1;
    }
    Some(if neg { -n } else { n })
}

//! Compound command execution.
//!
//! Handles `if`, `while`, `until`, `for`, `for((;;))`, `case`, `select`,
//! brace groups `{ }`, subshells `( )`, arithmetic `(( ))`, and
//! conditional expressions `[[ ]]`.

extern crate alloc;

use alloc::string::String;

use crate::{
    eprintln,
    error::{Result, VshError},
    parser::ast::*,
    Shell,
};

/// Execute a compound command.
pub fn execute_compound(shell: &mut Shell, cmd: &CompoundCommand) -> Result<i32> {
    match cmd {
        CompoundCommand::If(clause) => execute_if(shell, clause),
        CompoundCommand::While(clause) => execute_while(shell, clause),
        CompoundCommand::Until(clause) => execute_until(shell, clause),
        CompoundCommand::For(clause) => execute_for(shell, clause),
        CompoundCommand::ArithFor(clause) => execute_arith_for(shell, clause),
        CompoundCommand::Case(clause) => execute_case(shell, clause),
        CompoundCommand::Select(clause) => execute_select(shell, clause),
        CompoundCommand::BraceGroup(body) => execute_list(shell, body),
        CompoundCommand::Subshell(body) => super::subshell::execute_subshell(shell, body),
        CompoundCommand::ArithEval(expr) => execute_arith_evaluation(shell, expr),
        CompoundCommand::ConditionalExpr(expr) => execute_conditional(shell, expr),
    }
}

/// Execute a list of complete commands, returning the last status.
fn execute_list(shell: &mut Shell, body: &[CompleteCommand]) -> Result<i32> {
    let mut status = 0;
    for cmd in body {
        status = super::execute_complete_command(shell, cmd)?;
        shell.env.last_status = status;
    }
    Ok(status)
}

/// if/elif/else/fi
fn execute_if(shell: &mut Shell, clause: &IfClause) -> Result<i32> {
    for (condition, then_body) in &clause.branches {
        let cond_status = execute_list(shell, condition)?;
        if cond_status == 0 {
            return execute_list(shell, then_body);
        }
    }

    if let Some(else_body) = &clause.else_body {
        return execute_list(shell, else_body);
    }

    Ok(0)
}

/// while condition; do body; done
fn execute_while(shell: &mut Shell, clause: &WhileClause) -> Result<i32> {
    let mut last_status = 0;

    loop {
        let cond_status = execute_list(shell, &clause.condition)?;
        if cond_status != 0 {
            break;
        }

        match execute_list(shell, &clause.body) {
            Ok(s) => last_status = s,
            Err(VshError::Exit(code)) => return Err(VshError::Exit(code)),
            Err(e) => return Err(e),
        }
    }

    Ok(last_status)
}

/// until condition; do body; done
fn execute_until(shell: &mut Shell, clause: &UntilClause) -> Result<i32> {
    let mut last_status = 0;

    loop {
        let cond_status = execute_list(shell, &clause.condition)?;
        if cond_status == 0 {
            break;
        }

        match execute_list(shell, &clause.body) {
            Ok(s) => last_status = s,
            Err(VshError::Exit(code)) => return Err(VshError::Exit(code)),
            Err(e) => return Err(e),
        }
    }

    Ok(last_status)
}

/// for var [in words...]; do body; done
fn execute_for(shell: &mut Shell, clause: &ForClause) -> Result<i32> {
    let words = if let Some(word_list) = &clause.words {
        super::expand_words(shell, word_list)
    } else {
        // No word list: iterate over positional parameters
        shell.env.positional.clone()
    };

    let mut last_status = 0;

    for word in &words {
        let _ = shell.env.set(&clause.var, word);
        match execute_list(shell, &clause.body) {
            Ok(s) => last_status = s,
            Err(VshError::Exit(code)) => return Err(VshError::Exit(code)),
            Err(e) => return Err(e),
        }
    }

    Ok(last_status)
}

/// for ((init; cond; step)); do body; done
fn execute_arith_for(shell: &mut Shell, clause: &ArithForClause) -> Result<i32> {
    let vars = shell.vars_map();

    // Run init expression
    if !clause.init.is_empty() {
        let _ = crate::parser::arithmetic::eval_arithmetic(&clause.init, &vars);
    }

    let mut last_status = 0;

    loop {
        // Check condition
        if !clause.condition.is_empty() {
            let vars = shell.vars_map();
            let cond =
                crate::parser::arithmetic::eval_arithmetic(&clause.condition, &vars).unwrap_or(0);
            if cond == 0 {
                break;
            }
        }

        // Execute body
        match execute_list(shell, &clause.body) {
            Ok(s) => last_status = s,
            Err(VshError::Exit(code)) => return Err(VshError::Exit(code)),
            Err(e) => return Err(e),
        }

        // Run step expression
        if !clause.step.is_empty() {
            let vars = shell.vars_map();
            let _ = crate::parser::arithmetic::eval_arithmetic(&clause.step, &vars);
        }
    }

    Ok(last_status)
}

/// case word in pattern) body ;; ... esac
fn execute_case(shell: &mut Shell, clause: &CaseClause) -> Result<i32> {
    let word_expanded = super::expand_word(shell, &clause.word);
    let word_str = if word_expanded.is_empty() {
        String::new()
    } else {
        word_expanded[0].clone()
    };

    for item in &clause.items {
        let mut matched = false;

        for pattern in &item.patterns {
            let pattern_expanded = super::expand_word(shell, pattern);
            let pat_str = if pattern_expanded.is_empty() {
                String::new()
            } else {
                pattern_expanded[0].clone()
            };

            if crate::expand::glob::glob_match(&pat_str, &word_str) {
                matched = true;
                break;
            }
        }

        if matched {
            let status = execute_list(shell, &item.body)?;

            match item.terminator {
                CaseTerminator::Break => return Ok(status),
                CaseTerminator::FallThrough => {
                    // Fall through to next body
                    continue;
                }
                CaseTerminator::TestNext => {
                    // Continue testing next patterns
                    continue;
                }
            }
        }
    }

    Ok(0)
}

/// select var [in words]; do body; done
fn execute_select(shell: &mut Shell, clause: &SelectClause) -> Result<i32> {
    let words = if let Some(word_list) = &clause.words {
        super::expand_words(shell, word_list)
    } else {
        shell.env.positional.clone()
    };

    if words.is_empty() {
        return Ok(0);
    }

    let ps3 = String::from(shell.env.get_str("PS3"));
    let prompt = if ps3.is_empty() { "#? " } else { &ps3 };
    let out = crate::output::Writer::stderr();

    loop {
        // Display menu
        for (i, word) in words.iter().enumerate() {
            eprintln!("{}) {}", i + 1, word);
        }
        out.write_str(prompt);

        let line = match crate::input::read_line("") {
            Some(l) => l,
            None => break,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Parse selection number
        if let Some(n) = parse_usize(trimmed) {
            if n >= 1 && n <= words.len() {
                let _ = shell.env.set(&clause.var, &words[n - 1]);
                let _ = shell.env.set("REPLY", trimmed);
                match execute_list(shell, &clause.body) {
                    Ok(_) => {}
                    Err(VshError::Exit(code)) => return Err(VshError::Exit(code)),
                    Err(e) => return Err(e),
                }
            }
        } else {
            let _ = shell.env.set(&clause.var, "");
            let _ = shell.env.set("REPLY", trimmed);
            match execute_list(shell, &clause.body) {
                Ok(_) => {}
                Err(VshError::Exit(code)) => return Err(VshError::Exit(code)),
                Err(e) => return Err(e),
            }
        }
    }

    Ok(0)
}

/// (( expr )) -- arithmetic
fn execute_arith_evaluation(shell: &mut Shell, expr: &str) -> Result<i32> {
    let vars = shell.vars_map();
    let result = crate::parser::arithmetic::eval_arithmetic(expr, &vars).unwrap_or(0);
    // Returns 0 if non-zero, 1 if zero (like Bash)
    Ok(if result != 0 { 0 } else { 1 })
}

/// [[ expr ]] -- conditional expression
fn execute_conditional(shell: &mut Shell, expr: &ConditionalExpr) -> Result<i32> {
    let result = check_cond_expr(shell, expr);
    Ok(if result { 0 } else { 1 })
}

fn check_cond_expr(shell: &mut Shell, expr: &ConditionalExpr) -> bool {
    match expr {
        ConditionalExpr::Unary(op, word) => {
            let expanded = super::expand_word(shell, word);
            let val = if expanded.is_empty() {
                String::new()
            } else {
                expanded[0].clone()
            };
            check_unary_test(op, &val)
        }
        ConditionalExpr::Binary(left, op, right) => {
            let l = {
                let expanded = super::expand_word(shell, left);
                if expanded.is_empty() {
                    String::new()
                } else {
                    expanded[0].clone()
                }
            };
            let r = {
                let expanded = super::expand_word(shell, right);
                if expanded.is_empty() {
                    String::new()
                } else {
                    expanded[0].clone()
                }
            };
            check_binary_test(op, &l, &r)
        }
        ConditionalExpr::Regex(left, right) => {
            let l = {
                let expanded = super::expand_word(shell, left);
                if expanded.is_empty() {
                    String::new()
                } else {
                    expanded[0].clone()
                }
            };
            let r = {
                let expanded = super::expand_word(shell, right);
                if expanded.is_empty() {
                    String::new()
                } else {
                    expanded[0].clone()
                }
            };
            // Simplified regex: use glob match
            crate::expand::glob::glob_match(&r, &l)
        }
        ConditionalExpr::Not(inner) => !check_cond_expr(shell, inner),
        ConditionalExpr::And(a, b) => check_cond_expr(shell, a) && check_cond_expr(shell, b),
        ConditionalExpr::Or(a, b) => check_cond_expr(shell, a) || check_cond_expr(shell, b),
        ConditionalExpr::Group(inner) => check_cond_expr(shell, inner),
        ConditionalExpr::Word(word) => {
            let expanded = super::expand_word(shell, word);
            !expanded.is_empty() && !expanded[0].is_empty()
        }
    }
}

fn check_unary_test(op: &str, val: &str) -> bool {
    match op {
        "-z" => val.is_empty(),
        "-n" => !val.is_empty(),
        "-f" | "-e" | "-d" | "-r" | "-w" | "-x" | "-s" => {
            let mut buf = alloc::vec::Vec::with_capacity(val.len() + 1);
            buf.extend_from_slice(val.as_bytes());
            buf.push(0);
            let ret = crate::syscall::sys_access(buf.as_ptr(), crate::syscall::F_OK);
            ret >= 0
        }
        "-L" | "-h" => false,
        "-t" => {
            if let Some(fd) = parse_i32(val) {
                (0..=2).contains(&fd)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn check_binary_test(op: &str, left: &str, right: &str) -> bool {
    match op {
        "==" | "=" => left == right,
        "!=" => left != right,
        "<" => left < right,
        ">" => left > right,
        "-eq" => parse_i64(left).unwrap_or(0) == parse_i64(right).unwrap_or(0),
        "-ne" => parse_i64(left).unwrap_or(0) != parse_i64(right).unwrap_or(0),
        "-lt" => parse_i64(left).unwrap_or(0) < parse_i64(right).unwrap_or(0),
        "-le" => parse_i64(left).unwrap_or(0) <= parse_i64(right).unwrap_or(0),
        "-gt" => parse_i64(left).unwrap_or(0) > parse_i64(right).unwrap_or(0),
        "-ge" => parse_i64(left).unwrap_or(0) >= parse_i64(right).unwrap_or(0),
        "-ef" => left == right,
        "-nt" | "-ot" => false,
        _ => false,
    }
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

fn parse_i32(s: &str) -> Option<i32> {
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

//! Test/conditional expression parser for `[[ ... ]]` and `[ ... ]`.
//!
//! Parses conditional expressions into a `ConditionalExpr` AST node.

use alloc::{boxed::Box, string::String};

use crate::parser::ast::{ConditionalExpr, Word};

/// Parse a `[[ ... ]]` conditional expression from a list of words
/// (the words between `[[` and `]]`).
pub fn parse_conditional(words: &[String]) -> ConditionalExpr {
    let mut pos = 0;
    parse_or_expr(words, &mut pos)
}

fn parse_or_expr(words: &[String], pos: &mut usize) -> ConditionalExpr {
    let mut left = parse_and_expr(words, pos);
    while *pos < words.len() && words[*pos] == "||" {
        *pos += 1;
        let right = parse_and_expr(words, pos);
        left = ConditionalExpr::Or(Box::new(left), Box::new(right));
    }
    left
}

fn parse_and_expr(words: &[String], pos: &mut usize) -> ConditionalExpr {
    let mut left = parse_not_expr(words, pos);
    while *pos < words.len() && words[*pos] == "&&" {
        *pos += 1;
        let right = parse_not_expr(words, pos);
        left = ConditionalExpr::And(Box::new(left), Box::new(right));
    }
    left
}

fn parse_not_expr(words: &[String], pos: &mut usize) -> ConditionalExpr {
    if *pos < words.len() && words[*pos] == "!" {
        *pos += 1;
        let expr = parse_primary_expr(words, pos);
        ConditionalExpr::Not(Box::new(expr))
    } else {
        parse_primary_expr(words, pos)
    }
}

fn parse_primary_expr(words: &[String], pos: &mut usize) -> ConditionalExpr {
    if *pos >= words.len() {
        return ConditionalExpr::Word(Word::from_str(""));
    }

    // Parenthesized group
    if words[*pos] == "(" {
        *pos += 1;
        let expr = parse_or_expr(words, pos);
        if *pos < words.len() && words[*pos] == ")" {
            *pos += 1;
        }
        return ConditionalExpr::Group(Box::new(expr));
    }

    // Unary operators: -f, -d, -e, -z, -n, etc.
    if words[*pos].starts_with('-') && words[*pos].len() == 2 && *pos + 1 < words.len() {
        let op = words[*pos].clone();
        *pos += 1;
        let operand = words[*pos].clone();
        *pos += 1;
        return ConditionalExpr::Unary(op, Word::from_str(&operand));
    }

    // Binary operators: word OP word
    if *pos + 2 < words.len() {
        let lhs = words[*pos].clone();
        let op = &words[*pos + 1];
        if is_binary_op(op) {
            *pos += 1;
            let op = words[*pos].clone();
            *pos += 1;
            let rhs = words[*pos].clone();
            *pos += 1;
            if op == "=~" {
                return ConditionalExpr::Regex(Word::from_str(&lhs), Word::from_str(&rhs));
            }
            return ConditionalExpr::Binary(Word::from_str(&lhs), op, Word::from_str(&rhs));
        }
    }

    // Bare word (treated as -n word)
    let w = words[*pos].clone();
    *pos += 1;
    ConditionalExpr::Word(Word::from_str(&w))
}

fn is_binary_op(op: &str) -> bool {
    matches!(
        op,
        "==" | "="
            | "!="
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "=~"
            | "-nt"
            | "-ot"
            | "-ef"
    )
}

/// Evaluate a `[ ... ]` (test) expression.
///
/// This is a simpler form that handles the POSIX test(1) syntax.
pub fn eval_test(args: &[String]) -> bool {
    if args.is_empty() {
        return false;
    }

    match args.len() {
        1 => {
            // Single argument: true if non-empty
            !args[0].is_empty()
        }
        2 => {
            // Unary operator
            match args[0].as_str() {
                "!" => args[1].is_empty(),
                "-n" => !args[1].is_empty(),
                "-z" => args[1].is_empty(),
                _ => !args[0].is_empty(), // fallback: treat as non-empty test
            }
        }
        3 => {
            // Binary operator
            match args[1].as_str() {
                "=" | "==" => args[0] == args[2],
                "!=" => args[0] != args[2],
                "-eq" => parse_i64(&args[0]) == parse_i64(&args[2]),
                "-ne" => parse_i64(&args[0]) != parse_i64(&args[2]),
                "-lt" => parse_i64(&args[0]) < parse_i64(&args[2]),
                "-le" => parse_i64(&args[0]) <= parse_i64(&args[2]),
                "-gt" => parse_i64(&args[0]) > parse_i64(&args[2]),
                "-ge" => parse_i64(&args[0]) >= parse_i64(&args[2]),
                _ => false,
            }
        }
        _ => {
            // Handle `! expr` with 3+ args
            if args[0] == "!" {
                !eval_test(&args[1..])
            } else {
                false
            }
        }
    }
}

fn parse_i64(s: &str) -> i64 {
    let mut n: i64 = 0;
    let mut neg = false;
    let bytes = s.as_bytes();
    let mut i = 0;
    if !bytes.is_empty() && bytes[0] == b'-' {
        neg = true;
        i = 1;
    }
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            n = n.wrapping_mul(10).wrapping_add((bytes[i] - b'0') as i64);
        } else {
            break;
        }
        i += 1;
    }
    if neg {
        n.wrapping_neg()
    } else {
        n
    }
}

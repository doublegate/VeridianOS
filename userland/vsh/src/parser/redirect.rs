//! Redirection parsing.
//!
//! Parses I/O redirections from the token stream: `>`, `>>`, `<`, `<<`,
//! `<<<`, `>&`, `<&`, `>|`, `&>`, `&>>`.

use alloc::string::String;

use crate::{
    error::{Result, VshError},
    lexer::token::TokenKind,
    parser::ast::{Redirect, RedirectOp, RedirectTarget, Word},
};

/// Try to parse a redirection from the current token. Returns `None` if
/// the current token is not a redirection operator.
///
/// `io_number` is an optional file descriptor that preceded the operator.
pub fn parse_redirect(
    token: &TokenKind,
    io_number: Option<i32>,
    next_word: Option<&str>,
) -> Result<Option<Redirect>> {
    let (op, default_fd) = match token {
        TokenKind::Less => (RedirectOp::Input, 0),
        TokenKind::Greater => (RedirectOp::Output, 1),
        TokenKind::DGreater => (RedirectOp::Append, 1),
        TokenKind::DLess => (RedirectOp::HereDoc, 0),
        TokenKind::DLessDash => (RedirectOp::HereDocStrip, 0),
        TokenKind::TLess => (RedirectOp::HereString, 0),
        TokenKind::LessGreater => (RedirectOp::ReadWrite, 0),
        TokenKind::GreaterAnd => (RedirectOp::DupOutput, 1),
        TokenKind::LessAnd => (RedirectOp::DupInput, 0),
        TokenKind::Clobber => (RedirectOp::Clobber, 1),
        TokenKind::AndGreater => (RedirectOp::AndOutput, 1),
        TokenKind::AndDGreater => (RedirectOp::AndAppend, 1),
        _ => return Ok(None),
    };

    let fd = io_number.unwrap_or(default_fd);

    // For here-documents, the target is handled later when the body is
    // collected.
    let target = match op {
        RedirectOp::HereDoc | RedirectOp::HereDocStrip => {
            // The delimiter word was already consumed by the lexer.
            // The body will be filled in during here-doc collection.
            RedirectTarget::HereDocBody(String::new())
        }
        RedirectOp::DupOutput | RedirectOp::DupInput => match next_word {
            Some("-") => RedirectTarget::Close,
            Some(w) => {
                if let Ok(n) = parse_fd_number(w) {
                    RedirectTarget::Fd(n)
                } else {
                    RedirectTarget::File(Word::from_str(w))
                }
            }
            None => {
                return Err(VshError::Syntax(String::from(
                    "expected file descriptor or '-' after redirect",
                )));
            }
        },
        RedirectOp::HereString => match next_word {
            Some(w) => RedirectTarget::HereString(Word::from_str(w)),
            None => {
                return Err(VshError::Syntax(String::from("expected word after <<<")));
            }
        },
        _ => match next_word {
            Some(w) => RedirectTarget::File(Word::from_str(w)),
            None => {
                return Err(VshError::Syntax(String::from(
                    "expected filename after redirect",
                )));
            }
        },
    };

    Ok(Some(Redirect {
        fd: Some(fd),
        op,
        target,
    }))
}

/// Check if a token is a redirection operator.
pub fn is_redirect_op(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Less
            | TokenKind::Greater
            | TokenKind::DGreater
            | TokenKind::DLess
            | TokenKind::DLessDash
            | TokenKind::TLess
            | TokenKind::LessGreater
            | TokenKind::GreaterAnd
            | TokenKind::LessAnd
            | TokenKind::Clobber
            | TokenKind::AndGreater
            | TokenKind::AndDGreater
    )
}

/// Try to parse a string as a file descriptor number.
fn parse_fd_number(s: &str) -> core::result::Result<i32, ()> {
    let mut n: i32 = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return Err(());
        }
        n = n
            .checked_mul(10)
            .ok_or(())?
            .checked_add((b - b'0') as i32)
            .ok_or(())?;
    }
    Ok(n)
}

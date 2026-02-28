//! Eval and string evaluation.
//!
//! Provides `eval_string()` which tokenizes, parses, and executes a shell
//! command string.  Used by the REPL, `eval` builtin, and function bodies.

extern crate alloc;

use crate::{eprintln, error::Result, lexer::Lexer, parser::Parser, Shell};

/// Parse and execute a shell command string.
pub fn eval_string(shell: &mut Shell, input: &str) -> Result<i32> {
    // Tokenize
    let tokens = Lexer::new(input).tokenize();

    if tokens.is_empty() {
        return Ok(0);
    }

    // Check if we only have an EOF token
    if tokens.len() == 1 && tokens[0].kind == crate::lexer::token::TokenKind::Eof {
        return Ok(0);
    }

    // Parse
    let program = match Parser::new(tokens).parse() {
        Ok(prog) => prog,
        Err(e) => {
            eprintln!("vsh: {}", e);
            return Ok(2);
        }
    };

    // Execute
    super::execute_program(shell, &program)
}

//! Recursive descent parser for the vsh shell.
//!
//! Transforms a token stream from the lexer into an AST ([`ast::Program`]).

pub mod arithmetic;
pub mod ast;
pub mod redirect;
pub mod test_expr;
pub mod word;

use alloc::{boxed::Box, string::String, vec::Vec};

use ast::*;
use redirect::{is_redirect_op, parse_redirect};

use crate::{
    error::{Result, VshError},
    lexer::token::{Span, Token, TokenKind},
};

/// The parser state.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    /// Create a new parser for the given token stream.
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    /// Parse the entire input into a program.
    pub fn parse(&mut self) -> Result<Program> {
        let mut commands = Vec::new();
        self.skip_newlines();

        while !self.at_eof() {
            let cmd = self.parse_complete_command()?;
            commands.push(cmd);
            self.skip_newlines();
        }

        Ok(Program { commands })
    }

    // -----------------------------------------------------------------------
    // Token access
    // -----------------------------------------------------------------------

    fn peek(&self) -> &TokenKind {
        self.tokens
            .get(self.pos)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    #[allow(dead_code)] // Used for error reporting with span locations
    fn peek_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or_default()
    }

    fn advance(&mut self) -> &TokenKind {
        let tok = self
            .tokens
            .get(self.pos)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof);
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    fn expect(&mut self, expected: &TokenKind) -> Result<()> {
        let actual = self.peek().clone();
        if &actual == expected {
            self.advance();
            Ok(())
        } else {
            Err(VshError::Syntax(alloc::format!(
                "expected {:?}, got {:?}",
                expected,
                actual
            )))
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), TokenKind::Newline) {
            self.advance();
        }
    }

    fn at_command_terminator(&self) -> bool {
        matches!(
            self.peek(),
            TokenKind::Newline
                | TokenKind::Eof
                | TokenKind::Semi
                | TokenKind::Ampersand
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::Pipe
                | TokenKind::PipeAmpersand
                | TokenKind::RParen
                | TokenKind::RBrace
                | TokenKind::DoubleSemi
                | TokenKind::SemiAnd
                | TokenKind::SemiSemiAnd
        )
    }

    // -----------------------------------------------------------------------
    // Grammar rules
    // -----------------------------------------------------------------------

    /// complete_command : and_or_list (';' | '&' | newline)
    fn parse_complete_command(&mut self) -> Result<CompleteCommand> {
        let list = self.parse_and_or_list()?;

        let background = match self.peek() {
            TokenKind::Ampersand => {
                self.advance();
                true
            }
            TokenKind::Semi | TokenKind::Newline => {
                self.advance();
                false
            }
            _ => false,
        };

        Ok(CompleteCommand { list, background })
    }

    /// and_or_list : pipeline ( ('&&' | '||') pipeline )*
    fn parse_and_or_list(&mut self) -> Result<AndOrList> {
        let first = self.parse_pipeline()?;
        let mut rest = Vec::new();

        loop {
            self.skip_newlines();
            match self.peek() {
                TokenKind::And => {
                    self.advance();
                    self.skip_newlines();
                    let pipe = self.parse_pipeline()?;
                    rest.push((AndOrOp::And, pipe));
                }
                TokenKind::Or => {
                    self.advance();
                    self.skip_newlines();
                    let pipe = self.parse_pipeline()?;
                    rest.push((AndOrOp::Or, pipe));
                }
                _ => break,
            }
        }

        Ok(AndOrList { first, rest })
    }

    /// pipeline : ['time'] ['!'] command ( '|' command )*
    fn parse_pipeline(&mut self) -> Result<Pipeline> {
        let timed = matches!(self.peek(), TokenKind::Time);
        if timed {
            self.advance();
        }

        let negated = matches!(self.peek(), TokenKind::Bang);
        if negated {
            self.advance();
        }

        let mut commands = Vec::new();
        let mut pipe_stderr = false;

        commands.push(self.parse_command()?);

        loop {
            match self.peek() {
                TokenKind::Pipe => {
                    self.advance();
                    self.skip_newlines();
                    commands.push(self.parse_command()?);
                }
                TokenKind::PipeAmpersand => {
                    self.advance();
                    pipe_stderr = true;
                    self.skip_newlines();
                    commands.push(self.parse_command()?);
                }
                _ => break,
            }
        }

        Ok(Pipeline {
            negated,
            pipe_stderr,
            commands,
            timed,
        })
    }

    /// command : compound_command redirects
    ///         | function_def
    ///         | coproc
    ///         | simple_command
    fn parse_command(&mut self) -> Result<Command> {
        match self.peek() {
            // Compound commands
            TokenKind::If => self.parse_if_command(),
            TokenKind::While => self.parse_while_command(),
            TokenKind::Until => self.parse_until_command(),
            TokenKind::For => self.parse_for_command(),
            TokenKind::Case => self.parse_case_command(),
            TokenKind::Select => self.parse_select_command(),
            TokenKind::LBrace => self.parse_brace_group(),
            TokenKind::LParen => self.parse_subshell(),
            TokenKind::DLBracket => self.parse_conditional_expr(),
            TokenKind::Function => self.parse_function_keyword(),
            TokenKind::Coproc => self.parse_coproc(),
            _ => {
                // Check for function definition: NAME () { body }
                if self.is_function_def() {
                    self.parse_function_def()
                } else {
                    self.parse_simple_command()
                }
            }
        }
    }

    /// simple_command : (assignment | word | redirect)*
    fn parse_simple_command(&mut self) -> Result<Command> {
        let mut cmd = SimpleCommand {
            assignments: Vec::new(),
            words: Vec::new(),
            redirects: Vec::new(),
        };

        loop {
            match self.peek().clone() {
                TokenKind::Assignment(name, value) => {
                    self.advance();
                    cmd.assignments.push(Assignment {
                        name,
                        value: Word::new(value),
                        append: false,
                    });
                }
                TokenKind::Word(w) => {
                    // Check if the next token after this word is a redirection
                    // operator with this word being an IO number
                    let w_clone = w.clone();
                    if let Some(fd) = try_parse_io_number(&w_clone) {
                        if is_redirect_op(self.peek_next()) {
                            self.advance(); // consume the io number word
                            let redir = self.parse_redirect_with_fd(Some(fd))?;
                            cmd.redirects.push(redir);
                            continue;
                        }
                    }
                    self.advance();
                    cmd.words.push(Word::new(w_clone));
                }
                ref kind if is_redirect_op(kind) => {
                    let redir = self.parse_redirect_with_fd(None)?;
                    cmd.redirects.push(redir);
                }
                _ => break,
            }
        }

        Ok(Command::Simple(cmd))
    }

    fn peek_next(&self) -> &TokenKind {
        self.tokens
            .get(self.pos + 1)
            .map(|t| &t.kind)
            .unwrap_or(&TokenKind::Eof)
    }

    fn parse_redirect_with_fd(&mut self, fd: Option<i32>) -> Result<Redirect> {
        let op_kind = self.peek().clone();
        self.advance(); // consume operator

        // Get the target word
        let target_word = match self.peek() {
            TokenKind::Word(w) => {
                let w = w.clone();
                self.advance();
                Some(w)
            }
            _ => None,
        };

        let target_str = target_word.as_deref();
        match parse_redirect(&op_kind, fd, target_str)? {
            Some(redir) => Ok(redir),
            None => Err(VshError::Syntax(String::from("invalid redirection"))),
        }
    }

    // -----------------------------------------------------------------------
    // Compound command parsers
    // -----------------------------------------------------------------------

    fn parse_if_command(&mut self) -> Result<Command> {
        self.expect(&TokenKind::If)?;
        let mut branches = Vec::new();

        // if condition
        let cond = self.parse_compound_list()?;
        self.expect(&TokenKind::Then)?;
        let body = self.parse_compound_list()?;
        branches.push((cond, body));

        // elif branches
        while matches!(self.peek(), TokenKind::Elif) {
            self.advance();
            let cond = self.parse_compound_list()?;
            self.expect(&TokenKind::Then)?;
            let body = self.parse_compound_list()?;
            branches.push((cond, body));
        }

        // else branch
        let else_body = if matches!(self.peek(), TokenKind::Else) {
            self.advance();
            Some(self.parse_compound_list()?)
        } else {
            None
        };

        self.expect(&TokenKind::Fi)?;
        let redirects = self.parse_optional_redirects()?;

        Ok(Command::Compound(
            CompoundCommand::If(IfClause {
                branches,
                else_body,
            }),
            redirects,
        ))
    }

    fn parse_while_command(&mut self) -> Result<Command> {
        self.expect(&TokenKind::While)?;
        let condition = self.parse_compound_list()?;
        self.expect(&TokenKind::Do)?;
        let body = self.parse_compound_list()?;
        self.expect(&TokenKind::Done)?;
        let redirects = self.parse_optional_redirects()?;

        Ok(Command::Compound(
            CompoundCommand::While(WhileClause { condition, body }),
            redirects,
        ))
    }

    fn parse_until_command(&mut self) -> Result<Command> {
        self.expect(&TokenKind::Until)?;
        let condition = self.parse_compound_list()?;
        self.expect(&TokenKind::Do)?;
        let body = self.parse_compound_list()?;
        self.expect(&TokenKind::Done)?;
        let redirects = self.parse_optional_redirects()?;

        Ok(Command::Compound(
            CompoundCommand::Until(UntilClause { condition, body }),
            redirects,
        ))
    }

    fn parse_for_command(&mut self) -> Result<Command> {
        self.expect(&TokenKind::For)?;

        // Check for arithmetic for: for ((init; cond; step))
        if matches!(self.peek(), TokenKind::LParen) {
            // Simplified: just parse as a regular for for now
        }

        let var = match self.peek() {
            TokenKind::Word(w) => {
                let w = w.clone();
                self.advance();
                w
            }
            _ => {
                return Err(VshError::Syntax(String::from(
                    "expected variable name after 'for'",
                )))
            }
        };

        self.skip_newlines();

        let words = if matches!(self.peek(), TokenKind::In) {
            self.advance();
            let mut word_list = Vec::new();
            while !self.at_command_terminator() && !matches!(self.peek(), TokenKind::Do) {
                match self.peek() {
                    TokenKind::Word(w) => {
                        let w = w.clone();
                        self.advance();
                        word_list.push(Word::new(w));
                    }
                    _ => break,
                }
            }
            // Consume the terminator (`;` or newline) before `do`
            if matches!(self.peek(), TokenKind::Semi | TokenKind::Newline) {
                self.advance();
            }
            self.skip_newlines();
            Some(word_list)
        } else {
            // No `in` clause: iterate over "$@"
            if matches!(self.peek(), TokenKind::Semi | TokenKind::Newline) {
                self.advance();
            }
            self.skip_newlines();
            None
        };

        self.expect(&TokenKind::Do)?;
        let body = self.parse_compound_list()?;
        self.expect(&TokenKind::Done)?;
        let redirects = self.parse_optional_redirects()?;

        Ok(Command::Compound(
            CompoundCommand::For(ForClause { var, words, body }),
            redirects,
        ))
    }

    fn parse_case_command(&mut self) -> Result<Command> {
        self.expect(&TokenKind::Case)?;

        let word = match self.peek() {
            TokenKind::Word(w) => {
                let w = w.clone();
                self.advance();
                Word::new(w)
            }
            _ => return Err(VshError::Syntax(String::from("expected word after 'case'"))),
        };

        self.skip_newlines();
        self.expect(&TokenKind::In)?;
        self.skip_newlines();

        let mut items = Vec::new();

        while !matches!(self.peek(), TokenKind::Esac | TokenKind::Eof) {
            // Skip optional `(`
            if matches!(self.peek(), TokenKind::LParen) {
                self.advance();
            }

            // Parse patterns separated by `|`
            let mut patterns = Vec::new();
            while let TokenKind::Word(w) = self.peek() {
                let w = w.clone();
                self.advance();
                patterns.push(Word::new(w));
                if matches!(self.peek(), TokenKind::Pipe) {
                    self.advance();
                } else {
                    break;
                }
            }

            // Expect `)`
            self.expect(&TokenKind::RParen)?;

            // Parse body
            let body = self.parse_compound_list()?;

            // Parse terminator
            let terminator = match self.peek() {
                TokenKind::DoubleSemi => {
                    self.advance();
                    CaseTerminator::Break
                }
                TokenKind::SemiAnd => {
                    self.advance();
                    CaseTerminator::FallThrough
                }
                TokenKind::SemiSemiAnd => {
                    self.advance();
                    CaseTerminator::TestNext
                }
                _ => CaseTerminator::Break,
            };

            self.skip_newlines();
            items.push(CaseItem {
                patterns,
                body,
                terminator,
            });
        }

        self.expect(&TokenKind::Esac)?;
        let redirects = self.parse_optional_redirects()?;

        Ok(Command::Compound(
            CompoundCommand::Case(CaseClause { word, items }),
            redirects,
        ))
    }

    fn parse_select_command(&mut self) -> Result<Command> {
        self.expect(&TokenKind::Select)?;

        let var = match self.peek() {
            TokenKind::Word(w) => {
                let w = w.clone();
                self.advance();
                w
            }
            _ => {
                return Err(VshError::Syntax(String::from(
                    "expected variable name after 'select'",
                )))
            }
        };

        self.skip_newlines();

        let words = if matches!(self.peek(), TokenKind::In) {
            self.advance();
            let mut word_list = Vec::new();
            while !self.at_command_terminator() && !matches!(self.peek(), TokenKind::Do) {
                match self.peek() {
                    TokenKind::Word(w) => {
                        let w = w.clone();
                        self.advance();
                        word_list.push(Word::new(w));
                    }
                    _ => break,
                }
            }
            if matches!(self.peek(), TokenKind::Semi | TokenKind::Newline) {
                self.advance();
            }
            self.skip_newlines();
            Some(word_list)
        } else {
            None
        };

        self.expect(&TokenKind::Do)?;
        let body = self.parse_compound_list()?;
        self.expect(&TokenKind::Done)?;
        let redirects = self.parse_optional_redirects()?;

        Ok(Command::Compound(
            CompoundCommand::Select(SelectClause { var, words, body }),
            redirects,
        ))
    }

    fn parse_brace_group(&mut self) -> Result<Command> {
        self.expect(&TokenKind::LBrace)?;
        let body = self.parse_compound_list()?;
        self.expect(&TokenKind::RBrace)?;
        let redirects = self.parse_optional_redirects()?;

        Ok(Command::Compound(
            CompoundCommand::BraceGroup(body),
            redirects,
        ))
    }

    fn parse_subshell(&mut self) -> Result<Command> {
        self.expect(&TokenKind::LParen)?;
        let body = self.parse_compound_list()?;
        self.expect(&TokenKind::RParen)?;
        let redirects = self.parse_optional_redirects()?;

        Ok(Command::Compound(
            CompoundCommand::Subshell(body),
            redirects,
        ))
    }

    fn parse_conditional_expr(&mut self) -> Result<Command> {
        self.expect(&TokenKind::DLBracket)?;

        let mut words = Vec::new();
        while !matches!(self.peek(), TokenKind::DRBracket | TokenKind::Eof) {
            match self.peek() {
                TokenKind::Word(w) => {
                    words.push(w.clone());
                    self.advance();
                }
                _ => {
                    // Convert operator tokens to their string form
                    let s = match self.peek() {
                        TokenKind::And => String::from("&&"),
                        TokenKind::Or => String::from("||"),
                        TokenKind::Bang => String::from("!"),
                        TokenKind::LParen => String::from("("),
                        TokenKind::RParen => String::from(")"),
                        TokenKind::Less => String::from("<"),
                        TokenKind::Greater => String::from(">"),
                        _ => break,
                    };
                    self.advance();
                    words.push(s);
                }
            }
        }

        self.expect(&TokenKind::DRBracket)?;
        let redirects = self.parse_optional_redirects()?;

        let expr = test_expr::parse_conditional(&words);
        Ok(Command::Compound(
            CompoundCommand::ConditionalExpr(expr),
            redirects,
        ))
    }

    // -----------------------------------------------------------------------
    // Function definitions
    // -----------------------------------------------------------------------

    fn is_function_def(&self) -> bool {
        // NAME () { ... }
        if let TokenKind::Word(_) = self.peek() {
            if let Some(next) = self.tokens.get(self.pos + 1) {
                return next.kind == TokenKind::LParen;
            }
        }
        false
    }

    fn parse_function_def(&mut self) -> Result<Command> {
        let name = match self.peek() {
            TokenKind::Word(w) => {
                let w = w.clone();
                self.advance();
                w
            }
            _ => return Err(VshError::Syntax(String::from("expected function name"))),
        };

        self.expect(&TokenKind::LParen)?;
        self.expect(&TokenKind::RParen)?;
        self.skip_newlines();

        let body = self.parse_command()?;

        Ok(Command::FunctionDef(FunctionDef {
            name,
            body: Box::new(body),
        }))
    }

    fn parse_function_keyword(&mut self) -> Result<Command> {
        self.expect(&TokenKind::Function)?;

        let name = match self.peek() {
            TokenKind::Word(w) => {
                let w = w.clone();
                self.advance();
                w
            }
            _ => {
                return Err(VshError::Syntax(String::from(
                    "expected function name after 'function'",
                )))
            }
        };

        // Optional `()`
        if matches!(self.peek(), TokenKind::LParen) {
            self.advance();
            self.expect(&TokenKind::RParen)?;
        }

        self.skip_newlines();
        let body = self.parse_command()?;

        Ok(Command::FunctionDef(FunctionDef {
            name,
            body: Box::new(body),
        }))
    }

    fn parse_coproc(&mut self) -> Result<Command> {
        self.expect(&TokenKind::Coproc)?;

        // Optional name
        let (name, cmd) = if let TokenKind::Word(w) = self.peek() {
            let w = w.clone();
            // Check if this is the name (followed by a command) or the command itself
            if let Some(next) = self.tokens.get(self.pos + 1) {
                if !next.kind.is_terminator() && next.kind != TokenKind::Eof {
                    self.advance();
                    let cmd = self.parse_command()?;
                    (Some(w), cmd)
                } else {
                    let cmd = self.parse_command()?;
                    (None, cmd)
                }
            } else {
                let cmd = self.parse_command()?;
                (None, cmd)
            }
        } else {
            let cmd = self.parse_command()?;
            (None, cmd)
        };

        Ok(Command::Coproc(CoprocCommand {
            name,
            body: Box::new(cmd),
        }))
    }

    // -----------------------------------------------------------------------
    // Helper: compound list (list of complete commands up to a delimiter)
    // -----------------------------------------------------------------------

    fn parse_compound_list(&mut self) -> Result<Vec<CompleteCommand>> {
        let mut commands = Vec::new();
        self.skip_newlines();

        while !self.at_compound_list_end() {
            let cmd = self.parse_complete_command()?;
            commands.push(cmd);
            self.skip_newlines();
        }

        Ok(commands)
    }

    fn at_compound_list_end(&self) -> bool {
        matches!(
            self.peek(),
            TokenKind::Eof
                | TokenKind::Fi
                | TokenKind::Then
                | TokenKind::Else
                | TokenKind::Elif
                | TokenKind::Done
                | TokenKind::Esac
                | TokenKind::RBrace
                | TokenKind::RParen
                | TokenKind::DRBracket
                | TokenKind::DoubleSemi
                | TokenKind::SemiAnd
                | TokenKind::SemiSemiAnd
        )
    }

    fn parse_optional_redirects(&mut self) -> Result<Vec<Redirect>> {
        let mut redirects = Vec::new();
        while is_redirect_op(self.peek()) {
            let redir = self.parse_redirect_with_fd(None)?;
            redirects.push(redir);
        }
        Ok(redirects)
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Try to parse a word as an IO number (single digit for redirection).
fn try_parse_io_number(word: &str) -> Option<i32> {
    if word.len() == 1 {
        let b = word.as_bytes()[0];
        if b.is_ascii_digit() {
            return Some((b - b'0') as i32);
        }
    }
    None
}

/// Convenience: parse a string into an AST Program.
#[allow(dead_code)] // Public convenience API
pub fn parse_input(input: &str) -> Result<Program> {
    let mut lexer = crate::lexer::Lexer::new(input);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);
    parser.parse()
}

impl TokenKind {
    fn is_terminator(&self) -> bool {
        matches!(self, TokenKind::Newline | TokenKind::Eof | TokenKind::Semi)
    }
}

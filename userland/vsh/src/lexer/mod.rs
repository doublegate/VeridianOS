//! Lexer for the vsh shell.
//!
//! Transforms a raw input string into a sequence of [`Token`]s.  Handles
//! all bash operators, quoting, here-documents, and here-strings.

pub mod heredoc;
pub mod quote;
pub mod token;

use alloc::{string::String, vec::Vec};

use heredoc::PendingHereDoc;
use quote::{QuoteAction, QuoteState};
use token::{Span, Token, TokenKind};

/// The lexer state machine.
pub struct Lexer {
    /// Input characters.
    chars: Vec<char>,
    /// Current position in `chars`.
    pos: usize,
    /// Current line number (1-based).
    line: u32,
    /// Current column number (1-based).
    col: u32,
    /// Pending here-documents to collect.
    pub pending_heredocs: Vec<PendingHereDoc>,
    /// Quote tracking state.
    #[allow(dead_code)] // Will be used for multi-line input tracking
    quote_state: QuoteState,
}

impl Lexer {
    /// Create a new lexer for the given input.
    pub fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            pending_heredocs: Vec::new(),
            quote_state: QuoteState::new(),
        }
    }

    /// Tokenize the entire input and return all tokens.
    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            let is_eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        tokens
    }

    /// Current position as a span.
    fn span(&self) -> Span {
        Span {
            line: self.line,
            col: self.col,
        }
    }

    /// Peek at the current character without consuming it.
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// Peek at the character at offset `n` from current position.
    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    /// Consume and return the current character.
    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    /// Skip whitespace (spaces and tabs, NOT newlines).
    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Skip a comment (from `#` to end of line).
    fn skip_comment(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == '\n' {
                break;
            }
            self.advance();
        }
    }

    /// Produce the next token.
    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace();

        let span = self.span();

        match self.peek() {
            None => Token::new(TokenKind::Eof, span),
            Some('#') => {
                self.skip_comment();
                self.next_token()
            }
            Some('\n') => {
                self.advance();
                Token::new(TokenKind::Newline, span)
            }
            Some(_ch) => {
                // Check for operators first
                if let Some(tok) = self.try_operator(span) {
                    return tok;
                }
                // Otherwise, read a word
                self.read_word(span)
            }
        }
    }

    /// Try to read an operator token. Returns `None` if the current position
    /// does not start an operator.
    fn try_operator(&mut self, span: Span) -> Option<Token> {
        let c0 = self.peek()?;
        let c1 = self.peek_at(1);
        let c2 = self.peek_at(2);

        let (kind, consume) = match (c0, c1, c2) {
            // Three-character operators
            ('<', Some('<'), Some('<')) => (TokenKind::TLess, 3),
            ('<', Some('<'), Some('-')) => (TokenKind::DLessDash, 3),
            ('&', Some('>'), Some('>')) => (TokenKind::AndDGreater, 3),
            (';', Some(';'), Some('&')) => (TokenKind::SemiSemiAnd, 3),

            // Two-character operators
            ('|', Some('|'), _) => (TokenKind::Or, 2),
            ('|', Some('&'), _) => (TokenKind::PipeAmpersand, 2),
            ('&', Some('&'), _) => (TokenKind::And, 2),
            ('&', Some('>'), _) => (TokenKind::AndGreater, 2),
            (';', Some(';'), _) => (TokenKind::DoubleSemi, 2),
            (';', Some('&'), _) => (TokenKind::SemiAnd, 2),
            ('<', Some('<'), _) => (TokenKind::DLess, 2),
            ('<', Some('>'), _) => (TokenKind::LessGreater, 2),
            ('<', Some('&'), _) => (TokenKind::LessAnd, 2),
            ('<', Some('('), _) => (TokenKind::ProcSubIn, 2),
            ('>', Some('>'), _) => (TokenKind::DGreater, 2),
            ('>', Some('&'), _) => (TokenKind::GreaterAnd, 2),
            ('>', Some('|'), _) => (TokenKind::Clobber, 2),
            ('>', Some('('), _) => (TokenKind::ProcSubOut, 2),

            // Single-character operators
            ('|', _, _) => (TokenKind::Pipe, 1),
            ('&', _, _) => (TokenKind::Ampersand, 1),
            (';', _, _) => (TokenKind::Semi, 1),
            ('(', _, _) => (TokenKind::LParen, 1),
            (')', _, _) => (TokenKind::RParen, 1),
            ('<', _, _) => (TokenKind::Less, 1),
            ('>', _, _) => (TokenKind::Greater, 1),

            _ => return None,
        };

        for _ in 0..consume {
            self.advance();
        }

        // Handle here-document: after `<<` or `<<-`, read the delimiter word
        if kind == TokenKind::DLess || kind == TokenKind::DLessDash {
            self.skip_whitespace();
            let _delim_span = self.span();
            let delim_word = self.read_raw_word();
            let strip_tabs = kind == TokenKind::DLessDash;
            self.pending_heredocs
                .push(PendingHereDoc::new(&delim_word, strip_tabs));
        }

        Some(Token::new(kind, span))
    }

    /// Read a word token. A word is a sequence of characters that is not
    /// whitespace or an unquoted metacharacter.
    fn read_word(&mut self, span: Span) -> Token {
        let mut word = String::new();
        let mut quote_state = QuoteState::new();

        while let Some(ch) = self.peek() {
            // If unquoted, check for metacharacters that end the word
            if !quote_state.is_quoted() {
                if is_metachar(ch) {
                    break;
                }
                // Backslash-newline continuation
                if ch == '\\' && self.peek_at(1) == Some('\n') {
                    self.advance(); // skip backslash
                    self.advance(); // skip newline
                    continue;
                }
                // Regular backslash escape
                if ch == '\\' {
                    self.advance(); // skip backslash
                    word.push('\\');
                    if let Some(escaped) = self.advance() {
                        word.push(escaped);
                    }
                    continue;
                }
            }

            let peek = self.peek_at(1);
            let peek2 = self.peek_at(2);
            let action = quote_state.process(ch, peek, peek2);

            match action {
                QuoteAction::Literal => {
                    word.push(ch);
                    self.advance();
                }
                QuoteAction::Consumed => {
                    // Quote character consumed (e.g., opening/closing quote)
                    // but we still track it in the word for later expansion
                    word.push(ch);
                    self.advance();
                }
                QuoteAction::EscapeNext => {
                    word.push(ch); // the backslash
                    self.advance();
                    if let Some(next) = self.advance() {
                        word.push(next);
                    }
                }
                QuoteAction::StartCmdSub => {
                    word.push('$');
                    word.push('(');
                    self.advance(); // $
                    self.advance(); // (
                }
                QuoteAction::StartArithSub => {
                    word.push('$');
                    word.push('(');
                    word.push('(');
                    self.advance(); // $
                    self.advance(); // (
                    self.advance(); // (
                }
                QuoteAction::StartParamExp => {
                    word.push('$');
                    word.push('{');
                    self.advance(); // $
                    self.advance(); // {
                }
                QuoteAction::EndArithSub => {
                    word.push(')');
                    word.push(')');
                    self.advance(); // )
                    self.advance(); // )
                }
            }
        }

        if word.is_empty() {
            return Token::new(TokenKind::Eof, span);
        }

        // Check if this word is an assignment (NAME=VALUE)
        if let Some(eq_pos) = find_assignment_eq(&word) {
            let name = String::from(&word[..eq_pos]);
            let value = String::from(&word[eq_pos + 1..]);
            return Token::new(TokenKind::Assignment(name, value), span);
        }

        // Check if this is a reserved word (only in command position)
        if let Some(reserved) = TokenKind::reserved_word(&word) {
            return Token::new(reserved, span);
        }

        // Check for `{` and `}` as words
        if word == "{" {
            return Token::new(TokenKind::LBrace, span);
        }
        if word == "}" {
            return Token::new(TokenKind::RBrace, span);
        }

        Token::new(TokenKind::Word(word), span)
    }

    /// Read a raw word (for here-document delimiter, etc.) without quote
    /// processing.
    fn read_raw_word(&mut self) -> String {
        let mut word = String::new();
        loop {
            match self.peek() {
                Some(ch) if ch == ' ' || ch == '\t' || ch == '\n' => break,
                Some(ch) if is_metachar(ch) && ch != '\'' && ch != '"' => break,
                Some(ch) => {
                    word.push(ch);
                    self.advance();
                }
                None => break,
            }
        }
        word
    }
}

/// Returns true if `ch` is a shell metacharacter.
fn is_metachar(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t' | '\n' | '|' | '&' | ';' | '(' | ')' | '<' | '>'
    )
}

/// Check if a word is an assignment of the form `NAME=VALUE`.
/// Returns the position of `=` if it is, or `None`.
fn find_assignment_eq(word: &str) -> Option<usize> {
    let bytes = word.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    // First character must be a letter or underscore
    if !is_var_start(bytes[0]) {
        return None;
    }
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'=' {
            return Some(i);
        }
        if i > 0 && !is_var_char(b) {
            return None;
        }
    }
    None
}

fn is_var_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_var_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

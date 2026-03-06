//! JavaScript Lexer
//!
//! Tokenizes JavaScript source code into a stream of tokens. Handles
//! string escapes, number parsing (integer and decimal to 32.32 fixed-point),
//! and automatic semicolon insertion (ASI).

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

/// JavaScript number type: 32.32 fixed-point (i64)
pub type JsNumber = i64;

/// Fractional bits for JsNumber
pub const JS_FRAC_BITS: u32 = 32;

/// Convert integer to JsNumber
#[inline]
pub const fn js_int(v: i64) -> JsNumber {
    v << JS_FRAC_BITS
}

/// Convert JsNumber to integer (truncate)
#[inline]
pub const fn js_to_int(v: JsNumber) -> i64 {
    v >> JS_FRAC_BITS
}

/// Check if a JsNumber has no fractional part
#[inline]
pub const fn js_is_integer(v: JsNumber) -> bool {
    (v & ((1i64 << JS_FRAC_BITS) - 1)) == 0
}

/// JsNumber zero
pub const JS_ZERO: JsNumber = 0;

/// JsNumber one
pub const JS_ONE: JsNumber = 1i64 << JS_FRAC_BITS;

/// JsNumber NaN sentinel (max i64 -- not a real number)
pub const JS_NAN: JsNumber = i64::MAX;

// ---------------------------------------------------------------------------
// Token type
// ---------------------------------------------------------------------------

/// JavaScript token
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum JsToken {
    // Literals
    Identifier(String),
    Number(JsNumber),
    StringLiteral(String),
    Bool(bool),
    Null,
    Undefined,

    // Arithmetic operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,

    // Comparison / equality
    Eq,
    EqEq,
    EqEqEq,
    NotEq,
    NotEqEq,
    Lt,
    Gt,
    LtEq,
    GtEq,

    // Logical
    And,
    Or,
    Not,

    // Bitwise
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,

    // Assignment
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,

    // Delimiters
    OpenParen,
    CloseParen,
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,

    // Punctuation
    Dot,
    Comma,
    Semicolon,
    Colon,
    Question,
    Arrow,
    Spread,

    // Keywords
    Let,
    Const,
    Var,
    Function,
    Return,
    If,
    Else,
    While,
    For,
    Break,
    Continue,
    New,
    This,
    Typeof,
    Instanceof,
    In,
    Of,
    Delete,
    Void,
    Throw,
    Try,
    Catch,
    Finally,
    Switch,
    Case,
    Default,
    Class,
    Extends,
    Super,
    Import,
    Export,
    True,
    False,

    // Special
    Eof,
}

impl JsToken {
    /// Whether this token can end a statement (for ASI purposes)
    pub fn can_end_statement(&self) -> bool {
        matches!(
            self,
            Self::Identifier(_)
                | Self::Number(_)
                | Self::StringLiteral(_)
                | Self::Bool(_)
                | Self::Null
                | Self::Undefined
                | Self::True
                | Self::False
                | Self::This
                | Self::CloseParen
                | Self::CloseBracket
                | Self::Return
                | Self::Break
                | Self::Continue
        )
    }

    /// Whether this token is a keyword
    pub fn is_keyword(&self) -> bool {
        matches!(
            self,
            Self::Let
                | Self::Const
                | Self::Var
                | Self::Function
                | Self::Return
                | Self::If
                | Self::Else
                | Self::While
                | Self::For
                | Self::Break
                | Self::Continue
                | Self::New
                | Self::This
                | Self::Typeof
                | Self::Instanceof
                | Self::In
                | Self::Of
                | Self::Delete
                | Self::Void
                | Self::Throw
                | Self::Try
                | Self::Catch
                | Self::Finally
                | Self::Switch
                | Self::Case
                | Self::Default
                | Self::Class
                | Self::Extends
                | Self::Super
                | Self::Import
                | Self::Export
                | Self::True
                | Self::False
        )
    }
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

/// JavaScript lexer
#[allow(dead_code)]
pub struct JsLexer {
    /// Input source as bytes
    input: Vec<u8>,
    /// Current position
    pos: usize,
    /// Current line number (1-based)
    pub line: u32,
    /// Current column (1-based)
    pub col: u32,
    /// Whether the previous token could end a statement (for ASI)
    prev_can_end: bool,
    /// Whether we crossed a newline since the last token
    newline_before: bool,
}

impl JsLexer {
    pub fn new(source: &str) -> Self {
        Self {
            input: source.as_bytes().to_vec(),
            pos: 0,
            line: 1,
            col: 1,
            prev_can_end: false,
            newline_before: false,
        }
    }

    /// Tokenize the entire input into a Vec
    pub fn tokenize_all(&mut self) -> Vec<JsToken> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            if tok == JsToken::Eof {
                tokens.push(tok);
                break;
            }
            tokens.push(tok);
        }
        tokens
    }

    /// Get the next token
    pub fn next_token(&mut self) -> JsToken {
        self.newline_before = false;
        self.skip_whitespace_and_comments();

        // ASI: if we crossed a newline and the previous token can end
        // a statement, insert a semicolon
        if self.newline_before && self.prev_can_end {
            self.prev_can_end = false;
            return JsToken::Semicolon;
        }

        if self.pos >= self.input.len() {
            return JsToken::Eof;
        }

        let ch = self.input[self.pos];
        let tok = match ch {
            b'(' => {
                self.advance();
                JsToken::OpenParen
            }
            b')' => {
                self.advance();
                JsToken::CloseParen
            }
            b'{' => {
                self.advance();
                JsToken::OpenBrace
            }
            b'}' => {
                self.advance();
                JsToken::CloseBrace
            }
            b'[' => {
                self.advance();
                JsToken::OpenBracket
            }
            b']' => {
                self.advance();
                JsToken::CloseBracket
            }
            b',' => {
                self.advance();
                JsToken::Comma
            }
            b';' => {
                self.advance();
                JsToken::Semicolon
            }
            b':' => {
                self.advance();
                JsToken::Colon
            }
            b'?' => {
                self.advance();
                JsToken::Question
            }
            b'~' => {
                self.advance();
                JsToken::BitXor
            } // simplified
            b'.' => {
                if self.peek_at(1) == Some(b'.') && self.peek_at(2) == Some(b'.') {
                    self.advance();
                    self.advance();
                    self.advance();
                    JsToken::Spread
                } else {
                    self.advance();
                    JsToken::Dot
                }
            }

            b'+' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    JsToken::PlusAssign
                } else {
                    JsToken::Plus
                }
            }
            b'-' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    JsToken::MinusAssign
                } else {
                    JsToken::Minus
                }
            }
            b'*' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    JsToken::StarAssign
                } else {
                    JsToken::Star
                }
            }
            b'/' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    JsToken::SlashAssign
                } else {
                    JsToken::Slash
                }
            }
            b'%' => {
                self.advance();
                JsToken::Percent
            }

            b'=' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    if self.peek() == Some(b'=') {
                        self.advance();
                        JsToken::EqEqEq
                    } else {
                        JsToken::EqEq
                    }
                } else if self.peek() == Some(b'>') {
                    self.advance();
                    JsToken::Arrow
                } else {
                    JsToken::Assign
                }
            }

            b'!' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    if self.peek() == Some(b'=') {
                        self.advance();
                        JsToken::NotEqEq
                    } else {
                        JsToken::NotEq
                    }
                } else {
                    JsToken::Not
                }
            }

            b'<' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    JsToken::LtEq
                } else if self.peek() == Some(b'<') {
                    self.advance();
                    JsToken::ShiftLeft
                } else {
                    JsToken::Lt
                }
            }
            b'>' => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    JsToken::GtEq
                } else if self.peek() == Some(b'>') {
                    self.advance();
                    JsToken::ShiftRight
                } else {
                    JsToken::Gt
                }
            }

            b'&' => {
                self.advance();
                if self.peek() == Some(b'&') {
                    self.advance();
                    JsToken::And
                } else {
                    JsToken::BitAnd
                }
            }
            b'|' => {
                self.advance();
                if self.peek() == Some(b'|') {
                    self.advance();
                    JsToken::Or
                } else {
                    JsToken::BitOr
                }
            }
            b'^' => {
                self.advance();
                JsToken::BitXor
            }

            b'"' | b'\'' => self.read_string(ch),

            b'0'..=b'9' => self.read_number(),

            _ if is_ident_start(ch) => self.read_identifier(),

            _ => {
                self.advance();
                // Skip unknown characters
                return self.next_token();
            }
        };

        self.prev_can_end = tok.can_end_statement();
        tok
    }

    // -- Internal methods --

    fn advance(&mut self) {
        if self.pos < self.input.len() {
            if self.input[self.pos] == b'\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.input.get(self.pos + offset).copied()
    }

    fn skip_whitespace_and_comments(&mut self) {
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            match ch {
                b' ' | b'\t' | b'\r' => {
                    self.advance();
                }
                b'\n' => {
                    self.newline_before = true;
                    self.advance();
                }
                b'/' if self.peek_at(1) == Some(b'/') => {
                    // Single-line comment
                    while self.pos < self.input.len() && self.input[self.pos] != b'\n' {
                        self.advance();
                    }
                }
                b'/' if self.peek_at(1) == Some(b'*') => {
                    // Multi-line comment
                    self.advance(); // /
                    self.advance(); // *
                    while self.pos + 1 < self.input.len() {
                        if self.input[self.pos] == b'*' && self.input[self.pos + 1] == b'/' {
                            self.advance(); // *
                            self.advance(); // /
                            break;
                        }
                        if self.input[self.pos] == b'\n' {
                            self.newline_before = true;
                        }
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    fn read_string(&mut self, quote: u8) -> JsToken {
        self.advance(); // opening quote
        let mut s = String::new();
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            if ch == quote {
                self.advance();
                return JsToken::StringLiteral(s);
            }
            if ch == b'\\' {
                self.advance();
                if self.pos >= self.input.len() {
                    break;
                }
                let esc = self.input[self.pos];
                match esc {
                    b'n' => s.push('\n'),
                    b't' => s.push('\t'),
                    b'r' => s.push('\r'),
                    b'\\' => s.push('\\'),
                    b'\'' => s.push('\''),
                    b'"' => s.push('"'),
                    b'0' => s.push('\0'),
                    b'x' => {
                        // \xHH
                        self.advance();
                        let hi = self.hex_digit().unwrap_or(0);
                        self.advance();
                        let lo = self.hex_digit().unwrap_or(0);
                        self.advance();
                        s.push((hi * 16 + lo) as char);
                        continue;
                    }
                    b'u' => {
                        // \uHHHH
                        self.advance();
                        let mut code: u32 = 0;
                        for _ in 0..4 {
                            code = code * 16 + self.hex_digit().unwrap_or(0) as u32;
                            self.advance();
                        }
                        if let Some(ch) = char::from_u32(code) {
                            s.push(ch);
                        }
                        continue;
                    }
                    _ => {
                        s.push(esc as char);
                    }
                }
                self.advance();
            } else {
                s.push(ch as char);
                self.advance();
            }
        }
        JsToken::StringLiteral(s) // unterminated string
    }

    fn read_number(&mut self) -> JsToken {
        let start = self.pos;
        let mut has_dot = false;

        // Check for 0x hex prefix
        if self.input[self.pos] == b'0' && self.peek_at(1) == Some(b'x') {
            self.advance(); // 0
            self.advance(); // x
            let mut val: i64 = 0;
            while self.pos < self.input.len() {
                if let Some(d) = self.hex_digit() {
                    val = val * 16 + d as i64;
                    self.advance();
                } else {
                    break;
                }
            }
            return JsToken::Number(js_int(val));
        }

        // Integer part
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.advance();
        }

        // Fractional part
        if self.pos < self.input.len() && self.input[self.pos] == b'.' {
            // Check it's not a method call (e.g., 1.toString())
            if self.peek_at(1).is_none_or(|c| c.is_ascii_digit()) {
                has_dot = true;
                self.advance(); // .
                while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                    self.advance();
                }
            }
        }

        let text = core::str::from_utf8(&self.input[start..self.pos]).unwrap_or("0");

        if has_dot {
            // Parse decimal to 32.32 fixed-point
            JsToken::Number(parse_decimal_to_fixed(text))
        } else {
            // Integer
            let val: i64 = parse_int_simple(text);
            JsToken::Number(js_int(val))
        }
    }

    fn read_identifier(&mut self) -> JsToken {
        let start = self.pos;
        while self.pos < self.input.len() && is_ident_continue(self.input[self.pos]) {
            self.advance();
        }
        let ident = core::str::from_utf8(&self.input[start..self.pos]).unwrap_or("");

        match ident {
            "let" => JsToken::Let,
            "const" => JsToken::Const,
            "var" => JsToken::Var,
            "function" => JsToken::Function,
            "return" => JsToken::Return,
            "if" => JsToken::If,
            "else" => JsToken::Else,
            "while" => JsToken::While,
            "for" => JsToken::For,
            "break" => JsToken::Break,
            "continue" => JsToken::Continue,
            "new" => JsToken::New,
            "this" => JsToken::This,
            "typeof" => JsToken::Typeof,
            "instanceof" => JsToken::Instanceof,
            "in" => JsToken::In,
            "of" => JsToken::Of,
            "delete" => JsToken::Delete,
            "void" => JsToken::Void,
            "throw" => JsToken::Throw,
            "try" => JsToken::Try,
            "catch" => JsToken::Catch,
            "finally" => JsToken::Finally,
            "switch" => JsToken::Switch,
            "case" => JsToken::Case,
            "default" => JsToken::Default,
            "class" => JsToken::Class,
            "extends" => JsToken::Extends,
            "super" => JsToken::Super,
            "import" => JsToken::Import,
            "export" => JsToken::Export,
            "true" => JsToken::True,
            "false" => JsToken::False,
            "null" => JsToken::Null,
            "undefined" => JsToken::Undefined,
            _ => JsToken::Identifier(ident.to_string()),
        }
    }

    fn hex_digit(&self) -> Option<u8> {
        let ch = *self.input.get(self.pos)?;
        match ch {
            b'0'..=b'9' => Some(ch - b'0'),
            b'a'..=b'f' => Some(ch - b'a' + 10),
            b'A'..=b'F' => Some(ch - b'A' + 10),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_ident_start(ch: u8) -> bool {
    ch.is_ascii_alphabetic() || ch == b'_' || ch == b'$'
}

fn is_ident_continue(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'$'
}

/// Parse a simple integer string (no sign prefix needed -- lexer handles
/// negation)
fn parse_int_simple(s: &str) -> i64 {
    let mut result: i64 = 0;
    for &b in s.as_bytes() {
        if b.is_ascii_digit() {
            result = result.wrapping_mul(10).wrapping_add((b - b'0') as i64);
        }
    }
    result
}

/// Parse a decimal number string to 32.32 fixed-point
fn parse_decimal_to_fixed(s: &str) -> JsNumber {
    let mut int_part: i64 = 0;
    let mut frac_part: i64 = 0;
    let mut frac_divisor: i64 = 1;
    let mut after_dot = false;

    for &b in s.as_bytes() {
        if b == b'.' {
            after_dot = true;
            continue;
        }
        if !b.is_ascii_digit() {
            continue;
        }
        if after_dot {
            frac_part = frac_part.wrapping_mul(10).wrapping_add((b - b'0') as i64);
            frac_divisor = frac_divisor.wrapping_mul(10);
        } else {
            int_part = int_part.wrapping_mul(10).wrapping_add((b - b'0') as i64);
        }
    }

    let int_fixed = int_part << JS_FRAC_BITS;
    let frac_fixed = if frac_divisor > 0 {
        (frac_part << JS_FRAC_BITS) / frac_divisor
    } else {
        0
    };
    int_fixed + frac_fixed
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    fn lex(src: &str) -> Vec<JsToken> {
        JsLexer::new(src).tokenize_all()
    }

    #[test]
    fn test_empty() {
        let tokens = lex("");
        assert_eq!(tokens, vec![JsToken::Eof]);
    }

    #[test]
    fn test_number_integer() {
        let tokens = lex("42");
        assert_eq!(tokens[0], JsToken::Number(js_int(42)));
    }

    #[test]
    fn test_number_decimal() {
        let tokens = lex("3.5");
        let n = match &tokens[0] {
            JsToken::Number(n) => *n,
            _ => panic!("expected number"),
        };
        // 3.5 in 32.32: 3 << 32 + 0.5 << 32 = 3 << 32 + 1 << 31
        assert_eq!(js_to_int(n), 3);
        assert!(!js_is_integer(n)); // has fractional part
    }

    #[test]
    fn test_number_hex() {
        let tokens = lex("0xFF");
        assert_eq!(tokens[0], JsToken::Number(js_int(255)));
    }

    #[test]
    fn test_string_double() {
        let tokens = lex("\"hello\"");
        assert_eq!(tokens[0], JsToken::StringLiteral("hello".to_string()));
    }

    #[test]
    fn test_string_single() {
        let tokens = lex("'world'");
        assert_eq!(tokens[0], JsToken::StringLiteral("world".to_string()));
    }

    #[test]
    fn test_string_escapes() {
        let tokens = lex("\"a\\nb\\tc\"");
        assert_eq!(tokens[0], JsToken::StringLiteral("a\nb\tc".to_string()));
    }

    #[test]
    fn test_string_hex_escape() {
        let tokens = lex("\"\\x41\"");
        assert_eq!(tokens[0], JsToken::StringLiteral("A".to_string()));
    }

    #[test]
    fn test_string_unicode_escape() {
        let tokens = lex("\"\\u0041\"");
        assert_eq!(tokens[0], JsToken::StringLiteral("A".to_string()));
    }

    #[test]
    fn test_keywords() {
        let tokens = lex("let const var function if else while for return");
        assert_eq!(tokens[0], JsToken::Let);
        assert_eq!(tokens[1], JsToken::Const);
        assert_eq!(tokens[2], JsToken::Var);
        assert_eq!(tokens[3], JsToken::Function);
        assert_eq!(tokens[4], JsToken::If);
        assert_eq!(tokens[5], JsToken::Else);
        assert_eq!(tokens[6], JsToken::While);
        assert_eq!(tokens[7], JsToken::For);
        assert_eq!(tokens[8], JsToken::Return);
    }

    #[test]
    fn test_identifiers() {
        let tokens = lex("foo bar_1 $var _private");
        assert_eq!(tokens[0], JsToken::Identifier("foo".to_string()));
        assert_eq!(tokens[1], JsToken::Identifier("bar_1".to_string()));
        assert_eq!(tokens[2], JsToken::Identifier("$var".to_string()));
        assert_eq!(tokens[3], JsToken::Identifier("_private".to_string()));
    }

    #[test]
    fn test_operators() {
        let tokens = lex("+ - * / % === !== <= >=");
        assert_eq!(tokens[0], JsToken::Plus);
        assert_eq!(tokens[1], JsToken::Minus);
        assert_eq!(tokens[2], JsToken::Star);
        assert_eq!(tokens[3], JsToken::Slash);
        assert_eq!(tokens[4], JsToken::Percent);
        assert_eq!(tokens[5], JsToken::EqEqEq);
        assert_eq!(tokens[6], JsToken::NotEqEq);
        assert_eq!(tokens[7], JsToken::LtEq);
        assert_eq!(tokens[8], JsToken::GtEq);
    }

    #[test]
    fn test_assignment_ops() {
        let tokens = lex("+= -= *= /=");
        assert_eq!(tokens[0], JsToken::PlusAssign);
        assert_eq!(tokens[1], JsToken::MinusAssign);
        assert_eq!(tokens[2], JsToken::StarAssign);
        assert_eq!(tokens[3], JsToken::SlashAssign);
    }

    #[test]
    fn test_delimiters() {
        let tokens = lex("(){}[]");
        assert_eq!(tokens[0], JsToken::OpenParen);
        assert_eq!(tokens[1], JsToken::CloseParen);
        assert_eq!(tokens[2], JsToken::OpenBrace);
        assert_eq!(tokens[3], JsToken::CloseBrace);
        assert_eq!(tokens[4], JsToken::OpenBracket);
        assert_eq!(tokens[5], JsToken::CloseBracket);
    }

    #[test]
    fn test_logical_ops() {
        let tokens = lex("&& || !");
        assert_eq!(tokens[0], JsToken::And);
        assert_eq!(tokens[1], JsToken::Or);
        assert_eq!(tokens[2], JsToken::Not);
    }

    #[test]
    fn test_bitwise_ops() {
        let tokens = lex("& | ^ << >>");
        assert_eq!(tokens[0], JsToken::BitAnd);
        assert_eq!(tokens[1], JsToken::BitOr);
        assert_eq!(tokens[2], JsToken::BitXor);
        assert_eq!(tokens[3], JsToken::ShiftLeft);
        assert_eq!(tokens[4], JsToken::ShiftRight);
    }

    #[test]
    fn test_arrow() {
        let tokens = lex("=>");
        assert_eq!(tokens[0], JsToken::Arrow);
    }

    #[test]
    fn test_spread() {
        let tokens = lex("...");
        assert_eq!(tokens[0], JsToken::Spread);
    }

    #[test]
    fn test_single_line_comment() {
        let tokens = lex("42 // comment\n43");
        assert_eq!(tokens[0], JsToken::Number(js_int(42)));
        // ASI may insert semicolon
        let last_num = tokens
            .iter()
            .filter(|t| matches!(t, JsToken::Number(_)))
            .count();
        assert_eq!(last_num, 2);
    }

    #[test]
    fn test_multi_line_comment() {
        let tokens = lex("1 /* block\ncomment */ 2");
        let nums: Vec<_> = tokens
            .iter()
            .filter(|t| matches!(t, JsToken::Number(_)))
            .collect();
        assert_eq!(nums.len(), 2);
    }

    #[test]
    fn test_boolean_literals() {
        let tokens = lex("true false");
        assert_eq!(tokens[0], JsToken::True);
        assert_eq!(tokens[1], JsToken::False);
    }

    #[test]
    fn test_null_undefined() {
        let tokens = lex("null undefined");
        assert_eq!(tokens[0], JsToken::Null);
        assert_eq!(tokens[1], JsToken::Undefined);
    }

    #[test]
    fn test_var_declaration() {
        let tokens = lex("let x = 10;");
        assert_eq!(tokens[0], JsToken::Let);
        assert_eq!(tokens[1], JsToken::Identifier("x".to_string()));
        assert_eq!(tokens[2], JsToken::Assign);
        assert_eq!(tokens[3], JsToken::Number(js_int(10)));
        assert_eq!(tokens[4], JsToken::Semicolon);
    }

    #[test]
    fn test_function_call() {
        let tokens = lex("foo(1, 2)");
        assert_eq!(tokens[0], JsToken::Identifier("foo".to_string()));
        assert_eq!(tokens[1], JsToken::OpenParen);
        assert_eq!(tokens[2], JsToken::Number(js_int(1)));
        assert_eq!(tokens[3], JsToken::Comma);
        assert_eq!(tokens[4], JsToken::Number(js_int(2)));
        assert_eq!(tokens[5], JsToken::CloseParen);
    }

    #[test]
    fn test_more_keywords() {
        let tokens = lex("try catch finally throw switch case default");
        assert_eq!(tokens[0], JsToken::Try);
        assert_eq!(tokens[1], JsToken::Catch);
        assert_eq!(tokens[2], JsToken::Finally);
        assert_eq!(tokens[3], JsToken::Throw);
        assert_eq!(tokens[4], JsToken::Switch);
        assert_eq!(tokens[5], JsToken::Case);
        assert_eq!(tokens[6], JsToken::Default);
    }

    #[test]
    fn test_class_keywords() {
        let tokens = lex("class extends super new");
        assert_eq!(tokens[0], JsToken::Class);
        assert_eq!(tokens[1], JsToken::Extends);
        assert_eq!(tokens[2], JsToken::Super);
        assert_eq!(tokens[3], JsToken::New);
    }

    #[test]
    fn test_typeof_instanceof() {
        let tokens = lex("typeof x instanceof Y");
        assert_eq!(tokens[0], JsToken::Typeof);
        assert_eq!(tokens[1], JsToken::Identifier("x".to_string()));
        assert_eq!(tokens[2], JsToken::Instanceof);
        assert_eq!(tokens[3], JsToken::Identifier("Y".to_string()));
    }

    #[test]
    fn test_js_int_helpers() {
        assert_eq!(js_to_int(js_int(42)), 42);
        assert!(js_is_integer(js_int(10)));
        assert!(js_is_integer(JS_ZERO));
    }

    #[test]
    fn test_token_can_end_statement() {
        assert!(JsToken::Identifier("x".to_string()).can_end_statement());
        assert!(JsToken::Number(JS_ZERO).can_end_statement());
        assert!(JsToken::CloseParen.can_end_statement());
        assert!(!JsToken::Plus.can_end_statement());
        assert!(!JsToken::OpenBrace.can_end_statement());
    }

    #[test]
    fn test_comparison_eq() {
        let tokens = lex("== ===");
        assert_eq!(tokens[0], JsToken::EqEq);
        assert_eq!(tokens[1], JsToken::EqEqEq);
    }

    #[test]
    fn test_dot_vs_spread() {
        let tokens = lex(". ...");
        assert_eq!(tokens[0], JsToken::Dot);
        assert_eq!(tokens[1], JsToken::Spread);
    }

    #[test]
    fn test_asi_basic() {
        // After "x" (an identifier that can_end_statement), a newline triggers ASI
        let tokens = lex("x\ny");
        // Should produce: Identifier("x"), Semicolon (ASI), Identifier("y"), Eof
        assert!(tokens.contains(&JsToken::Semicolon));
    }

    #[test]
    fn test_number_zero() {
        let tokens = lex("0");
        assert_eq!(tokens[0], JsToken::Number(js_int(0)));
    }

    #[test]
    fn test_line_tracking() {
        let mut lexer = JsLexer::new("a\nb\nc");
        lexer.next_token(); // a
        assert_eq!(lexer.line, 1);
        lexer.next_token(); // ASI semicolon
        lexer.next_token(); // b (after newline)
                            // line should have advanced
    }

    #[test]
    fn test_parse_decimal_to_fixed() {
        let fp = parse_decimal_to_fixed("1.5");
        assert_eq!(js_to_int(fp), 1);
        assert!(!js_is_integer(fp));
    }
}

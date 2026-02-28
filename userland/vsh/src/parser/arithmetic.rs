//! Arithmetic expression parser for `$((...))` and `let`.
//!
//! Implements a recursive descent parser for C-like integer arithmetic
//! expressions including:
//! - Integer literals (decimal, octal 0NNN, hex 0xNNN)
//! - Variable references (bare names)
//! - Unary operators: +, -, ~, !
//! - Binary operators: +, -, *, /, %, **, <<, >>
//! - Comparison: <, >, <=, >=, ==, !=
//! - Bitwise: &, ^, |
//! - Logical: &&, ||
//! - Ternary: ? :
//! - Assignment: =, +=, -=, *=, /=, %=, <<=, >>=, &=, ^=, |=
//! - Comma operator
//! - Parenthesized expressions

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use crate::error::{Result, VshError};

/// Evaluate an arithmetic expression string with the given variable context.
pub fn eval_arithmetic(expr: &str, vars: &BTreeMap<String, String>) -> Result<i64> {
    let tokens = tokenize_arith(expr)?;
    let mut parser = ArithParser::new(&tokens, vars);
    let result = parser.parse_comma()?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// Arithmetic tokens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum ArithToken {
    Number(i64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    DoubleStar, // **
    LShift,     // <<
    RShift,     // >>
    Less,
    Greater,
    LessEq,
    GreaterEq,
    EqEq,
    NotEq,
    Amp,      // &
    Caret,    // ^
    Bar,      // |
    AmpAmp,   // &&
    BarBar,   // ||
    Bang,     // !
    Tilde,    // ~
    Question, // ?
    Colon,    // :
    Eq,       // =
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    LShiftEq,
    RShiftEq,
    AmpEq,
    CaretEq,
    BarEq,
    PlusPlus,   // ++
    MinusMinus, // --
    Comma,
    LParen,
    RParen,
}

fn tokenize_arith(expr: &str) -> Result<Vec<ArithToken>> {
    let chars: Vec<char> = expr.chars().collect();
    let len = chars.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        // Skip whitespace
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Numbers
        if ch.is_ascii_digit() {
            let start = i;
            // Hex
            if ch == '0' && i + 1 < len && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
                i += 2;
                while i < len && chars[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n = i64::from_str_radix(&s[2..], 16).map_err(|_| VshError::NotANumber(s))?;
                tokens.push(ArithToken::Number(n));
            }
            // Octal (leading 0)
            else if ch == '0' && i + 1 < len && chars[i + 1].is_ascii_digit() {
                i += 1;
                while i < len && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n = i64::from_str_radix(&s[1..], 8).map_err(|_| VshError::NotANumber(s))?;
                tokens.push(ArithToken::Number(n));
            }
            // Decimal
            else {
                while i < len && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n = parse_decimal(&s)?;
                tokens.push(ArithToken::Number(n));
            }
            continue;
        }

        // Identifiers (variable names)
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            tokens.push(ArithToken::Ident(name));
            continue;
        }

        // Two-character operators (check before single-char)
        let next = if i + 1 < len {
            Some(chars[i + 1])
        } else {
            None
        };

        match (ch, next) {
            ('*', Some('*')) => {
                tokens.push(ArithToken::DoubleStar);
                i += 2;
            }
            ('+', Some('+')) => {
                tokens.push(ArithToken::PlusPlus);
                i += 2;
            }
            ('-', Some('-')) => {
                tokens.push(ArithToken::MinusMinus);
                i += 2;
            }
            ('<', Some('<')) => {
                if i + 2 < len && chars[i + 2] == '=' {
                    tokens.push(ArithToken::LShiftEq);
                    i += 3;
                } else {
                    tokens.push(ArithToken::LShift);
                    i += 2;
                }
            }
            ('>', Some('>')) => {
                if i + 2 < len && chars[i + 2] == '=' {
                    tokens.push(ArithToken::RShiftEq);
                    i += 3;
                } else {
                    tokens.push(ArithToken::RShift);
                    i += 2;
                }
            }
            ('<', Some('=')) => {
                tokens.push(ArithToken::LessEq);
                i += 2;
            }
            ('>', Some('=')) => {
                tokens.push(ArithToken::GreaterEq);
                i += 2;
            }
            ('=', Some('=')) => {
                tokens.push(ArithToken::EqEq);
                i += 2;
            }
            ('!', Some('=')) => {
                tokens.push(ArithToken::NotEq);
                i += 2;
            }
            ('&', Some('&')) => {
                tokens.push(ArithToken::AmpAmp);
                i += 2;
            }
            ('|', Some('|')) => {
                tokens.push(ArithToken::BarBar);
                i += 2;
            }
            ('+', Some('=')) => {
                tokens.push(ArithToken::PlusEq);
                i += 2;
            }
            ('-', Some('=')) => {
                tokens.push(ArithToken::MinusEq);
                i += 2;
            }
            ('*', Some('=')) => {
                tokens.push(ArithToken::StarEq);
                i += 2;
            }
            ('/', Some('=')) => {
                tokens.push(ArithToken::SlashEq);
                i += 2;
            }
            ('%', Some('=')) => {
                tokens.push(ArithToken::PercentEq);
                i += 2;
            }
            ('&', Some('=')) => {
                tokens.push(ArithToken::AmpEq);
                i += 2;
            }
            ('^', Some('=')) => {
                tokens.push(ArithToken::CaretEq);
                i += 2;
            }
            ('|', Some('=')) => {
                tokens.push(ArithToken::BarEq);
                i += 2;
            }
            _ => {
                // Single-character operators
                let tok = match ch {
                    '+' => ArithToken::Plus,
                    '-' => ArithToken::Minus,
                    '*' => ArithToken::Star,
                    '/' => ArithToken::Slash,
                    '%' => ArithToken::Percent,
                    '<' => ArithToken::Less,
                    '>' => ArithToken::Greater,
                    '&' => ArithToken::Amp,
                    '^' => ArithToken::Caret,
                    '|' => ArithToken::Bar,
                    '!' => ArithToken::Bang,
                    '~' => ArithToken::Tilde,
                    '?' => ArithToken::Question,
                    ':' => ArithToken::Colon,
                    '=' => ArithToken::Eq,
                    ',' => ArithToken::Comma,
                    '(' => ArithToken::LParen,
                    ')' => ArithToken::RParen,
                    _ => {
                        return Err(VshError::Syntax(alloc::format!(
                            "unexpected character '{}' in arithmetic",
                            ch
                        )));
                    }
                };
                tokens.push(tok);
                i += 1;
            }
        }
    }

    Ok(tokens)
}

fn parse_decimal(s: &str) -> Result<i64> {
    let mut n: i64 = 0;
    let mut neg = false;
    let bytes = s.as_bytes();
    let mut i = 0;
    if !bytes.is_empty() && bytes[0] == b'-' {
        neg = true;
        i = 1;
    }
    while i < bytes.len() {
        let d = bytes[i];
        if !d.is_ascii_digit() {
            return Err(VshError::NotANumber(String::from(s)));
        }
        n = n.wrapping_mul(10).wrapping_add((d - b'0') as i64);
        i += 1;
    }
    if neg {
        n = n.wrapping_neg();
    }
    Ok(n)
}

// ---------------------------------------------------------------------------
// Recursive descent parser
// ---------------------------------------------------------------------------

struct ArithParser<'a> {
    tokens: &'a [ArithToken],
    pos: usize,
    vars: &'a BTreeMap<String, String>,
}

impl<'a> ArithParser<'a> {
    fn new(tokens: &'a [ArithToken], vars: &'a BTreeMap<String, String>) -> Self {
        Self {
            tokens,
            pos: 0,
            vars,
        }
    }

    fn peek(&self) -> Option<&ArithToken> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&ArithToken> {
        let tok = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(tok)
    }

    fn expect(&mut self, expected: &ArithToken) -> Result<()> {
        match self.advance() {
            Some(tok) if tok == expected => Ok(()),
            _ => Err(VshError::Syntax(String::from(
                "unexpected token in arithmetic",
            ))),
        }
    }

    fn var_value(&self, name: &str) -> i64 {
        self.vars
            .get(name)
            .and_then(|v| parse_decimal(v).ok())
            .unwrap_or(0)
    }

    // Comma: expr, expr, ...
    fn parse_comma(&mut self) -> Result<i64> {
        let mut val = self.parse_assignment()?;
        while self.peek() == Some(&ArithToken::Comma) {
            self.advance();
            val = self.parse_assignment()?;
        }
        Ok(val)
    }

    // Assignment: lvalue = expr
    fn parse_assignment(&mut self) -> Result<i64> {
        self.parse_ternary()
    }

    // Ternary: cond ? then : else
    fn parse_ternary(&mut self) -> Result<i64> {
        let cond = self.parse_or()?;
        if self.peek() == Some(&ArithToken::Question) {
            self.advance();
            let then_val = self.parse_assignment()?;
            self.expect(&ArithToken::Colon)?;
            let else_val = self.parse_assignment()?;
            Ok(if cond != 0 { then_val } else { else_val })
        } else {
            Ok(cond)
        }
    }

    fn parse_or(&mut self) -> Result<i64> {
        let mut val = self.parse_and()?;
        while self.peek() == Some(&ArithToken::BarBar) {
            self.advance();
            let rhs = self.parse_and()?;
            val = if val != 0 || rhs != 0 { 1 } else { 0 };
        }
        Ok(val)
    }

    fn parse_and(&mut self) -> Result<i64> {
        let mut val = self.parse_bitor()?;
        while self.peek() == Some(&ArithToken::AmpAmp) {
            self.advance();
            let rhs = self.parse_bitor()?;
            val = if val != 0 && rhs != 0 { 1 } else { 0 };
        }
        Ok(val)
    }

    fn parse_bitor(&mut self) -> Result<i64> {
        let mut val = self.parse_bitxor()?;
        while self.peek() == Some(&ArithToken::Bar) {
            self.advance();
            let rhs = self.parse_bitxor()?;
            val |= rhs;
        }
        Ok(val)
    }

    fn parse_bitxor(&mut self) -> Result<i64> {
        let mut val = self.parse_bitand()?;
        while self.peek() == Some(&ArithToken::Caret) {
            self.advance();
            let rhs = self.parse_bitand()?;
            val ^= rhs;
        }
        Ok(val)
    }

    fn parse_bitand(&mut self) -> Result<i64> {
        let mut val = self.parse_equality()?;
        while self.peek() == Some(&ArithToken::Amp) {
            self.advance();
            let rhs = self.parse_equality()?;
            val &= rhs;
        }
        Ok(val)
    }

    fn parse_equality(&mut self) -> Result<i64> {
        let mut val = self.parse_relational()?;
        loop {
            match self.peek() {
                Some(&ArithToken::EqEq) => {
                    self.advance();
                    let rhs = self.parse_relational()?;
                    val = if val == rhs { 1 } else { 0 };
                }
                Some(&ArithToken::NotEq) => {
                    self.advance();
                    let rhs = self.parse_relational()?;
                    val = if val != rhs { 1 } else { 0 };
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_relational(&mut self) -> Result<i64> {
        let mut val = self.parse_shift()?;
        loop {
            match self.peek() {
                Some(&ArithToken::Less) => {
                    self.advance();
                    let rhs = self.parse_shift()?;
                    val = if val < rhs { 1 } else { 0 };
                }
                Some(&ArithToken::Greater) => {
                    self.advance();
                    let rhs = self.parse_shift()?;
                    val = if val > rhs { 1 } else { 0 };
                }
                Some(&ArithToken::LessEq) => {
                    self.advance();
                    let rhs = self.parse_shift()?;
                    val = if val <= rhs { 1 } else { 0 };
                }
                Some(&ArithToken::GreaterEq) => {
                    self.advance();
                    let rhs = self.parse_shift()?;
                    val = if val >= rhs { 1 } else { 0 };
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_shift(&mut self) -> Result<i64> {
        let mut val = self.parse_additive()?;
        loop {
            match self.peek() {
                Some(&ArithToken::LShift) => {
                    self.advance();
                    let rhs = self.parse_additive()?;
                    val = val.wrapping_shl(rhs as u32);
                }
                Some(&ArithToken::RShift) => {
                    self.advance();
                    let rhs = self.parse_additive()?;
                    val = val.wrapping_shr(rhs as u32);
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_additive(&mut self) -> Result<i64> {
        let mut val = self.parse_multiplicative()?;
        loop {
            match self.peek() {
                Some(&ArithToken::Plus) => {
                    self.advance();
                    let rhs = self.parse_multiplicative()?;
                    val = val.wrapping_add(rhs);
                }
                Some(&ArithToken::Minus) => {
                    self.advance();
                    let rhs = self.parse_multiplicative()?;
                    val = val.wrapping_sub(rhs);
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_multiplicative(&mut self) -> Result<i64> {
        let mut val = self.parse_exponent()?;
        loop {
            match self.peek() {
                Some(&ArithToken::Star) => {
                    self.advance();
                    let rhs = self.parse_exponent()?;
                    val = val.wrapping_mul(rhs);
                }
                Some(&ArithToken::Slash) => {
                    self.advance();
                    let rhs = self.parse_exponent()?;
                    if rhs == 0 {
                        return Err(VshError::DivisionByZero);
                    }
                    val = val.wrapping_div(rhs);
                }
                Some(&ArithToken::Percent) => {
                    self.advance();
                    let rhs = self.parse_exponent()?;
                    if rhs == 0 {
                        return Err(VshError::DivisionByZero);
                    }
                    val = val.wrapping_rem(rhs);
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_exponent(&mut self) -> Result<i64> {
        let base = self.parse_unary()?;
        if self.peek() == Some(&ArithToken::DoubleStar) {
            self.advance();
            let exp = self.parse_exponent()?; // right-associative
            Ok(int_pow(base, exp))
        } else {
            Ok(base)
        }
    }

    fn parse_unary(&mut self) -> Result<i64> {
        match self.peek() {
            Some(&ArithToken::Plus) => {
                self.advance();
                self.parse_unary()
            }
            Some(&ArithToken::Minus) => {
                self.advance();
                let val = self.parse_unary()?;
                Ok(val.wrapping_neg())
            }
            Some(&ArithToken::Bang) => {
                self.advance();
                let val = self.parse_unary()?;
                Ok(if val == 0 { 1 } else { 0 })
            }
            Some(&ArithToken::Tilde) => {
                self.advance();
                let val = self.parse_unary()?;
                Ok(!val)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<i64> {
        match self.advance() {
            Some(ArithToken::Number(n)) => Ok(*n),
            Some(ArithToken::Ident(name)) => {
                let name = name.clone();
                Ok(self.var_value(&name))
            }
            Some(ArithToken::LParen) => {
                let val = self.parse_comma()?;
                self.expect(&ArithToken::RParen)?;
                Ok(val)
            }
            _ => Err(VshError::Syntax(String::from(
                "expected number, variable, or '(' in arithmetic",
            ))),
        }
    }
}

/// Integer exponentiation.
fn int_pow(mut base: i64, mut exp: i64) -> i64 {
    if exp < 0 {
        return 0; // Integer division: base^(-n) = 0 for |base| > 1
    }
    let mut result: i64 = 1;
    while exp > 0 {
        if exp & 1 != 0 {
            result = result.wrapping_mul(base);
        }
        base = base.wrapping_mul(base);
        exp >>= 1;
    }
    result
}

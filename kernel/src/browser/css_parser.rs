//! CSS Parser
//!
//! Tokenizes and parses CSS stylesheets into structured rule sets.
//! Supports selectors (tag, class, id, descendant, child, universal,
//! compound), declarations with typed values, and specificity calculation.
//! All numeric values use 26.6 fixed-point (i32).

use alloc::{string::String, vec::Vec};

/// 26.6 fixed-point type: multiply pixel values by 64
#[allow(dead_code)]
pub type FixedPoint = i32;

/// Convert pixels to 26.6 fixed-point
#[allow(dead_code)]
pub const fn px_to_fp(px: i32) -> FixedPoint {
    px * 64
}

/// Convert 26.6 fixed-point to pixels
#[allow(dead_code)]
pub const fn fp_to_px(fp: FixedPoint) -> i32 {
    fp / 64
}

/// CSS token types
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CssToken {
    Ident(String),
    Hash(String),
    StringToken(String),
    Number(i32),
    Percentage(i32),
    Dimension(i32, String),
    Whitespace,
    Colon,
    Semicolon,
    Comma,
    OpenBrace,
    CloseBrace,
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
    Delim(char),
    AtKeyword(String),
    Function(String),
    Url(String),
    Eof,
}

/// CSS tokenizer
#[allow(dead_code)]
pub struct CssTokenizer {
    input: Vec<u8>,
    pos: usize,
}

#[allow(dead_code)]
impl CssTokenizer {
    pub fn new(input: &str) -> Self {
        Self {
            input: input.as_bytes().to_vec(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn skip_whitespace(&mut self) -> bool {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' || ch == b'\x0C' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.pos > start
    }

    fn skip_comment(&mut self) -> bool {
        if self.pos + 1 < self.input.len()
            && self.input[self.pos] == b'/'
            && self.input[self.pos + 1] == b'*'
        {
            self.pos += 2;
            while self.pos + 1 < self.input.len() {
                if self.input[self.pos] == b'*' && self.input[self.pos + 1] == b'/' {
                    self.pos += 2;
                    return true;
                }
                self.pos += 1;
            }
            self.pos = self.input.len();
            return true;
        }
        false
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_' {
                s.push(ch as char);
                self.pos += 1;
            } else {
                break;
            }
        }
        s
    }

    fn read_string(&mut self, quote: u8) -> String {
        let mut s = String::new();
        while let Some(ch) = self.advance() {
            if ch == quote {
                break;
            }
            if ch == b'\\' {
                if let Some(escaped) = self.advance() {
                    s.push(escaped as char);
                }
            } else {
                s.push(ch as char);
            }
        }
        s
    }

    fn read_number(&mut self) -> i32 {
        let mut s = String::new();
        let mut negative = false;

        if self.peek() == Some(b'-') {
            negative = true;
            self.pos += 1;
        } else if self.peek() == Some(b'+') {
            self.pos += 1;
        }

        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                s.push(ch as char);
                self.pos += 1;
            } else {
                break;
            }
        }

        // Skip decimal part (we use integer math)
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }

        let val = s.parse::<i32>().unwrap_or(0);
        if negative {
            -val
        } else {
            val
        }
    }

    pub fn next_token(&mut self) -> CssToken {
        // Skip comments
        while self.skip_comment() {}

        if self.skip_whitespace() {
            return CssToken::Whitespace;
        }

        match self.peek() {
            None => CssToken::Eof,
            Some(b'{') => {
                self.pos += 1;
                CssToken::OpenBrace
            }
            Some(b'}') => {
                self.pos += 1;
                CssToken::CloseBrace
            }
            Some(b'(') => {
                self.pos += 1;
                CssToken::OpenParen
            }
            Some(b')') => {
                self.pos += 1;
                CssToken::CloseParen
            }
            Some(b'[') => {
                self.pos += 1;
                CssToken::OpenBracket
            }
            Some(b']') => {
                self.pos += 1;
                CssToken::CloseBracket
            }
            Some(b':') => {
                self.pos += 1;
                CssToken::Colon
            }
            Some(b';') => {
                self.pos += 1;
                CssToken::Semicolon
            }
            Some(b',') => {
                self.pos += 1;
                CssToken::Comma
            }
            Some(b'#') => {
                self.pos += 1;
                let name = self.read_ident();
                CssToken::Hash(name)
            }
            Some(b'.') => {
                self.pos += 1;
                // Check if it's a number after dot
                if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    // Skip decimal digits
                    while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                        self.pos += 1;
                    }
                    CssToken::Number(0)
                } else {
                    CssToken::Delim('.')
                }
            }
            Some(b'@') => {
                self.pos += 1;
                let name = self.read_ident();
                CssToken::AtKeyword(name)
            }
            Some(b'"') => {
                self.pos += 1;
                let s = self.read_string(b'"');
                CssToken::StringToken(s)
            }
            Some(b'\'') => {
                self.pos += 1;
                let s = self.read_string(b'\'');
                CssToken::StringToken(s)
            }
            Some(ch) if ch.is_ascii_digit() || ch == b'-' || ch == b'+' => {
                // Check if it's actually a number (or negative number)
                if ch == b'-' || ch == b'+' {
                    // Peek ahead to see if it's a number
                    let next = self.input.get(self.pos + 1).copied();
                    if next.is_some_and(|c| c.is_ascii_digit()) {
                        let num = self.read_number();
                        // Check for unit or %
                        if self.peek() == Some(b'%') {
                            self.pos += 1;
                            CssToken::Percentage(num)
                        } else if self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                            let unit = self.read_ident();
                            CssToken::Dimension(num, unit)
                        } else {
                            CssToken::Number(num)
                        }
                    } else if ch == b'-' {
                        // It's an ident starting with -
                        let ident = self.read_ident();
                        if self.peek() == Some(b'(') {
                            self.pos += 1;
                            CssToken::Function(ident)
                        } else {
                            CssToken::Ident(ident)
                        }
                    } else {
                        self.pos += 1;
                        CssToken::Delim(ch as char)
                    }
                } else {
                    let num = self.read_number();
                    if self.peek() == Some(b'%') {
                        self.pos += 1;
                        CssToken::Percentage(num)
                    } else if self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                        let unit = self.read_ident();
                        CssToken::Dimension(num, unit)
                    } else {
                        CssToken::Number(num)
                    }
                }
            }
            Some(ch) if ch.is_ascii_alphabetic() || ch == b'_' => {
                let ident = self.read_ident();
                if self.peek() == Some(b'(') {
                    self.pos += 1;
                    if ident == "url" {
                        // Read URL content
                        while self.skip_whitespace() {}
                        let url = if self.peek() == Some(b'"') || self.peek() == Some(b'\'') {
                            let q = self.advance().unwrap();
                            let s = self.read_string(q);
                            while self.skip_whitespace() {}
                            if self.peek() == Some(b')') {
                                self.pos += 1;
                            }
                            s
                        } else {
                            let mut s = String::new();
                            while let Some(c) = self.peek() {
                                if c == b')' {
                                    self.pos += 1;
                                    break;
                                }
                                s.push(c as char);
                                self.pos += 1;
                            }
                            s
                        };
                        CssToken::Url(url)
                    } else {
                        CssToken::Function(ident)
                    }
                } else {
                    CssToken::Ident(ident)
                }
            }
            Some(b'*') => {
                self.pos += 1;
                CssToken::Delim('*')
            }
            Some(b'>') => {
                self.pos += 1;
                CssToken::Delim('>')
            }
            Some(b'~') => {
                self.pos += 1;
                CssToken::Delim('~')
            }
            Some(b'!') => {
                self.pos += 1;
                CssToken::Delim('!')
            }
            Some(ch) => {
                self.pos += 1;
                CssToken::Delim(ch as char)
            }
        }
    }

    pub fn tokenize_all(&mut self) -> Vec<CssToken> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token();
            if token == CssToken::Eof {
                tokens.push(CssToken::Eof);
                break;
            }
            tokens.push(token);
        }
        tokens
    }
}

/// A simple CSS selector
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimpleSelector {
    pub tag_name: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

/// Selector types
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    Simple(SimpleSelector),
    Descendant(Vec<Selector>),
    Child(Vec<Selector>),
    Class(String),
    Id(String),
    Tag(String),
    Universal,
    Compound(Vec<Selector>),
}

/// Specificity: (id_count, class_count, tag_count)
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Specificity(pub u32, pub u32, pub u32);

#[allow(dead_code)]
impl Selector {
    pub fn specificity(&self) -> Specificity {
        match self {
            Selector::Id(_) => Specificity(1, 0, 0),
            Selector::Class(_) => Specificity(0, 1, 0),
            Selector::Tag(_) => Specificity(0, 0, 1),
            Selector::Universal => Specificity(0, 0, 0),
            Selector::Simple(s) => {
                let ids = if s.id.is_some() { 1 } else { 0 };
                let classes = s.classes.len() as u32;
                let tags = if s.tag_name.is_some() { 1 } else { 0 };
                Specificity(ids, classes, tags)
            }
            Selector::Descendant(parts) | Selector::Child(parts) | Selector::Compound(parts) => {
                let mut spec = Specificity(0, 0, 0);
                for p in parts {
                    let s = p.specificity();
                    spec.0 += s.0;
                    spec.1 += s.1;
                    spec.2 += s.2;
                }
                spec
            }
        }
    }
}

/// CSS measurement unit
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Percent,
    Vh,
    Vw,
}

/// CSS property value
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CssValue {
    Keyword(String),
    Length(FixedPoint, Unit),
    Color(u32),
    Percentage(FixedPoint),
    Number(i32),
    Auto,
    #[default]
    None,
    Inherit,
    Initial,
}

/// A CSS declaration (property: value)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Declaration {
    pub property: String,
    pub value: CssValue,
    pub important: bool,
}

/// A CSS rule (selectors + declarations)
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CssRule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

/// A parsed CSS stylesheet
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub rules: Vec<CssRule>,
}

/// CSS parser
#[allow(dead_code)]
pub struct CssParser {
    tokens: Vec<CssToken>,
    pos: usize,
}

#[allow(dead_code)]
impl CssParser {
    pub fn new(css: &str) -> Self {
        let mut tokenizer = CssTokenizer::new(css);
        let tokens = tokenizer.tokenize_all();
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &CssToken {
        self.tokens.get(self.pos).unwrap_or(&CssToken::Eof)
    }

    fn advance(&mut self) -> CssToken {
        let token = self.tokens.get(self.pos).cloned().unwrap_or(CssToken::Eof);
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        token
    }

    fn skip_whitespace(&mut self) {
        while *self.peek() == CssToken::Whitespace {
            self.pos += 1;
        }
    }

    /// Parse a complete stylesheet
    pub fn parse(css: &str) -> Stylesheet {
        let mut parser = CssParser::new(css);
        parser.parse_stylesheet()
    }

    fn parse_stylesheet(&mut self) -> Stylesheet {
        let mut rules = Vec::new();
        loop {
            self.skip_whitespace();
            if *self.peek() == CssToken::Eof {
                break;
            }
            // Skip @-rules
            if let CssToken::AtKeyword(_) = self.peek() {
                self.skip_at_rule();
                continue;
            }
            if let Some(rule) = self.parse_rule() {
                rules.push(rule);
            }
        }
        Stylesheet { rules }
    }

    fn skip_at_rule(&mut self) {
        let mut depth = 0;
        loop {
            match self.advance() {
                CssToken::OpenBrace => depth += 1,
                CssToken::CloseBrace => {
                    if depth <= 1 {
                        break;
                    }
                    depth -= 1;
                }
                CssToken::Semicolon if depth == 0 => break,
                CssToken::Eof => break,
                _ => {}
            }
        }
    }

    fn parse_rule(&mut self) -> Option<CssRule> {
        let selectors = self.parse_selectors();
        if selectors.is_empty() {
            // Skip to closing brace
            let mut depth = 0;
            loop {
                match self.advance() {
                    CssToken::OpenBrace => depth += 1,
                    CssToken::CloseBrace if depth <= 1 => break,
                    CssToken::CloseBrace => depth -= 1,
                    CssToken::Eof => break,
                    _ => {}
                }
            }
            return None;
        }

        self.skip_whitespace();
        if *self.peek() == CssToken::OpenBrace {
            self.advance();
        }

        let declarations = self.parse_declarations();

        self.skip_whitespace();
        if *self.peek() == CssToken::CloseBrace {
            self.advance();
        }

        Some(CssRule {
            selectors,
            declarations,
        })
    }

    fn parse_selectors(&mut self) -> Vec<Selector> {
        let mut selectors = Vec::new();
        if let Some(sel) = self.parse_selector() {
            selectors.push(sel);
        }
        while *self.peek() == CssToken::Comma {
            self.advance();
            self.skip_whitespace();
            if let Some(sel) = self.parse_selector() {
                selectors.push(sel);
            }
        }
        selectors
    }

    fn parse_selector(&mut self) -> Option<Selector> {
        self.skip_whitespace();
        let mut parts: Vec<Selector> = Vec::new();
        let mut combinators: Vec<char> = Vec::new();

        if let Some(simple) = self.parse_simple_selector() {
            parts.push(simple);
        } else {
            return None;
        }

        loop {
            let had_ws = *self.peek() == CssToken::Whitespace;
            self.skip_whitespace();

            match self.peek() {
                CssToken::Delim('>') => {
                    self.advance();
                    self.skip_whitespace();
                    combinators.push('>');
                    if let Some(s) = self.parse_simple_selector() {
                        parts.push(s);
                    }
                }
                CssToken::OpenBrace | CssToken::Comma | CssToken::Eof => break,
                _ if had_ws => {
                    // Descendant combinator (space)
                    if let Some(s) = self.parse_simple_selector() {
                        combinators.push(' ');
                        parts.push(s);
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }

        if parts.len() == 1 {
            Some(parts.remove(0))
        } else if combinators.iter().all(|&c| c == '>') {
            Some(Selector::Child(parts))
        } else {
            Some(Selector::Descendant(parts))
        }
    }

    fn parse_simple_selector(&mut self) -> Option<Selector> {
        let mut selector = SimpleSelector::default();
        let mut matched = false;

        loop {
            match self.peek().clone() {
                CssToken::Ident(name) => {
                    let name = name.clone();
                    self.advance();
                    selector.tag_name = Some(name);
                    matched = true;
                }
                CssToken::Hash(name) => {
                    let name = name.clone();
                    self.advance();
                    selector.id = Some(name);
                    matched = true;
                }
                CssToken::Delim('.') => {
                    self.advance();
                    if let CssToken::Ident(name) = self.peek().clone() {
                        let name = name.clone();
                        self.advance();
                        selector.classes.push(name);
                        matched = true;
                    }
                }
                CssToken::Delim('*') => {
                    self.advance();
                    if !matched
                        && selector.tag_name.is_none()
                        && selector.id.is_none()
                        && selector.classes.is_empty()
                    {
                        return Some(Selector::Universal);
                    }
                    matched = true;
                }
                _ => break,
            }
        }

        if !matched {
            return None;
        }

        // Simplify: if only tag, id, or class, use specific variant
        if selector.id.is_none() && selector.classes.is_empty() {
            if let Some(ref tag) = selector.tag_name {
                return Some(Selector::Tag(tag.clone()));
            }
        }
        if selector.tag_name.is_none() && selector.classes.is_empty() {
            if let Some(ref id) = selector.id {
                return Some(Selector::Id(id.clone()));
            }
        }
        if selector.tag_name.is_none() && selector.id.is_none() && selector.classes.len() == 1 {
            return Some(Selector::Class(selector.classes[0].clone()));
        }

        Some(Selector::Simple(selector))
    }

    fn parse_declarations(&mut self) -> Vec<Declaration> {
        let mut declarations = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                CssToken::CloseBrace | CssToken::Eof => break,
                _ => {
                    if let Some(decl) = self.parse_declaration() {
                        declarations.push(decl);
                    } else {
                        // Skip to next semicolon or close brace
                        loop {
                            match self.peek() {
                                CssToken::Semicolon => {
                                    self.advance();
                                    break;
                                }
                                CssToken::CloseBrace | CssToken::Eof => break,
                                _ => {
                                    self.advance();
                                }
                            }
                        }
                    }
                }
            }
        }
        declarations
    }

    fn parse_declaration(&mut self) -> Option<Declaration> {
        self.skip_whitespace();
        let property = match self.peek().clone() {
            CssToken::Ident(name) => {
                let name = name.clone();
                self.advance();
                name
            }
            _ => return None,
        };

        self.skip_whitespace();
        if *self.peek() != CssToken::Colon {
            return None;
        }
        self.advance();
        self.skip_whitespace();

        let value = self.parse_value(&property);

        // Check for !important
        let mut important = false;
        self.skip_whitespace();
        if *self.peek() == CssToken::Delim('!') {
            self.advance();
            self.skip_whitespace();
            if let CssToken::Ident(ref s) = *self.peek() {
                if s == "important" {
                    important = true;
                    self.advance();
                }
            }
        }

        self.skip_whitespace();
        if *self.peek() == CssToken::Semicolon {
            self.advance();
        }

        Some(Declaration {
            property,
            value,
            important,
        })
    }

    fn parse_value(&mut self, _property: &str) -> CssValue {
        self.skip_whitespace();
        match self.peek().clone() {
            CssToken::Ident(name) => {
                let name = name.clone();
                self.advance();
                match name.as_str() {
                    "auto" => CssValue::Auto,
                    "none" => CssValue::None,
                    "inherit" => CssValue::Inherit,
                    "initial" => CssValue::Initial,
                    _ => CssValue::Keyword(name),
                }
            }
            CssToken::Number(n) => {
                self.advance();
                CssValue::Number(n)
            }
            CssToken::Dimension(n, ref unit) => {
                let unit = unit.clone();
                self.advance();
                let u = match unit.as_str() {
                    "px" => Unit::Px,
                    "em" => Unit::Em,
                    "rem" => Unit::Rem,
                    "vh" => Unit::Vh,
                    "vw" => Unit::Vw,
                    _ => Unit::Px,
                };
                CssValue::Length(px_to_fp(n), u)
            }
            CssToken::Percentage(n) => {
                self.advance();
                CssValue::Percentage(px_to_fp(n))
            }
            CssToken::Hash(ref color) => {
                let color = color.clone();
                self.advance();
                CssValue::Color(parse_hex_color(&color))
            }
            CssToken::Function(ref name) => {
                let name = name.clone();
                self.advance();
                if name == "rgb" || name == "rgba" {
                    self.parse_rgb_function()
                } else {
                    // Skip function content
                    let mut depth = 1;
                    while depth > 0 {
                        match self.advance() {
                            CssToken::OpenParen => depth += 1,
                            CssToken::CloseParen => depth -= 1,
                            CssToken::Eof => break,
                            _ => {}
                        }
                    }
                    CssValue::None
                }
            }
            CssToken::StringToken(s) => {
                let s = s.clone();
                self.advance();
                CssValue::Keyword(s)
            }
            _ => {
                self.advance();
                CssValue::None
            }
        }
    }

    fn parse_rgb_function(&mut self) -> CssValue {
        let mut values = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek().clone() {
                CssToken::Number(n) => {
                    self.advance();
                    values.push(n);
                }
                CssToken::Percentage(n) => {
                    self.advance();
                    values.push(fp_to_px(n) * 255 / 100);
                }
                CssToken::CloseParen => {
                    self.advance();
                    break;
                }
                CssToken::Comma | CssToken::Whitespace | CssToken::Delim('/') => {
                    self.advance();
                }
                CssToken::Eof => break,
                _ => {
                    self.advance();
                }
            }
        }

        let r = values.first().copied().unwrap_or(0).clamp(0, 255) as u32;
        let g = values.get(1).copied().unwrap_or(0).clamp(0, 255) as u32;
        let b = values.get(2).copied().unwrap_or(0).clamp(0, 255) as u32;
        let a = if values.len() >= 4 {
            values[3].clamp(0, 255) as u32
        } else {
            255
        };

        CssValue::Color((a << 24) | (r << 16) | (g << 8) | b)
    }
}

/// Parse a hex color string to ARGB u32
#[allow(dead_code)]
pub fn parse_hex_color(hex: &str) -> u32 {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        3 => {
            let r = u8_from_hex_char(hex.as_bytes()[0]);
            let g = u8_from_hex_char(hex.as_bytes()[1]);
            let b = u8_from_hex_char(hex.as_bytes()[2]);
            let r = (r << 4) | r;
            let g = (g << 4) | g;
            let b = (b << 4) | b;
            0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
        }
        6 => {
            let val = u32::from_str_radix(hex, 16).unwrap_or(0);
            0xFF000000 | val
        }
        8 => u32::from_str_radix(hex, 16).unwrap_or(0xFF000000),
        _ => 0xFF000000,
    }
}

fn u8_from_hex_char(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

/// Named CSS colors
#[allow(dead_code)]
pub fn named_color(name: &str) -> Option<u32> {
    match name {
        "black" => Some(0xFF000000),
        "white" => Some(0xFFFFFFFF),
        "red" => Some(0xFFFF0000),
        "green" => Some(0xFF008000),
        "blue" => Some(0xFF0000FF),
        "yellow" => Some(0xFFFFFF00),
        "cyan" | "aqua" => Some(0xFF00FFFF),
        "magenta" | "fuchsia" => Some(0xFFFF00FF),
        "gray" | "grey" => Some(0xFF808080),
        "silver" => Some(0xFFC0C0C0),
        "maroon" => Some(0xFF800000),
        "olive" => Some(0xFF808000),
        "lime" => Some(0xFF00FF00),
        "navy" => Some(0xFF000080),
        "purple" => Some(0xFF800080),
        "teal" => Some(0xFF008080),
        "orange" => Some(0xFFFFA500),
        "transparent" => Some(0x00000000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use alloc::vec;

    use super::*;

    #[test]
    fn test_fixed_point_conversion() {
        assert_eq!(px_to_fp(16), 1024);
        assert_eq!(fp_to_px(1024), 16);
        assert_eq!(fp_to_px(px_to_fp(42)), 42);
    }

    #[test]
    fn test_tokenize_simple() {
        let mut t = CssTokenizer::new("body { color: red; }");
        let tokens = t.tokenize_all();
        assert!(tokens.len() > 3);
    }

    #[test]
    fn test_tokenize_hash() {
        let mut t = CssTokenizer::new("#main");
        let token = t.next_token();
        assert_eq!(token, CssToken::Hash("main".into()));
    }

    #[test]
    fn test_tokenize_dimension() {
        let mut t = CssTokenizer::new("16px");
        let token = t.next_token();
        assert_eq!(token, CssToken::Dimension(16, "px".into()));
    }

    #[test]
    fn test_tokenize_percentage() {
        let mut t = CssTokenizer::new("50%");
        let token = t.next_token();
        assert_eq!(token, CssToken::Percentage(50));
    }

    #[test]
    fn test_tokenize_string() {
        let mut t = CssTokenizer::new("\"hello\"");
        let token = t.next_token();
        assert_eq!(token, CssToken::StringToken("hello".into()));
    }

    #[test]
    fn test_tokenize_comment() {
        let mut t = CssTokenizer::new("/* comment */ body");
        let token = t.next_token();
        // Should skip comment and return next token
        assert_eq!(token, CssToken::Whitespace);
    }

    #[test]
    fn test_parse_simple_rule() {
        let ss = CssParser::parse("p { color: red; }");
        assert_eq!(ss.rules.len(), 1);
        assert_eq!(ss.rules[0].selectors.len(), 1);
        assert_eq!(ss.rules[0].declarations.len(), 1);
    }

    #[test]
    fn test_parse_tag_selector() {
        let ss = CssParser::parse("div { margin: 0; }");
        assert_eq!(ss.rules[0].selectors[0], Selector::Tag("div".into()));
    }

    #[test]
    fn test_parse_class_selector() {
        let ss = CssParser::parse(".main { padding: 10px; }");
        assert_eq!(ss.rules[0].selectors[0], Selector::Class("main".into()));
    }

    #[test]
    fn test_parse_id_selector() {
        let ss = CssParser::parse("#header { height: 100px; }");
        assert_eq!(ss.rules[0].selectors[0], Selector::Id("header".into()));
    }

    #[test]
    fn test_parse_universal_selector() {
        let ss = CssParser::parse("* { margin: 0; }");
        assert_eq!(ss.rules[0].selectors[0], Selector::Universal);
    }

    #[test]
    fn test_parse_descendant_selector() {
        let ss = CssParser::parse("div p { color: blue; }");
        assert_eq!(ss.rules.len(), 1);
        if let Selector::Descendant(parts) = &ss.rules[0].selectors[0] {
            assert_eq!(parts.len(), 2);
        } else {
            panic!("Expected descendant selector");
        }
    }

    #[test]
    fn test_parse_child_selector() {
        let ss = CssParser::parse("div > p { color: red; }");
        assert_eq!(ss.rules.len(), 1);
        if let Selector::Child(parts) = &ss.rules[0].selectors[0] {
            assert_eq!(parts.len(), 2);
        } else {
            panic!("Expected child selector");
        }
    }

    #[test]
    fn test_parse_multiple_selectors() {
        let ss = CssParser::parse("h1, h2, h3 { font-weight: bold; }");
        assert_eq!(ss.rules[0].selectors.len(), 3);
    }

    #[test]
    fn test_parse_hex_color_6() {
        assert_eq!(parse_hex_color("ff0000"), 0xFFFF0000);
    }

    #[test]
    fn test_parse_hex_color_3() {
        assert_eq!(parse_hex_color("f00"), 0xFFFF0000);
    }

    #[test]
    fn test_parse_color_declaration() {
        let ss = CssParser::parse("p { color: #ff0000; }");
        let decl = &ss.rules[0].declarations[0];
        assert_eq!(decl.property, "color");
        assert_eq!(decl.value, CssValue::Color(0xFFFF0000));
    }

    #[test]
    fn test_parse_length_declaration() {
        let ss = CssParser::parse("div { width: 100px; }");
        let decl = &ss.rules[0].declarations[0];
        assert_eq!(decl.property, "width");
        assert_eq!(decl.value, CssValue::Length(px_to_fp(100), Unit::Px));
    }

    #[test]
    fn test_parse_keyword_value() {
        let ss = CssParser::parse("div { display: block; }");
        let decl = &ss.rules[0].declarations[0];
        assert_eq!(decl.value, CssValue::Keyword("block".into()));
    }

    #[test]
    fn test_parse_auto_value() {
        let ss = CssParser::parse("div { margin: auto; }");
        let decl = &ss.rules[0].declarations[0];
        assert_eq!(decl.value, CssValue::Auto);
    }

    #[test]
    fn test_parse_none_value() {
        let ss = CssParser::parse("div { display: none; }");
        let decl = &ss.rules[0].declarations[0];
        assert_eq!(decl.value, CssValue::None);
    }

    #[test]
    fn test_parse_inherit() {
        let ss = CssParser::parse("div { color: inherit; }");
        let decl = &ss.rules[0].declarations[0];
        assert_eq!(decl.value, CssValue::Inherit);
    }

    #[test]
    fn test_parse_multiple_declarations() {
        let ss = CssParser::parse("p { color: red; font-size: 16px; margin: 0; }");
        assert_eq!(ss.rules[0].declarations.len(), 3);
    }

    #[test]
    fn test_specificity_id() {
        let sel = Selector::Id("main".into());
        assert_eq!(sel.specificity(), Specificity(1, 0, 0));
    }

    #[test]
    fn test_specificity_class() {
        let sel = Selector::Class("box".into());
        assert_eq!(sel.specificity(), Specificity(0, 1, 0));
    }

    #[test]
    fn test_specificity_tag() {
        let sel = Selector::Tag("div".into());
        assert_eq!(sel.specificity(), Specificity(0, 0, 1));
    }

    #[test]
    fn test_named_colors() {
        assert_eq!(named_color("red"), Some(0xFFFF0000));
        assert_eq!(named_color("blue"), Some(0xFF0000FF));
        assert_eq!(named_color("unknown"), None);
    }

    #[test]
    fn test_parse_empty() {
        let ss = CssParser::parse("");
        assert!(ss.rules.is_empty());
    }
}

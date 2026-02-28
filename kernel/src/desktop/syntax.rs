//! Syntax Highlighting Engine
//!
//! Provides syntax highlighting for the text editor with support for
//! Rust, C, and Shell languages. Uses a character-by-character state machine
//! approach to tokenize lines into colored spans.

#![allow(dead_code)]

use alloc::{boxed::Box, vec::Vec};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A single highlighted span within a line.
#[derive(Debug, Clone)]
pub struct SyntaxToken {
    /// Byte offset of the first character (inclusive).
    pub start: usize,
    /// Byte offset one past the last character (exclusive).
    pub end: usize,
    /// Semantic category that determines the color.
    pub token_type: TokenType,
}

/// Semantic token categories used by all highlighters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Keyword,
    Type,
    StringLit,
    Comment,
    Number,
    Operator,
    Punctuation,
    Function,
    Macro,
    Attribute,
    Lifetime,
    Label,
    Normal,
}

/// Supported source languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    C,
    Cpp,
    Shell,
    Python,
    Markdown,
    Unknown,
}

/// Color theme mapping each token type to a BGRA u32 color.
///
/// Format: 0xAABBGGRR when written to the BGRA framebuffer, but the
/// existing `draw_char_into_buffer` expects a *RGB* u32 (0x00RRGGBB)
/// which it converts internally. We store colors in that same convention.
#[derive(Debug, Clone)]
pub struct SyntaxTheme {
    pub keyword_color: u32,
    pub type_color: u32,
    pub string_color: u32,
    pub comment_color: u32,
    pub number_color: u32,
    pub operator_color: u32,
    pub function_color: u32,
    pub macro_color: u32,
    pub attribute_color: u32,
    pub lifetime_color: u32,
    pub normal_color: u32,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Language-specific line tokenizer.
pub trait SyntaxHighlighter {
    /// Break a single line into a sequence of colored tokens.
    fn tokenize_line(&self, line: &str) -> Vec<SyntaxToken>;

    /// The language this highlighter handles.
    fn language(&self) -> Language;
}

// ---------------------------------------------------------------------------
// Language detection and factory
// ---------------------------------------------------------------------------

/// Detect the source language from a filename extension.
pub fn detect_language(filename: &str) -> Language {
    // Find the last '.' to extract the extension.
    let ext = match filename.rfind('.') {
        Some(pos) => &filename[pos + 1..],
        None => return Language::Unknown,
    };

    match ext {
        "rs" => Language::Rust,
        "c" | "h" => Language::C,
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" => Language::Cpp,
        "sh" | "bash" => Language::Shell,
        "py" => Language::Python,
        "md" | "markdown" => Language::Markdown,
        _ => Language::Unknown,
    }
}

/// Create the appropriate highlighter for a language (returns `None` for
/// languages without a highlighter implementation).
pub fn create_highlighter(lang: Language) -> Option<Box<dyn SyntaxHighlighter>> {
    match lang {
        Language::Rust => Some(Box::new(RustHighlighter)),
        Language::C | Language::Cpp => Some(Box::new(CHighlighter)),
        Language::Shell => Some(Box::new(ShHighlighter)),
        _ => None,
    }
}

/// Return a dark-theme color palette similar to VS Code Dark+.
///
/// Colors are in RGB format (0x00RRGGBB) to match the existing
/// `draw_char_into_buffer` convention.
pub fn default_theme() -> SyntaxTheme {
    SyntaxTheme {
        keyword_color: 0x569CD6,   // blue
        type_color: 0x4EC9B0,      // teal
        string_color: 0xCE9178,    // orange
        comment_color: 0x6A9955,   // green
        number_color: 0xB5CEA8,    // light green
        operator_color: 0xD4D4D4,  // white
        function_color: 0xDCDCAA,  // yellow
        macro_color: 0x569CD6,     // blue (same as keyword)
        attribute_color: 0x9CDCFE, // light blue
        lifetime_color: 0xD7BA7D,  // gold
        normal_color: 0xCCCCCC,    // light gray
    }
}

/// Map a `TokenType` to its display color within a theme.
pub fn get_token_color(token_type: &TokenType, theme: &SyntaxTheme) -> u32 {
    match token_type {
        TokenType::Keyword => theme.keyword_color,
        TokenType::Type => theme.type_color,
        TokenType::StringLit => theme.string_color,
        TokenType::Comment => theme.comment_color,
        TokenType::Number => theme.number_color,
        TokenType::Operator => theme.operator_color,
        TokenType::Punctuation => theme.operator_color,
        TokenType::Function => theme.function_color,
        TokenType::Macro => theme.macro_color,
        TokenType::Attribute => theme.attribute_color,
        TokenType::Lifetime => theme.lifetime_color,
        TokenType::Label => theme.lifetime_color,
        TokenType::Normal => theme.normal_color,
    }
}

// ===========================================================================
// Shared helpers
// ===========================================================================

/// Return `true` when `ch` can appear inside an identifier.
fn is_ident_char(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || ch == b'_'
}

/// Return `true` when `ch` is an operator character common across languages.
fn is_operator(ch: u8) -> bool {
    matches!(
        ch,
        b'+' | b'-' | b'*' | b'/' | b'%' | b'=' | b'!' | b'<' | b'>' | b'&' | b'|' | b'^' | b'~'
    )
}

/// Return `true` when `ch` is punctuation.
fn is_punctuation(ch: u8) -> bool {
    matches!(
        ch,
        b'(' | b')' | b'{' | b'}' | b'[' | b']' | b',' | b';' | b':' | b'.'
    )
}

/// Check whether `word` appears in `list`.
fn word_in_list(word: &[u8], list: &[&[u8]]) -> bool {
    for &entry in list {
        if word == entry {
            return true;
        }
    }
    false
}

/// Emit a token, but only if start < end.
fn push_token(tokens: &mut Vec<SyntaxToken>, start: usize, end: usize, tt: TokenType) {
    if start < end {
        tokens.push(SyntaxToken {
            start,
            end,
            token_type: tt,
        });
    }
}

// ===========================================================================
// Rust Highlighter
// ===========================================================================

/// Syntax highlighter for the Rust programming language.
pub struct RustHighlighter;

/// Rust keywords.
const RUST_KEYWORDS: &[&[u8]] = &[
    b"fn",
    b"let",
    b"mut",
    b"const",
    b"static",
    b"if",
    b"else",
    b"match",
    b"for",
    b"while",
    b"loop",
    b"return",
    b"break",
    b"continue",
    b"pub",
    b"use",
    b"mod",
    b"struct",
    b"enum",
    b"impl",
    b"trait",
    b"where",
    b"type",
    b"as",
    b"in",
    b"ref",
    b"self",
    b"super",
    b"crate",
    b"unsafe",
    b"async",
    b"await",
    b"move",
    b"dyn",
    b"extern",
    b"true",
    b"false",
];

/// Rust built-in / standard-library types.
const RUST_TYPES: &[&[u8]] = &[
    b"bool", b"u8", b"u16", b"u32", b"u64", b"u128", b"usize", b"i8", b"i16", b"i32", b"i64",
    b"i128", b"isize", b"f32", b"f64", b"char", b"str", b"String", b"Vec", b"Option", b"Result",
    b"Box", b"Rc", b"Arc", b"Self",
];

impl SyntaxHighlighter for RustHighlighter {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn tokenize_line(&self, line: &str) -> Vec<SyntaxToken> {
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut tokens: Vec<SyntaxToken> = Vec::new();
        let mut i: usize = 0;

        while i < len {
            let ch = bytes[i];

            // ----------------------------------------------------------
            // Line comments: // or /// (doc comment)
            // ----------------------------------------------------------
            if ch == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
                push_token(&mut tokens, i, len, TokenType::Comment);
                break; // rest of line is a comment
            }

            // ----------------------------------------------------------
            // Block comment (single-line portion): /* ... */
            // ----------------------------------------------------------
            if ch == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                let start = i;
                i += 2;
                while i + 1 < len {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                // If we ran off the end without closing, consume rest of line.
                if i >= len {
                    i = len;
                }
                push_token(&mut tokens, start, i, TokenType::Comment);
                continue;
            }

            // ----------------------------------------------------------
            // Attribute: #[...] or #![...]
            // ----------------------------------------------------------
            if ch == b'#' && i + 1 < len && (bytes[i + 1] == b'[' || bytes[i + 1] == b'!') {
                let start = i;
                // Consume until matching ']' or end of line.
                let mut depth: usize = 0;
                while i < len {
                    if bytes[i] == b'[' {
                        depth += 1;
                    } else if bytes[i] == b']' {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::Attribute);
                continue;
            }

            // ----------------------------------------------------------
            // Raw string: r"..." or r#"..."#
            // ----------------------------------------------------------
            if ch == b'r' && i + 1 < len && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
                // Count leading '#' symbols.
                let start = i;
                i += 1; // skip 'r'
                let mut hashes: usize = 0;
                while i < len && bytes[i] == b'#' {
                    hashes += 1;
                    i += 1;
                }
                if i < len && bytes[i] == b'"' {
                    i += 1; // skip opening '"'
                            // Scan for closing '"' followed by the same number of '#'.
                    'raw_scan: while i < len {
                        if bytes[i] == b'"' {
                            let mut matched: usize = 0;
                            let after_quote = i + 1;
                            while matched < hashes
                                && after_quote + matched < len
                                && bytes[after_quote + matched] == b'#'
                            {
                                matched += 1;
                            }
                            if matched == hashes {
                                i = after_quote + matched;
                                break 'raw_scan;
                            }
                        }
                        i += 1;
                    }
                    if i > len {
                        i = len;
                    }
                    push_token(&mut tokens, start, i, TokenType::StringLit);
                    continue;
                }
                // Not actually a raw string -- fall through and let the identifier
                // path handle the 'r'.
                i = start;
            }

            // ----------------------------------------------------------
            // String literal: "..."
            // ----------------------------------------------------------
            if ch == b'"' {
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2; // skip escaped character
                        continue;
                    }
                    if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::StringLit);
                continue;
            }

            // ----------------------------------------------------------
            // Character literal: '.'  (but NOT lifetime 'a)
            // ----------------------------------------------------------
            if ch == b'\'' && i + 2 < len && bytes[i + 2] == b'\'' && bytes[i + 1] != b'\\' {
                push_token(&mut tokens, i, i + 3, TokenType::StringLit);
                i += 3;
                continue;
            }
            // Escaped char literal: '\n' etc.
            if ch == b'\'' && i + 3 < len && bytes[i + 1] == b'\\' && bytes[i + 3] == b'\'' {
                push_token(&mut tokens, i, i + 4, TokenType::StringLit);
                i += 4;
                continue;
            }

            // ----------------------------------------------------------
            // Lifetime: 'a, 'static, '_ etc.
            // ----------------------------------------------------------
            if ch == b'\''
                && i + 1 < len
                && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
            {
                let start = i;
                i += 1; // skip the tick
                while i < len && is_ident_char(bytes[i]) {
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::Lifetime);
                continue;
            }

            // ----------------------------------------------------------
            // Number literals: 0x.., 0b.., 0o.., decimal, with _ separators
            // ----------------------------------------------------------
            if ch.is_ascii_digit() {
                let start = i;
                if ch == b'0' && i + 1 < len {
                    match bytes[i + 1] {
                        b'x' | b'X' => {
                            i += 2;
                            while i < len && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                                i += 1;
                            }
                            push_token(&mut tokens, start, i, TokenType::Number);
                            continue;
                        }
                        b'b' | b'B' => {
                            i += 2;
                            while i < len
                                && (bytes[i] == b'0' || bytes[i] == b'1' || bytes[i] == b'_')
                            {
                                i += 1;
                            }
                            push_token(&mut tokens, start, i, TokenType::Number);
                            continue;
                        }
                        b'o' | b'O' => {
                            i += 2;
                            while i < len
                                && ((bytes[i] >= b'0' && bytes[i] <= b'7') || bytes[i] == b'_')
                            {
                                i += 1;
                            }
                            push_token(&mut tokens, start, i, TokenType::Number);
                            continue;
                        }
                        _ => {}
                    }
                }
                // Decimal (possibly with '.' for float, but we keep it simple
                // and just consume digits + underscores).
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    // Avoid consuming '..' (range operator) as part of a number.
                    if bytes[i] == b'.' && i + 1 < len && bytes[i + 1] == b'.' {
                        break;
                    }
                    i += 1;
                }
                // Optional type suffix like u32, i64, usize, etc.
                if i < len && (bytes[i] == b'u' || bytes[i] == b'i' || bytes[i] == b'f') {
                    while i < len && is_ident_char(bytes[i]) {
                        i += 1;
                    }
                }
                push_token(&mut tokens, start, i, TokenType::Number);
                continue;
            }

            // ----------------------------------------------------------
            // Identifiers, keywords, types, macros, functions
            // ----------------------------------------------------------
            if ch.is_ascii_alphabetic() || ch == b'_' {
                let start = i;
                while i < len && is_ident_char(bytes[i]) {
                    i += 1;
                }
                let word = &bytes[start..i];

                // Macro invocation: name followed by '!'
                if i < len && bytes[i] == b'!' {
                    push_token(&mut tokens, start, i + 1, TokenType::Macro);
                    i += 1;
                    continue;
                }

                // Function call: name followed by '('
                if i < len && bytes[i] == b'(' {
                    // But only if it is not a keyword (e.g. `if (...)` in C-style).
                    if !word_in_list(word, RUST_KEYWORDS) {
                        push_token(&mut tokens, start, i, TokenType::Function);
                        continue;
                    }
                }

                if word_in_list(word, RUST_KEYWORDS) {
                    push_token(&mut tokens, start, i, TokenType::Keyword);
                } else if word_in_list(word, RUST_TYPES) {
                    push_token(&mut tokens, start, i, TokenType::Type);
                } else {
                    push_token(&mut tokens, start, i, TokenType::Normal);
                }
                continue;
            }

            // ----------------------------------------------------------
            // Operators
            // ----------------------------------------------------------
            if is_operator(ch) {
                let start = i;
                // Consume consecutive operator characters so that `==` etc.
                // are a single token.
                while i < len && is_operator(bytes[i]) {
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::Operator);
                continue;
            }

            // ----------------------------------------------------------
            // Punctuation
            // ----------------------------------------------------------
            if is_punctuation(ch) {
                push_token(&mut tokens, i, i + 1, TokenType::Punctuation);
                i += 1;
                continue;
            }

            // ----------------------------------------------------------
            // Whitespace and anything else -- skip without emitting
            // ----------------------------------------------------------
            i += 1;
        }

        tokens
    }
}

// ===========================================================================
// C / C++ Highlighter
// ===========================================================================

/// Syntax highlighter for C and C++ source code.
pub struct CHighlighter;

const C_KEYWORDS: &[&[u8]] = &[
    b"if",
    b"else",
    b"for",
    b"while",
    b"do",
    b"switch",
    b"case",
    b"break",
    b"continue",
    b"return",
    b"goto",
    b"typedef",
    b"struct",
    b"union",
    b"enum",
    b"sizeof",
    b"void",
    b"static",
    b"extern",
    b"const",
    b"volatile",
    b"register",
    b"inline",
    b"restrict",
    b"default",
    b"true",
    b"false",
    b"NULL",
];

const C_TYPES: &[&[u8]] = &[
    b"int",
    b"char",
    b"float",
    b"double",
    b"long",
    b"short",
    b"unsigned",
    b"signed",
    b"size_t",
    b"uint8_t",
    b"uint16_t",
    b"uint32_t",
    b"uint64_t",
    b"int8_t",
    b"int16_t",
    b"int32_t",
    b"int64_t",
    b"bool",
    b"FILE",
    b"ssize_t",
    b"ptrdiff_t",
];

impl SyntaxHighlighter for CHighlighter {
    fn language(&self) -> Language {
        Language::C
    }

    fn tokenize_line(&self, line: &str) -> Vec<SyntaxToken> {
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut tokens: Vec<SyntaxToken> = Vec::new();
        let mut i: usize = 0;

        // Skip leading whitespace to detect preprocessor directives.
        let mut ws = 0;
        while ws < len && (bytes[ws] == b' ' || bytes[ws] == b'\t') {
            ws += 1;
        }

        // Preprocessor directive: line begins with optional whitespace then '#'
        if ws < len && bytes[ws] == b'#' {
            push_token(&mut tokens, ws, len, TokenType::Attribute);
            return tokens;
        }

        while i < len {
            let ch = bytes[i];

            // ----------------------------------------------------------
            // Line comment: //
            // ----------------------------------------------------------
            if ch == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
                push_token(&mut tokens, i, len, TokenType::Comment);
                break;
            }

            // ----------------------------------------------------------
            // Block comment: /* ... */
            // ----------------------------------------------------------
            if ch == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                let start = i;
                i += 2;
                while i + 1 < len {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                if i >= len {
                    i = len;
                }
                push_token(&mut tokens, start, i, TokenType::Comment);
                continue;
            }

            // ----------------------------------------------------------
            // String literal
            // ----------------------------------------------------------
            if ch == b'"' {
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::StringLit);
                continue;
            }

            // ----------------------------------------------------------
            // Character literal
            // ----------------------------------------------------------
            if ch == b'\'' {
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'\'' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::StringLit);
                continue;
            }

            // ----------------------------------------------------------
            // Number literals
            // ----------------------------------------------------------
            if ch.is_ascii_digit() {
                let start = i;
                if ch == b'0' && i + 1 < len {
                    match bytes[i + 1] {
                        b'x' | b'X' => {
                            i += 2;
                            while i < len && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                                i += 1;
                            }
                            // Consume optional suffix (u, ul, ull, l, ll, etc.)
                            while i < len && bytes[i].is_ascii_alphabetic() {
                                i += 1;
                            }
                            push_token(&mut tokens, start, i, TokenType::Number);
                            continue;
                        }
                        b'b' | b'B' => {
                            i += 2;
                            while i < len
                                && (bytes[i] == b'0' || bytes[i] == b'1' || bytes[i] == b'_')
                            {
                                i += 1;
                            }
                            push_token(&mut tokens, start, i, TokenType::Number);
                            continue;
                        }
                        _ => {}
                    }
                }
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'_')
                {
                    i += 1;
                }
                // Optional suffix: f, l, u, ul, ull, etc.
                while i < len && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::Number);
                continue;
            }

            // ----------------------------------------------------------
            // Identifiers, keywords, types, functions
            // ----------------------------------------------------------
            if ch.is_ascii_alphabetic() || ch == b'_' {
                let start = i;
                while i < len && is_ident_char(bytes[i]) {
                    i += 1;
                }
                let word = &bytes[start..i];

                // Function call heuristic: identifier followed by '('
                if i < len && bytes[i] == b'(' && !word_in_list(word, C_KEYWORDS) {
                    push_token(&mut tokens, start, i, TokenType::Function);
                    continue;
                }

                if word_in_list(word, C_KEYWORDS) {
                    push_token(&mut tokens, start, i, TokenType::Keyword);
                } else if word_in_list(word, C_TYPES) {
                    push_token(&mut tokens, start, i, TokenType::Type);
                } else {
                    push_token(&mut tokens, start, i, TokenType::Normal);
                }
                continue;
            }

            // ----------------------------------------------------------
            // Operators
            // ----------------------------------------------------------
            if is_operator(ch) {
                let start = i;
                while i < len && is_operator(bytes[i]) {
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::Operator);
                continue;
            }

            // ----------------------------------------------------------
            // Punctuation
            // ----------------------------------------------------------
            if is_punctuation(ch) {
                push_token(&mut tokens, i, i + 1, TokenType::Punctuation);
                i += 1;
                continue;
            }

            // Whitespace / other
            i += 1;
        }

        tokens
    }
}

// ===========================================================================
// Shell / Bash Highlighter
// ===========================================================================

/// Syntax highlighter for shell (Bash) scripts.
pub struct ShHighlighter;

const SH_KEYWORDS: &[&[u8]] = &[
    b"if",
    b"then",
    b"elif",
    b"else",
    b"fi",
    b"for",
    b"in",
    b"do",
    b"done",
    b"while",
    b"until",
    b"case",
    b"esac",
    b"function",
    b"return",
    b"local",
    b"export",
    b"source",
    b"eval",
    b"exec",
    b"exit",
    b"break",
    b"continue",
    b"select",
];

const SH_BUILTINS: &[&[u8]] = &[
    b"echo", b"cd", b"pwd", b"ls", b"cat", b"grep", b"sed", b"awk", b"find", b"test", b"read",
    b"set", b"unset", b"shift", b"trap", b"printf", b"declare", b"typeset", b"let",
];

impl SyntaxHighlighter for ShHighlighter {
    fn language(&self) -> Language {
        Language::Shell
    }

    fn tokenize_line(&self, line: &str) -> Vec<SyntaxToken> {
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut tokens: Vec<SyntaxToken> = Vec::new();
        let mut i: usize = 0;

        // Skip leading whitespace to detect comment lines.
        let mut ws = 0;
        while ws < len && (bytes[ws] == b' ' || bytes[ws] == b'\t') {
            ws += 1;
        }

        while i < len {
            let ch = bytes[i];

            // ----------------------------------------------------------
            // Comment: '#' (but not inside a string, and not #! shebang
            // which we still highlight as a comment)
            // ----------------------------------------------------------
            if ch == b'#' {
                push_token(&mut tokens, i, len, TokenType::Comment);
                break;
            }

            // ----------------------------------------------------------
            // Double-quoted string: "..." (allows $var expansion inside,
            // but we just color the whole thing as a string for simplicity)
            // ----------------------------------------------------------
            if ch == b'"' {
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::StringLit);
                continue;
            }

            // ----------------------------------------------------------
            // Single-quoted string: '...' (no escapes except '')
            // ----------------------------------------------------------
            if ch == b'\'' {
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\'' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::StringLit);
                continue;
            }

            // ----------------------------------------------------------
            // Backtick command substitution: `...`
            // ----------------------------------------------------------
            if ch == b'`' {
                let start = i;
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'`' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::StringLit);
                continue;
            }

            // ----------------------------------------------------------
            // Variable / expansion: $var, ${var}, $(...), $((..))
            // ----------------------------------------------------------
            if ch == b'$' {
                let start = i;
                i += 1;
                if i < len {
                    match bytes[i] {
                        b'{' => {
                            // ${...}
                            i += 1;
                            while i < len && bytes[i] != b'}' {
                                i += 1;
                            }
                            if i < len {
                                i += 1;
                            }
                        }
                        b'(' => {
                            // $(...) or $((...))
                            let mut depth: usize = 1;
                            i += 1;
                            while i < len && depth > 0 {
                                if bytes[i] == b'(' {
                                    depth += 1;
                                } else if bytes[i] == b')' {
                                    depth -= 1;
                                }
                                if depth > 0 {
                                    i += 1;
                                }
                            }
                            if i < len {
                                i += 1; // skip closing ')'
                            }
                        }
                        b'?' | b'!' | b'$' | b'#' | b'@' | b'*' | b'-' | b'0'..=b'9' => {
                            // Special variable: $?, $$, $!, $#, $@, $*, $-, $0..$9
                            i += 1;
                        }
                        _ => {
                            // $VARIABLE_NAME
                            while i < len && is_ident_char(bytes[i]) {
                                i += 1;
                            }
                        }
                    }
                }
                push_token(&mut tokens, start, i, TokenType::Macro);
                continue;
            }

            // ----------------------------------------------------------
            // Number literals
            // ----------------------------------------------------------
            if ch.is_ascii_digit() {
                let start = i;
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::Number);
                continue;
            }

            // ----------------------------------------------------------
            // Identifiers, keywords, builtins
            // ----------------------------------------------------------
            if ch.is_ascii_alphabetic() || ch == b'_' {
                let start = i;
                while i < len && (is_ident_char(bytes[i]) || bytes[i] == b'-') {
                    i += 1;
                }
                let word = &bytes[start..i];

                if word_in_list(word, SH_KEYWORDS) {
                    push_token(&mut tokens, start, i, TokenType::Keyword);
                } else if word_in_list(word, SH_BUILTINS) {
                    push_token(&mut tokens, start, i, TokenType::Function);
                } else {
                    push_token(&mut tokens, start, i, TokenType::Normal);
                }
                continue;
            }

            // ----------------------------------------------------------
            // Operators (shell-specific: |, ||, &&, ;, ;;, &, >, >>, <, etc.)
            // ----------------------------------------------------------
            if is_operator(ch) || ch == b'@' {
                let start = i;
                while i < len && is_operator(bytes[i]) {
                    i += 1;
                }
                push_token(&mut tokens, start, i, TokenType::Operator);
                continue;
            }

            // ----------------------------------------------------------
            // Punctuation
            // ----------------------------------------------------------
            if is_punctuation(ch) {
                push_token(&mut tokens, i, i + 1, TokenType::Punctuation);
                i += 1;
                continue;
            }

            // Whitespace / other
            i += 1;
        }

        tokens
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Language detection --------------------------------------------------

    #[test]
    fn test_detect_rust() {
        assert_eq!(detect_language("main.rs"), Language::Rust);
    }

    #[test]
    fn test_detect_c() {
        assert_eq!(detect_language("foo.c"), Language::C);
        assert_eq!(detect_language("bar.h"), Language::C);
    }

    #[test]
    fn test_detect_shell() {
        assert_eq!(detect_language("run.sh"), Language::Shell);
        assert_eq!(detect_language("setup.bash"), Language::Shell);
    }

    #[test]
    fn test_detect_unknown() {
        assert_eq!(detect_language("notes.txt"), Language::Unknown);
        assert_eq!(detect_language("Makefile"), Language::Unknown);
    }

    // -- Theme ---------------------------------------------------------------

    #[test]
    fn test_default_theme_nonzero() {
        let theme = default_theme();
        assert_ne!(theme.keyword_color, 0);
        assert_ne!(theme.comment_color, 0);
        assert_ne!(theme.string_color, 0);
    }

    #[test]
    fn test_get_token_color() {
        let theme = default_theme();
        assert_eq!(
            get_token_color(&TokenType::Keyword, &theme),
            theme.keyword_color
        );
        assert_eq!(
            get_token_color(&TokenType::Comment, &theme),
            theme.comment_color
        );
    }

    // -- Factory -------------------------------------------------------------

    #[test]
    fn test_create_highlighter_rust() {
        let hl = create_highlighter(Language::Rust);
        assert!(hl.is_some());
        assert_eq!(hl.unwrap().language(), Language::Rust);
    }

    #[test]
    fn test_create_highlighter_unknown() {
        assert!(create_highlighter(Language::Unknown).is_none());
    }

    // -- Rust tokenizer ------------------------------------------------------

    #[test]
    fn test_rust_keyword() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("fn main() {");
        assert!(tokens.len() >= 2);
        assert_eq!(tokens[0].token_type, TokenType::Keyword); // fn
        assert_eq!(tokens[1].token_type, TokenType::Function); // main
    }

    #[test]
    fn test_rust_comment() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("// this is a comment");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].token_type, TokenType::Comment);
        assert_eq!(tokens[0].start, 0);
    }

    #[test]
    fn test_rust_string() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("let s = \"hello\";");
        // Find the string token
        let string_tok = tokens.iter().find(|t| t.token_type == TokenType::StringLit);
        assert!(string_tok.is_some());
    }

    #[test]
    fn test_rust_number_hex() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("let x = 0xFF;");
        let num_tok = tokens.iter().find(|t| t.token_type == TokenType::Number);
        assert!(num_tok.is_some());
    }

    #[test]
    fn test_rust_lifetime() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("fn foo<'a>(x: &'a str)");
        let lt_tok = tokens.iter().find(|t| t.token_type == TokenType::Lifetime);
        assert!(lt_tok.is_some());
    }

    #[test]
    fn test_rust_macro() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("println!(\"hi\");");
        assert_eq!(tokens[0].token_type, TokenType::Macro);
    }

    #[test]
    fn test_rust_attribute() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("#[derive(Debug)]");
        assert_eq!(tokens[0].token_type, TokenType::Attribute);
    }

    #[test]
    fn test_rust_type() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("let v: Vec<u32> = Vec::new();");
        let type_tok = tokens.iter().find(|t| t.token_type == TokenType::Type);
        assert!(type_tok.is_some());
    }

    // -- C tokenizer ---------------------------------------------------------

    #[test]
    fn test_c_preprocessor() {
        let hl = CHighlighter;
        let tokens = hl.tokenize_line("#include <stdio.h>");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].token_type, TokenType::Attribute);
    }

    #[test]
    fn test_c_keyword() {
        let hl = CHighlighter;
        let tokens = hl.tokenize_line("if (x > 0) return 1;");
        assert_eq!(tokens[0].token_type, TokenType::Keyword); // if
    }

    #[test]
    fn test_c_string() {
        let hl = CHighlighter;
        let tokens = hl.tokenize_line("char *s = \"hello\";");
        let string_tok = tokens.iter().find(|t| t.token_type == TokenType::StringLit);
        assert!(string_tok.is_some());
    }

    #[test]
    fn test_c_function() {
        let hl = CHighlighter;
        let tokens = hl.tokenize_line("printf(\"hi\");");
        assert_eq!(tokens[0].token_type, TokenType::Function);
    }

    // -- Shell tokenizer -----------------------------------------------------

    #[test]
    fn test_sh_comment() {
        let hl = ShHighlighter;
        let tokens = hl.tokenize_line("# this is a comment");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].token_type, TokenType::Comment);
    }

    #[test]
    fn test_sh_keyword() {
        let hl = ShHighlighter;
        let tokens = hl.tokenize_line("if [ -f file ]; then");
        assert_eq!(tokens[0].token_type, TokenType::Keyword); // if
    }

    #[test]
    fn test_sh_variable() {
        let hl = ShHighlighter;
        let tokens = hl.tokenize_line("echo $HOME");
        let var_tok = tokens.iter().find(|t| t.token_type == TokenType::Macro);
        assert!(var_tok.is_some());
    }

    #[test]
    fn test_sh_double_quoted() {
        let hl = ShHighlighter;
        let tokens = hl.tokenize_line("echo \"hello world\"");
        let str_tok = tokens.iter().find(|t| t.token_type == TokenType::StringLit);
        assert!(str_tok.is_some());
    }

    #[test]
    fn test_sh_single_quoted() {
        let hl = ShHighlighter;
        let tokens = hl.tokenize_line("echo 'hello world'");
        let str_tok = tokens.iter().find(|t| t.token_type == TokenType::StringLit);
        assert!(str_tok.is_some());
    }

    #[test]
    fn test_sh_builtin() {
        let hl = ShHighlighter;
        let tokens = hl.tokenize_line("echo hello");
        assert_eq!(tokens[0].token_type, TokenType::Function); // echo is a builtin
    }

    #[test]
    fn test_sh_expansion() {
        let hl = ShHighlighter;
        let tokens = hl.tokenize_line("echo ${PATH}");
        let var_tok = tokens.iter().find(|t| t.token_type == TokenType::Macro);
        assert!(var_tok.is_some());
    }

    // -- Edge cases ----------------------------------------------------------

    #[test]
    fn test_empty_line() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_whitespace_only() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("    ");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_rust_escaped_string() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line(r#"let s = "he\"llo";"#);
        let str_tok = tokens.iter().find(|t| t.token_type == TokenType::StringLit);
        assert!(str_tok.is_some());
    }

    #[test]
    fn test_rust_block_comment() {
        let hl = RustHighlighter;
        let tokens = hl.tokenize_line("let x = /* comment */ 5;");
        let comment_tok = tokens.iter().find(|t| t.token_type == TokenType::Comment);
        assert!(comment_tok.is_some());
        let num_tok = tokens.iter().find(|t| t.token_type == TokenType::Number);
        assert!(num_tok.is_some());
    }

    #[test]
    fn test_token_spans_cover_text() {
        let hl = RustHighlighter;
        let line = "fn foo()";
        let tokens = hl.tokenize_line(line);
        // Verify that token spans reference valid byte offsets.
        for tok in &tokens {
            assert!(tok.start < line.len());
            assert!(tok.end <= line.len());
            assert!(tok.start < tok.end);
        }
    }
}

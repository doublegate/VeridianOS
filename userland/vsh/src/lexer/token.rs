//! Token types for the vsh lexer.
//!
//! Defines the complete set of tokens produced by lexing a bash command line.

use alloc::string::String;

/// Source position for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

/// A single token produced by the lexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Returns true if this token is a newline or EOF.
    #[allow(dead_code)] // Public API for parser
    pub fn is_terminator(&self) -> bool {
        matches!(self.kind, TokenKind::Newline | TokenKind::Eof)
    }

    /// Returns true if this is a word token.
    #[allow(dead_code)] // Public API for parser
    pub fn is_word(&self) -> bool {
        matches!(self.kind, TokenKind::Word(_))
    }

    /// Extract the word value if this is a Word token.
    #[allow(dead_code)] // Public API for parser
    pub fn word_value(&self) -> Option<&str> {
        match &self.kind {
            TokenKind::Word(w) => Some(w.as_str()),
            _ => None,
        }
    }
}

/// The kind of a token.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // All variants are part of the token API
pub enum TokenKind {
    // --- Atoms ---
    /// A word (command name, argument, filename, etc.). May contain
    /// quotes, escapes, and expansion markers that are resolved later.
    Word(String),

    /// An assignment word of the form `NAME=VALUE`.
    Assignment(String, String),

    /// A numeric file descriptor prefix for redirection (e.g. `2` in `2>`).
    IoNumber(i32),

    // --- Operators ---
    /// `|`
    Pipe,
    /// `|&` (pipe stdout and stderr)
    PipeAmpersand,
    /// `||`
    Or,
    /// `&&`
    And,
    /// `;`
    Semi,
    /// `;;` (case delimiter)
    DoubleSemi,
    /// `;&` (case fall-through)
    SemiAnd,
    /// `;;&` (case test-next)
    SemiSemiAnd,
    /// `&`
    Ampersand,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
    RBrace,

    // --- Redirections ---
    /// `<`
    Less,
    /// `>`
    Greater,
    /// `>>`
    DGreater,
    /// `<<` (here-document)
    DLess,
    /// `<<-` (here-document with tab stripping)
    DLessDash,
    /// `<<<` (here-string)
    TLess,
    /// `<>`
    LessGreater,
    /// `>&`
    GreaterAnd,
    /// `<&`
    LessAnd,
    /// `>|` (clobber)
    Clobber,
    /// `&>` (redirect stdout+stderr)
    AndGreater,
    /// `&>>` (append stdout+stderr)
    AndDGreater,

    // --- Process substitution markers ---
    /// `<(` (process substitution input)
    ProcSubIn,
    /// `>(` (process substitution output)
    ProcSubOut,

    // --- Reserved words ---
    /// `if`
    If,
    /// `then`
    Then,
    /// `else`
    Else,
    /// `elif`
    Elif,
    /// `fi`
    Fi,
    /// `while`
    While,
    /// `until`
    Until,
    /// `for`
    For,
    /// `do`
    Do,
    /// `done`
    Done,
    /// `case`
    Case,
    /// `esac`
    Esac,
    /// `in`
    In,
    /// `select`
    Select,
    /// `function`
    Function,
    /// `coproc`
    Coproc,
    /// `time`
    Time,
    /// `!` (pipeline negation)
    Bang,
    /// `[[`
    DLBracket,
    /// `]]`
    DRBracket,

    // --- Special ---
    /// Newline (command separator)
    Newline,
    /// End of input
    Eof,

    /// A here-document body. Produced after the corresponding `<<DELIM`
    /// operator is found and the body is collected.
    HereDocBody(String),
}

impl TokenKind {
    /// Attempt to classify a word as a reserved word. Returns the reserved
    /// word token kind if it matches, otherwise returns None.
    pub fn reserved_word(word: &str) -> Option<TokenKind> {
        match word {
            "if" => Some(TokenKind::If),
            "then" => Some(TokenKind::Then),
            "else" => Some(TokenKind::Else),
            "elif" => Some(TokenKind::Elif),
            "fi" => Some(TokenKind::Fi),
            "while" => Some(TokenKind::While),
            "until" => Some(TokenKind::Until),
            "for" => Some(TokenKind::For),
            "do" => Some(TokenKind::Do),
            "done" => Some(TokenKind::Done),
            "case" => Some(TokenKind::Case),
            "esac" => Some(TokenKind::Esac),
            "in" => Some(TokenKind::In),
            "select" => Some(TokenKind::Select),
            "function" => Some(TokenKind::Function),
            "coproc" => Some(TokenKind::Coproc),
            "time" => Some(TokenKind::Time),
            "!" => Some(TokenKind::Bang),
            "[[" => Some(TokenKind::DLBracket),
            "]]" => Some(TokenKind::DRBracket),
            _ => None,
        }
    }
}

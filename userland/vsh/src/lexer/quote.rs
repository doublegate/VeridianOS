//! Quote state machine for the lexer.
//!
//! Tracks nesting of single quotes, double quotes, command substitution
//! `$(...)`, backtick substitution `` `...` ``, arithmetic `$((...))`,
//! and parameter expansion `${...}`.

use alloc::vec::Vec;

/// The current quoting context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteContext {
    /// Not inside any quotes.
    None,
    /// Inside single quotes: everything is literal.
    Single,
    /// Inside double quotes: variable/command expansion still active.
    Double,
    /// Inside `$(...)` command substitution.
    CommandSub,
    /// Inside `` `...` `` backtick command substitution.
    Backtick,
    /// Inside `$((...))` arithmetic expansion.
    ArithmeticSub,
    /// Inside `${...}` parameter expansion.
    ParamExpansion,
}

/// Quote state tracker that supports nested contexts.
#[derive(Debug, Clone)]
pub struct QuoteState {
    /// Stack of nesting contexts. The last element is the current context.
    stack: Vec<QuoteContext>,
    /// Paren depth inside command substitution (for matching nested parens).
    paren_depth: u32,
    /// Brace depth inside parameter expansion.
    brace_depth: u32,
}

impl QuoteState {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            paren_depth: 0,
            brace_depth: 0,
        }
    }

    /// Current quoting context.
    pub fn current(&self) -> QuoteContext {
        self.stack.last().copied().unwrap_or(QuoteContext::None)
    }

    /// Whether we are inside any quoting context.
    pub fn is_quoted(&self) -> bool {
        !self.stack.is_empty()
    }

    /// Whether the current context allows variable expansion.
    #[allow(dead_code)] // Public API for expansion engine
    pub fn allows_expansion(&self) -> bool {
        !matches!(self.current(), QuoteContext::Single)
    }

    /// Whether the current character should be treated as a metacharacter
    /// (i.e., we are not inside quotes that suppress metachar meaning).
    #[allow(dead_code)] // Public API for expansion engine
    pub fn is_unquoted(&self) -> bool {
        matches!(
            self.current(),
            QuoteContext::None | QuoteContext::CommandSub | QuoteContext::ArithmeticSub
        )
    }

    /// Push a new quoting context.
    pub fn push(&mut self, ctx: QuoteContext) {
        self.stack.push(ctx);
    }

    /// Pop the current quoting context.
    pub fn pop(&mut self) -> Option<QuoteContext> {
        self.stack.pop()
    }

    /// Process a character and update state. Returns `true` if the character
    /// was consumed as a quote metacharacter (should not be added to the
    /// current token).
    ///
    /// `peek` is the next character (if any), used for multi-character
    /// sequences like `$(`, `$((`, `${`.
    pub fn process(&mut self, ch: char, peek: Option<char>, peek2: Option<char>) -> QuoteAction {
        match self.current() {
            QuoteContext::None | QuoteContext::CommandSub | QuoteContext::ArithmeticSub => {
                self.process_unquoted(ch, peek, peek2)
            }
            QuoteContext::Single => {
                if ch == '\'' {
                    self.pop();
                    QuoteAction::Consumed
                } else {
                    QuoteAction::Literal
                }
            }
            QuoteContext::Double => self.process_double_quoted(ch, peek, peek2),
            QuoteContext::Backtick => {
                if ch == '`' {
                    self.pop();
                    QuoteAction::Consumed
                } else if ch == '\\' {
                    // In backtick substitution, backslash only escapes
                    // $, `, \, and newline.
                    QuoteAction::Literal
                } else {
                    QuoteAction::Literal
                }
            }
            QuoteContext::ParamExpansion => {
                if ch == '}' {
                    if self.brace_depth > 0 {
                        self.brace_depth -= 1;
                        QuoteAction::Literal
                    } else {
                        self.pop();
                        QuoteAction::Consumed
                    }
                } else if ch == '{' {
                    self.brace_depth += 1;
                    QuoteAction::Literal
                } else {
                    QuoteAction::Literal
                }
            }
        }
    }

    fn process_unquoted(
        &mut self,
        ch: char,
        peek: Option<char>,
        peek2: Option<char>,
    ) -> QuoteAction {
        match ch {
            '\'' => {
                self.push(QuoteContext::Single);
                QuoteAction::Consumed
            }
            '"' => {
                self.push(QuoteContext::Double);
                QuoteAction::Consumed
            }
            '`' => {
                self.push(QuoteContext::Backtick);
                QuoteAction::Consumed
            }
            '$' => {
                match peek {
                    Some('(') => {
                        if peek2 == Some('(') {
                            self.push(QuoteContext::ArithmeticSub);
                            QuoteAction::StartArithSub // consume `$((`, caller
                                                       // skips 2 chars
                        } else {
                            self.push(QuoteContext::CommandSub);
                            self.paren_depth = 0;
                            QuoteAction::StartCmdSub // consume `$(`, caller
                                                     // skips 1 char
                        }
                    }
                    Some('{') => {
                        self.push(QuoteContext::ParamExpansion);
                        self.brace_depth = 0;
                        QuoteAction::StartParamExp // consume `${`, caller skips
                                                   // 1 char
                    }
                    _ => QuoteAction::Literal,
                }
            }
            ')' if self.current() == QuoteContext::CommandSub => {
                if self.paren_depth > 0 {
                    self.paren_depth -= 1;
                    QuoteAction::Literal
                } else {
                    self.pop();
                    QuoteAction::Consumed
                }
            }
            '(' if self.current() == QuoteContext::CommandSub => {
                self.paren_depth += 1;
                QuoteAction::Literal
            }
            ')' if self.current() == QuoteContext::ArithmeticSub => {
                // Need `))` to close arithmetic
                if peek == Some(')') {
                    self.pop();
                    QuoteAction::EndArithSub // consume `))`, caller skips 1
                                             // char
                } else {
                    QuoteAction::Literal
                }
            }
            _ => QuoteAction::Literal,
        }
    }

    fn process_double_quoted(
        &mut self,
        ch: char,
        peek: Option<char>,
        peek2: Option<char>,
    ) -> QuoteAction {
        match ch {
            '"' => {
                self.pop();
                QuoteAction::Consumed
            }
            '\\' => {
                // In double quotes, backslash escapes $, `, ", \, and newline.
                match peek {
                    Some('$') | Some('`') | Some('"') | Some('\\') | Some('\n') => {
                        QuoteAction::EscapeNext
                    }
                    _ => QuoteAction::Literal,
                }
            }
            '$' => match peek {
                Some('(') => {
                    if peek2 == Some('(') {
                        self.push(QuoteContext::ArithmeticSub);
                        QuoteAction::StartArithSub
                    } else {
                        self.push(QuoteContext::CommandSub);
                        self.paren_depth = 0;
                        QuoteAction::StartCmdSub
                    }
                }
                Some('{') => {
                    self.push(QuoteContext::ParamExpansion);
                    self.brace_depth = 0;
                    QuoteAction::StartParamExp
                }
                _ => QuoteAction::Literal,
            },
            '`' => {
                self.push(QuoteContext::Backtick);
                QuoteAction::Consumed
            }
            _ => QuoteAction::Literal,
        }
    }
}

/// Action the lexer should take after the quote state machine processes
/// a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteAction {
    /// The character is a literal: add it to the current token.
    Literal,
    /// The character was consumed as a quote character (don't add to token).
    Consumed,
    /// The next character should be treated as an escaped literal.
    EscapeNext,
    /// `$(` was recognized -- skip the next char (`(`).
    StartCmdSub,
    /// `$((` was recognized -- skip the next 2 chars (`((`).
    StartArithSub,
    /// `${` was recognized -- skip the next char (`{`).
    StartParamExp,
    /// `))` was recognized -- skip the next char (`)`).
    EndArithSub,
}

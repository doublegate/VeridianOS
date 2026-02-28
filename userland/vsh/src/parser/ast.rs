//! Abstract Syntax Tree node types.
//!
//! Represents the full Bash grammar as a tree of typed nodes.

use alloc::{boxed::Box, string::String, vec::Vec};

/// Top-level program: a sequence of complete commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub commands: Vec<CompleteCommand>,
}

/// A complete command is an and-or list optionally followed by `&` or `;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteCommand {
    pub list: AndOrList,
    /// Whether to run in the background (`&` terminator).
    pub background: bool,
}

/// A list of pipelines connected by `&&` or `||`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndOrList {
    pub first: Pipeline,
    pub rest: Vec<(AndOrOp, Pipeline)>,
}

/// The operator connecting two pipelines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndOrOp {
    /// `&&`
    And,
    /// `||`
    Or,
}

/// A pipeline: one or more commands connected by `|`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    /// Whether the pipeline is negated with `!`.
    pub negated: bool,
    /// Whether to pipe stderr as well (`|&`).
    pub pipe_stderr: bool,
    /// Commands in the pipeline.
    pub commands: Vec<Command>,
    /// If `time` keyword preceded the pipeline.
    pub timed: bool,
}

/// A single command in a pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// A simple command: optional assignments, command words, and redirections.
    Simple(SimpleCommand),
    /// A compound command (if, while, for, case, etc.) with optional
    /// redirections.
    Compound(CompoundCommand, Vec<Redirect>),
    /// A function definition.
    FunctionDef(FunctionDef),
    /// A coprocess.
    Coproc(CoprocCommand),
}

/// A simple command: assignments, words, and redirections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    /// Variable assignments that precede the command.
    pub assignments: Vec<Assignment>,
    /// Command name and arguments.
    pub words: Vec<Word>,
    /// I/O redirections.
    pub redirects: Vec<Redirect>,
}

/// A variable assignment: `NAME=VALUE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub name: String,
    pub value: Word,
    /// Whether this is an append assignment (`+=`).
    pub append: bool,
}

/// A word in the command line. Contains raw text with embedded expansion
/// markers.  The expansion engine resolves these later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    /// The raw text of the word including quotes and expansion syntax.
    pub raw: String,
}

impl Word {
    pub fn new(raw: String) -> Self {
        Self { raw }
    }

    pub fn from_str(s: &str) -> Self {
        Self {
            raw: String::from(s),
        }
    }
}

/// An I/O redirection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    /// Optional file descriptor number (defaults depend on operator).
    pub fd: Option<i32>,
    /// The redirection operator.
    pub op: RedirectOp,
    /// The target (filename, fd number, or here-doc body).
    pub target: RedirectTarget,
}

/// Redirection operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectOp {
    /// `<` -- input from file
    Input,
    /// `>` -- output to file (truncate)
    Output,
    /// `>>` -- output to file (append)
    Append,
    /// `<<` -- here-document
    HereDoc,
    /// `<<-` -- here-document with tab stripping
    HereDocStrip,
    /// `<<<` -- here-string
    HereString,
    /// `<>` -- open for reading and writing
    ReadWrite,
    /// `>&` -- duplicate output fd
    DupOutput,
    /// `<&` -- duplicate input fd
    DupInput,
    /// `>|` -- output with clobber override
    Clobber,
    /// `&>` -- redirect stdout and stderr
    AndOutput,
    /// `&>>` -- append stdout and stderr
    AndAppend,
}

/// The target of a redirection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectTarget {
    /// A filename or word to be expanded.
    File(Word),
    /// A file descriptor number (for `>&N` or `<&N`).
    Fd(i32),
    /// Close the fd (`>&-` or `<&-`).
    Close,
    /// A here-document body.
    HereDocBody(String),
    /// A here-string value.
    HereString(Word),
}

/// Compound commands.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // All variants are part of the AST
pub enum CompoundCommand {
    /// `if cond; then body; [elif cond; then body;]... [else body;] fi`
    If(IfClause),
    /// `while cond; do body; done`
    While(WhileClause),
    /// `until cond; do body; done`
    Until(UntilClause),
    /// `for var [in words...]; do body; done`
    For(ForClause),
    /// `for ((init; cond; step)); do body; done`
    ArithFor(ArithForClause),
    /// `case word in [pattern) body ;;]... esac`
    Case(CaseClause),
    /// `select var [in words...]; do body; done`
    Select(SelectClause),
    /// `{ list; }` -- brace group
    BraceGroup(Vec<CompleteCommand>),
    /// `(list)` -- subshell
    Subshell(Vec<CompleteCommand>),
    /// `((expr))` -- arithmetic evaluation
    ArithEval(String),
    /// `[[ expr ]]` -- conditional expression
    ConditionalExpr(ConditionalExpr),
}

/// `if ... then ... elif ... else ... fi`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfClause {
    /// (condition, then-body) for `if` and each `elif`.
    pub branches: Vec<(Vec<CompleteCommand>, Vec<CompleteCommand>)>,
    /// Optional `else` body.
    pub else_body: Option<Vec<CompleteCommand>>,
}

/// `while cond; do body; done`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhileClause {
    pub condition: Vec<CompleteCommand>,
    pub body: Vec<CompleteCommand>,
}

/// `until cond; do body; done`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UntilClause {
    pub condition: Vec<CompleteCommand>,
    pub body: Vec<CompleteCommand>,
}

/// `for var [in words...]; do body; done`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForClause {
    pub var: String,
    /// The words to iterate over. If None, iterate over `$@`.
    pub words: Option<Vec<Word>>,
    pub body: Vec<CompleteCommand>,
}

/// `for ((init; cond; step)); do body; done`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArithForClause {
    pub init: String,
    pub condition: String,
    pub step: String,
    pub body: Vec<CompleteCommand>,
}

/// `case word in ... esac`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseClause {
    pub word: Word,
    pub items: Vec<CaseItem>,
}

/// A single case item: `pattern) body ;;`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseItem {
    /// One or more patterns separated by `|`.
    pub patterns: Vec<Word>,
    /// Commands to execute.
    pub body: Vec<CompleteCommand>,
    /// Terminator: `;;`, `;&`, or `;;&`.
    pub terminator: CaseTerminator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseTerminator {
    /// `;;` -- stop matching
    Break,
    /// `;&` -- fall through to next body
    FallThrough,
    /// `;;&` -- continue testing next pattern
    TestNext,
}

/// `select var [in words...]; do body; done`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectClause {
    pub var: String,
    pub words: Option<Vec<Word>>,
    pub body: Vec<CompleteCommand>,
}

/// A function definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDef {
    pub name: String,
    pub body: Box<Command>,
}

/// A coprocess command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoprocCommand {
    /// Optional name (defaults to "COPROC").
    pub name: Option<String>,
    pub body: Box<Command>,
}

/// `[[ ... ]]` conditional expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionalExpr {
    /// Unary test: `-f file`, `-z string`, etc.
    Unary(String, Word),
    /// Binary test: `str1 == str2`, `n1 -eq n2`, etc.
    Binary(Word, String, Word),
    /// Regex match: `str =~ regex`
    Regex(Word, Word),
    /// Logical NOT: `! expr`
    Not(Box<ConditionalExpr>),
    /// Logical AND: `expr && expr`
    And(Box<ConditionalExpr>, Box<ConditionalExpr>),
    /// Logical OR: `expr || expr`
    Or(Box<ConditionalExpr>, Box<ConditionalExpr>),
    /// Parenthesized group: `( expr )`
    Group(Box<ConditionalExpr>),
    /// A bare word (evaluated as `-n word`)
    Word(Word),
}

//! Shell scripting engine for the VeridianOS shell.
//!
//! Provides parsing and representation of control flow constructs:
//! - `if`/`elif`/`else`/`fi`
//! - `while`/`do`/`done`
//! - `for var in words...; do ... done`
//! - `case word in pattern) commands ;; esac`
//!
//! Also includes:
//! - `test` / `[` builtin evaluation
//! - Arithmetic expansion `$((expr))`
//! - Command substitution placeholder `$(command)`
//! - Block nesting with depth tracking
//!
//! The script engine does NOT directly execute commands. It parses script
//! lines into an AST-like structure ([`ScriptNode`]) that the shell can
//! then traverse and execute.

// Shell scripting -- parser/AST complete, execution deferred to Phase 6
#![allow(dead_code)]

use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

// ---------------------------------------------------------------------------
// AST types
// ---------------------------------------------------------------------------

/// A single node in the parsed script AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptNode {
    /// A simple command line (no control flow).
    Simple(String),

    /// An if/elif/else/fi block.
    If {
        /// The condition command for the initial `if`.
        condition: String,
        /// Commands to run when the condition is true.
        then_body: Vec<ScriptNode>,
        /// Zero or more `elif` branches: `(condition, body)`.
        elif_branches: Vec<(String, Vec<ScriptNode>)>,
        /// Commands to run if no condition matches (may be empty).
        else_body: Vec<ScriptNode>,
    },

    /// A while/do/done loop.
    While {
        /// The condition command evaluated before each iteration.
        condition: String,
        /// The loop body.
        body: Vec<ScriptNode>,
    },

    /// A for/in/do/done loop.
    For {
        /// The loop variable name.
        var: String,
        /// The list of words to iterate over.
        words: Vec<String>,
        /// The loop body.
        body: Vec<ScriptNode>,
    },

    /// A case/in/esac block.
    Case {
        /// The word being matched.
        word: String,
        /// Branches: each is `(patterns, body)` where patterns may contain
        /// glob-style wildcards.
        branches: Vec<(Vec<String>, Vec<ScriptNode>)>,
    },
}

// ---------------------------------------------------------------------------
// ScriptEngine
// ---------------------------------------------------------------------------

/// Shell scripting engine that parses script lines into an AST.
///
/// The engine maintains no mutable state between parse calls -- each
/// invocation of [`parse_script`] is independent.
pub struct ScriptEngine {
    /// Optional variable bindings for substitution during parsing.
    /// Not currently used but reserved for future expansion.
    _variables: BTreeMap<String, String>,
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptEngine {
    /// Create a new script engine.
    pub fn new() -> Self {
        Self {
            _variables: BTreeMap::new(),
        }
    }

    /// Parse a sequence of script lines into an AST.
    ///
    /// Returns a list of top-level [`ScriptNode`]s, or an error message
    /// if the script is malformed (e.g., unmatched `if`/`fi`).
    pub fn parse_script(&self, lines: &[String]) -> Result<Vec<ScriptNode>, String> {
        let mut pos = 0;
        let mut nodes = Vec::new();

        while pos < lines.len() {
            let (node, next_pos) = self.parse_node(lines, pos)?;
            if let Some(n) = node {
                nodes.push(n);
            }
            pos = next_pos;
        }

        Ok(nodes)
    }

    /// Execute a parsed script, returning the last exit code.
    ///
    /// This is a dry-run evaluator that parses the AST. The actual
    /// command execution is deferred to the shell. Returns 0 on
    /// successful parse, 1 on parse error.
    pub fn execute_script(&self, lines: &[String]) -> i32 {
        match self.parse_script(lines) {
            Ok(_nodes) => 0,
            Err(_) => 1,
        }
    }

    // -----------------------------------------------------------------------
    // Recursive-descent parser
    // -----------------------------------------------------------------------

    /// Parse a single node starting at `pos`.
    ///
    /// Returns `(Some(node), next_pos)` or `(None, next_pos)` for blank
    /// lines. Advances `next_pos` past the consumed lines.
    fn parse_node(
        &self,
        lines: &[String],
        pos: usize,
    ) -> Result<(Option<ScriptNode>, usize), String> {
        if pos >= lines.len() {
            return Ok((None, pos));
        }

        let line = lines[pos].trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            return Ok((None, pos + 1));
        }

        // Determine which construct we are entering
        let first_word = first_token(line);

        match first_word {
            "if" => self.parse_if(lines, pos),
            "while" => self.parse_while(lines, pos),
            "for" => self.parse_for(lines, pos),
            "case" => self.parse_case(lines, pos),
            _ => Ok((Some(ScriptNode::Simple(line.to_string())), pos + 1)),
        }
    }

    /// Parse an `if ... then ... elif ... else ... fi` block.
    fn parse_if(
        &self,
        lines: &[String],
        start: usize,
    ) -> Result<(Option<ScriptNode>, usize), String> {
        let first_line = lines[start].trim();

        // Extract condition: everything after "if" and before optional "; then"
        let after_if = strip_keyword(first_line, "if");
        let (condition, then_on_same_line) = split_then(after_if);

        if condition.is_empty() {
            return Err(format!(
                "syntax error: empty condition on line {}",
                start + 1
            ));
        }

        let mut pos = start + 1;

        // If "then" is not on the same line, look for it on the next line
        if !then_on_same_line {
            pos = skip_blank_and_comments(lines, pos);
            if pos >= lines.len() || first_token(lines[pos].trim()) != "then" {
                return Err(format!(
                    "syntax error: expected 'then' after 'if' condition near line {}",
                    start + 1
                ));
            }
            pos += 1;
        }

        // Parse the then-body until we hit elif, else, or fi
        let mut then_body = Vec::new();
        let mut elif_branches: Vec<(String, Vec<ScriptNode>)> = Vec::new();
        let mut else_body: Vec<ScriptNode> = Vec::new();

        enum IfSection {
            Then,
            Elif(String),
            Else,
        }

        let mut section = IfSection::Then;

        loop {
            if pos >= lines.len() {
                return Err(format!(
                    "syntax error: 'if' without matching 'fi' (started on line {})",
                    start + 1
                ));
            }

            let line = lines[pos].trim();

            // Skip blanks/comments inside body
            if line.is_empty() || line.starts_with('#') {
                pos += 1;
                continue;
            }

            let kw = first_token(line);

            if kw == "fi" {
                // Finalize the current elif section if needed
                if let IfSection::Elif(ref cond) = section {
                    // Check if we already pushed this elif branch
                    let already_pushed = elif_branches.last().map(|b| &b.0) == Some(cond);
                    if !already_pushed {
                        elif_branches.push((cond.clone(), Vec::new()));
                    }
                }
                // Close the if block
                return Ok((
                    Some(ScriptNode::If {
                        condition,
                        then_body,
                        elif_branches,
                        else_body,
                    }),
                    pos + 1,
                ));
            }

            if kw == "elif" {
                // Finish the previous elif section if any
                if let IfSection::Elif(ref cond) = section {
                    let already_pushed = elif_branches.last().map(|b| &b.0) == Some(cond);
                    if !already_pushed {
                        elif_branches.push((cond.clone(), Vec::new()));
                    }
                }

                let after_elif = strip_keyword(line, "elif");
                let (elif_cond, elif_then) = split_then(after_elif);
                if elif_cond.is_empty() {
                    return Err(format!(
                        "syntax error: empty 'elif' condition on line {}",
                        pos + 1
                    ));
                }

                if let IfSection::Else = section {
                    return Err(format!(
                        "syntax error: 'elif' after 'else' on line {}",
                        pos + 1
                    ));
                }

                // Start a new elif branch
                elif_branches.push((elif_cond.clone(), Vec::new()));
                section = IfSection::Elif(elif_cond);
                pos += 1;

                if !elif_then {
                    let next = skip_blank_and_comments(lines, pos);
                    if next >= lines.len() || first_token(lines[next].trim()) != "then" {
                        return Err(format!(
                            "syntax error: expected 'then' after 'elif' near line {}",
                            pos
                        ));
                    }
                    pos = next + 1;
                }

                continue;
            }

            if kw == "else" {
                // Finish the previous elif section if any
                if let IfSection::Elif(ref cond) = section {
                    let already_pushed = elif_branches.last().map(|b| &b.0) == Some(cond);
                    if !already_pushed {
                        elif_branches.push((cond.clone(), Vec::new()));
                    }
                }
                section = IfSection::Else;
                pos += 1;
                continue;
            }

            // Parse a body node
            let (node, next_pos) = self.parse_node(lines, pos)?;
            if let Some(n) = node {
                match section {
                    IfSection::Then => then_body.push(n),
                    IfSection::Elif(_) => {
                        if let Some(last) = elif_branches.last_mut() {
                            last.1.push(n);
                        }
                    }
                    IfSection::Else => else_body.push(n),
                }
            }
            pos = next_pos;
        }
    }

    /// Parse a `while condition; do ... done` block.
    fn parse_while(
        &self,
        lines: &[String],
        start: usize,
    ) -> Result<(Option<ScriptNode>, usize), String> {
        let first_line = lines[start].trim();
        let after_while = strip_keyword(first_line, "while");
        let (condition, do_on_same_line) = split_do(after_while);

        if condition.is_empty() {
            return Err(format!(
                "syntax error: empty 'while' condition on line {}",
                start + 1
            ));
        }

        let mut pos = start + 1;

        if !do_on_same_line {
            pos = skip_blank_and_comments(lines, pos);
            if pos >= lines.len() || first_token(lines[pos].trim()) != "do" {
                return Err(format!(
                    "syntax error: expected 'do' after 'while' near line {}",
                    start + 1
                ));
            }
            pos += 1;
        }

        let mut body = Vec::new();
        loop {
            if pos >= lines.len() {
                return Err(format!(
                    "syntax error: 'while' without matching 'done' (started on line {})",
                    start + 1
                ));
            }

            let line = lines[pos].trim();
            if line.is_empty() || line.starts_with('#') {
                pos += 1;
                continue;
            }

            if first_token(line) == "done" {
                return Ok((Some(ScriptNode::While { condition, body }), pos + 1));
            }

            let (node, next_pos) = self.parse_node(lines, pos)?;
            if let Some(n) = node {
                body.push(n);
            }
            pos = next_pos;
        }
    }

    /// Parse a `for var in word...; do ... done` block.
    fn parse_for(
        &self,
        lines: &[String],
        start: usize,
    ) -> Result<(Option<ScriptNode>, usize), String> {
        let first_line = lines[start].trim();
        let after_for = strip_keyword(first_line, "for");

        // Expected format: "var in word1 word2 ..." possibly followed by "; do"
        let (var, words, do_on_same_line) = parse_for_header(after_for)?;

        if var.is_empty() {
            return Err(format!(
                "syntax error: missing variable in 'for' on line {}",
                start + 1
            ));
        }

        let mut pos = start + 1;

        if !do_on_same_line {
            pos = skip_blank_and_comments(lines, pos);
            if pos >= lines.len() || first_token(lines[pos].trim()) != "do" {
                return Err(format!(
                    "syntax error: expected 'do' after 'for' near line {}",
                    start + 1
                ));
            }
            pos += 1;
        }

        let mut body = Vec::new();
        loop {
            if pos >= lines.len() {
                return Err(format!(
                    "syntax error: 'for' without matching 'done' (started on line {})",
                    start + 1
                ));
            }

            let line = lines[pos].trim();
            if line.is_empty() || line.starts_with('#') {
                pos += 1;
                continue;
            }

            if first_token(line) == "done" {
                return Ok((Some(ScriptNode::For { var, words, body }), pos + 1));
            }

            let (node, next_pos) = self.parse_node(lines, pos)?;
            if let Some(n) = node {
                body.push(n);
            }
            pos = next_pos;
        }
    }

    /// Parse a `case word in ... esac` block.
    fn parse_case(
        &self,
        lines: &[String],
        start: usize,
    ) -> Result<(Option<ScriptNode>, usize), String> {
        let first_line = lines[start].trim();
        let after_case = strip_keyword(first_line, "case");

        // Expected: "word in"
        let (word, has_in) = parse_case_header(after_case);
        if word.is_empty() {
            return Err(format!(
                "syntax error: missing word in 'case' on line {}",
                start + 1
            ));
        }

        let mut pos = start + 1;

        if !has_in {
            pos = skip_blank_and_comments(lines, pos);
            if pos >= lines.len() || lines[pos].trim() != "in" {
                return Err(format!(
                    "syntax error: expected 'in' after 'case' near line {}",
                    start + 1
                ));
            }
            pos += 1;
        }

        let mut branches: Vec<(Vec<String>, Vec<ScriptNode>)> = Vec::new();

        loop {
            if pos >= lines.len() {
                return Err(format!(
                    "syntax error: 'case' without matching 'esac' (started on line {})",
                    start + 1
                ));
            }

            let line = lines[pos].trim();
            if line.is_empty() || line.starts_with('#') {
                pos += 1;
                continue;
            }

            if line == "esac" {
                return Ok((Some(ScriptNode::Case { word, branches }), pos + 1));
            }

            // Parse a case branch: "pattern1|pattern2) commands ;;"
            let (patterns, branch_body, next_pos) = self.parse_case_branch(lines, pos)?;
            branches.push((patterns, branch_body));
            pos = next_pos;
        }
    }

    /// Parse a single case branch starting at `pos`.
    ///
    /// Returns `(patterns, body, next_pos)`.
    fn parse_case_branch(
        &self,
        lines: &[String],
        start: usize,
    ) -> Result<(Vec<String>, Vec<ScriptNode>, usize), String> {
        let line = lines[start].trim();

        // Find the ")" delimiter that separates patterns from commands
        let paren_pos = line.find(')').ok_or_else(|| {
            format!(
                "syntax error: expected ')' in case pattern on line {}",
                start + 1
            )
        })?;

        let pattern_str = &line[..paren_pos];
        let patterns: Vec<String> = pattern_str
            .split('|')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();

        if patterns.is_empty() {
            return Err(format!(
                "syntax error: empty pattern in case branch on line {}",
                start + 1
            ));
        }

        // Check for inline commands after ")"
        let after_paren = line[paren_pos + 1..].trim();

        // If the line ends with ";;", it is a single-line branch
        if let Some(cmd) = after_paren.strip_suffix(";;") {
            let cmd = cmd.trim();
            let body = if cmd.is_empty() {
                Vec::new()
            } else {
                vec![ScriptNode::Simple(cmd.to_string())]
            };
            return Ok((patterns, body, start + 1));
        }

        // Multi-line branch: collect body until ";;"
        let mut body = Vec::new();
        let mut pos = if after_paren.is_empty() {
            start + 1
        } else {
            // There is a command on the pattern line itself
            body.push(ScriptNode::Simple(after_paren.to_string()));
            start + 1
        };

        loop {
            if pos >= lines.len() {
                return Err(format!(
                    "syntax error: case branch without ';;' (started on line {})",
                    start + 1
                ));
            }

            let bline = lines[pos].trim();
            if bline.is_empty() || bline.starts_with('#') {
                pos += 1;
                continue;
            }

            if bline == ";;" {
                return Ok((patterns, body, pos + 1));
            }

            // Check if line ends with ";;"
            if let Some(cmd) = bline.strip_suffix(";;") {
                let cmd = cmd.trim();
                if !cmd.is_empty() {
                    body.push(ScriptNode::Simple(cmd.to_string()));
                }
                return Ok((patterns, body, pos + 1));
            }

            // Check for esac (branch implicitly closed)
            if bline == "esac" {
                return Ok((patterns, body, pos));
            }

            let (node, next_pos) = self.parse_node(lines, pos)?;
            if let Some(n) = node {
                body.push(n);
            }
            pos = next_pos;
        }
    }
}

// ---------------------------------------------------------------------------
// Test / bracket builtin evaluator
// ---------------------------------------------------------------------------

/// Evaluate a `test` / `[` expression.
///
/// Supports:
/// - `-f path` -- file exists (checks VFS)
/// - `-d path` -- directory exists (checks VFS)
/// - `-z string` -- string is empty
/// - `-n string` -- string is non-empty
/// - `s1 = s2` -- string equality
/// - `s1 != s2` -- string inequality
/// - `n1 -eq n2` -- integer equality
/// - `n1 -ne n2` -- integer inequality
/// - `n1 -lt n2` -- integer less than
/// - `n1 -gt n2` -- integer greater than
/// - `! expr` -- logical negation (single-level)
///
/// The `args` slice should NOT include the `test` or `[` / `]` tokens
/// themselves.
pub fn evaluate_test(args: &[String]) -> bool {
    if args.is_empty() {
        return false;
    }

    // Handle negation
    if args[0] == "!" {
        return !evaluate_test(&args[1..]);
    }

    // Unary operators
    if args.len() >= 2 {
        match args[0].as_str() {
            "-f" => return test_file_exists(&args[1]),
            "-d" => return test_dir_exists(&args[1]),
            "-z" => return args[1].is_empty(),
            "-n" => return !args[1].is_empty(),
            _ => {}
        }
    }

    // Binary operators (requires exactly 3 args)
    if args.len() >= 3 {
        let left = &args[0];
        let op = &args[1];
        let right = &args[2];

        match op.as_str() {
            "=" => return left == right,
            "!=" => return left != right,
            "-eq" => {
                if let (Ok(l), Ok(r)) = (parse_i64(left), parse_i64(right)) {
                    return l == r;
                }
                return false;
            }
            "-ne" => {
                if let (Ok(l), Ok(r)) = (parse_i64(left), parse_i64(right)) {
                    return l != r;
                }
                return false;
            }
            "-lt" => {
                if let (Ok(l), Ok(r)) = (parse_i64(left), parse_i64(right)) {
                    return l < r;
                }
                return false;
            }
            "-gt" => {
                if let (Ok(l), Ok(r)) = (parse_i64(left), parse_i64(right)) {
                    return l > r;
                }
                return false;
            }
            _ => {}
        }
    }

    // Single-argument: true if non-empty
    if args.len() == 1 {
        return !args[0].is_empty();
    }

    false
}

/// Check whether a file exists in the VFS.
fn test_file_exists(path: &str) -> bool {
    if let Some(vfs) = crate::fs::try_get_vfs() {
        let vfs_guard = vfs.read();
        if let Ok(node) = vfs_guard.resolve_path(path) {
            if let Ok(meta) = node.metadata() {
                return meta.node_type == crate::fs::NodeType::File;
            }
        }
    }
    false
}

/// Check whether a directory exists in the VFS.
fn test_dir_exists(path: &str) -> bool {
    if let Some(vfs) = crate::fs::try_get_vfs() {
        let vfs_guard = vfs.read();
        if let Ok(node) = vfs_guard.resolve_path(path) {
            if let Ok(meta) = node.metadata() {
                return meta.node_type == crate::fs::NodeType::Directory;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Arithmetic expansion
// ---------------------------------------------------------------------------

/// Evaluate an arithmetic expression (the contents of `$(( ... ))`).
///
/// Supports:
/// - Integer literals (decimal, including negative via unary minus)
/// - Binary operators: `+`, `-`, `*`, `/`, `%`
/// - Parenthesized sub-expressions
/// - Whitespace between tokens
///
/// Returns the computed value or an error message.
pub fn evaluate_arithmetic(expr: &str) -> Result<i64, String> {
    let tokens = tokenize_arithmetic(expr)?;
    if tokens.is_empty() {
        return Err("empty arithmetic expression".to_string());
    }
    let mut pos = 0;
    let result = parse_expr(&tokens, &mut pos)?;
    if pos < tokens.len() {
        return Err(format!(
            "unexpected token '{}' in arithmetic expression",
            tokens[pos]
        ));
    }
    Ok(result)
}

/// Expand command substitution markers in a string.
///
/// Replaces `$(command)` with a placeholder marker since actual command
/// execution is deferred to the shell. Returns the command strings found.
pub fn extract_command_substitutions(input: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if i + 1 < len && chars[i] == '$' && chars[i + 1] == '(' {
            // Check for arithmetic: $((
            if i + 2 < len && chars[i + 2] == '(' {
                // Skip $(( ... )) -- this is arithmetic, not command subst
                i += 3;
                let mut depth = 1;
                while i < len && depth > 0 {
                    if i + 1 < len && chars[i] == ')' && chars[i + 1] == ')' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }

            // Command substitution $(...)
            i += 2; // skip "$("
            let start = i;
            let mut depth = 1;
            while i < len && depth > 0 {
                if chars[i] == '(' {
                    depth += 1;
                } else if chars[i] == ')' {
                    depth -= 1;
                }
                if depth > 0 {
                    i += 1;
                }
            }
            let cmd: String = chars[start..i].iter().collect();
            commands.push(cmd.trim().to_string());
            if i < len {
                i += 1; // skip closing ')'
            }
        } else {
            i += 1;
        }
    }

    commands
}

// ---------------------------------------------------------------------------
// Arithmetic tokenizer and parser (recursive descent)
// ---------------------------------------------------------------------------

/// Token types for arithmetic expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ArithToken {
    Number(i64),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    LParen,
    RParen,
}

impl core::fmt::Display for ArithToken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ArithToken::Number(n) => write!(f, "{}", n),
            ArithToken::Plus => write!(f, "+"),
            ArithToken::Minus => write!(f, "-"),
            ArithToken::Star => write!(f, "*"),
            ArithToken::Slash => write!(f, "/"),
            ArithToken::Percent => write!(f, "%"),
            ArithToken::LParen => write!(f, "("),
            ArithToken::RParen => write!(f, ")"),
        }
    }
}

/// Tokenize an arithmetic expression string.
fn tokenize_arithmetic(expr: &str) -> Result<Vec<ArithToken>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        if ch.is_whitespace() {
            i += 1;
            continue;
        }

        match ch {
            '+' => {
                tokens.push(ArithToken::Plus);
                i += 1;
            }
            '-' => {
                // Determine if this is unary minus or binary minus.
                // Unary if: first token, or preceded by an operator or '('
                let is_unary = tokens.is_empty()
                    || matches!(
                        tokens.last(),
                        Some(
                            ArithToken::Plus
                                | ArithToken::Minus
                                | ArithToken::Star
                                | ArithToken::Slash
                                | ArithToken::Percent
                                | ArithToken::LParen
                        )
                    );

                if is_unary && i + 1 < len && chars[i + 1].is_ascii_digit() {
                    // Negative number literal
                    i += 1;
                    let start = i;
                    while i < len && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                    let num_str: String = chars[start..i].iter().collect();
                    let n = parse_i64(&num_str)
                        .map_err(|_| format!("invalid number in arithmetic: -{}", num_str))?;
                    tokens.push(ArithToken::Number(-n));
                } else {
                    tokens.push(ArithToken::Minus);
                    i += 1;
                }
            }
            '*' => {
                tokens.push(ArithToken::Star);
                i += 1;
            }
            '/' => {
                tokens.push(ArithToken::Slash);
                i += 1;
            }
            '%' => {
                tokens.push(ArithToken::Percent);
                i += 1;
            }
            '(' => {
                tokens.push(ArithToken::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(ArithToken::RParen);
                i += 1;
            }
            _ if ch.is_ascii_digit() => {
                let start = i;
                while i < len && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let num_str: String = chars[start..i].iter().collect();
                let n = parse_i64(&num_str)
                    .map_err(|_| format!("invalid number in arithmetic: {}", num_str))?;
                tokens.push(ArithToken::Number(n));
            }
            _ => {
                return Err(format!(
                    "unexpected character '{}' in arithmetic expression",
                    ch
                ));
            }
        }
    }

    Ok(tokens)
}

/// Parse an expression: handles `+` and `-` (lowest precedence).
fn parse_expr(tokens: &[ArithToken], pos: &mut usize) -> Result<i64, String> {
    let mut left = parse_term(tokens, pos)?;

    while *pos < tokens.len() {
        match tokens[*pos] {
            ArithToken::Plus => {
                *pos += 1;
                let right = parse_term(tokens, pos)?;
                left = left.wrapping_add(right);
            }
            ArithToken::Minus => {
                *pos += 1;
                let right = parse_term(tokens, pos)?;
                left = left.wrapping_sub(right);
            }
            _ => break,
        }
    }

    Ok(left)
}

/// Parse a term: handles `*`, `/`, `%` (higher precedence).
fn parse_term(tokens: &[ArithToken], pos: &mut usize) -> Result<i64, String> {
    let mut left = parse_factor(tokens, pos)?;

    while *pos < tokens.len() {
        match tokens[*pos] {
            ArithToken::Star => {
                *pos += 1;
                let right = parse_factor(tokens, pos)?;
                left = left.wrapping_mul(right);
            }
            ArithToken::Slash => {
                *pos += 1;
                let right = parse_factor(tokens, pos)?;
                if right == 0 {
                    return Err("division by zero".to_string());
                }
                left /= right;
            }
            ArithToken::Percent => {
                *pos += 1;
                let right = parse_factor(tokens, pos)?;
                if right == 0 {
                    return Err("modulo by zero".to_string());
                }
                left %= right;
            }
            _ => break,
        }
    }

    Ok(left)
}

/// Parse a factor: a number or a parenthesized expression.
fn parse_factor(tokens: &[ArithToken], pos: &mut usize) -> Result<i64, String> {
    if *pos >= tokens.len() {
        return Err("unexpected end of arithmetic expression".to_string());
    }

    match &tokens[*pos] {
        ArithToken::Number(n) => {
            let val = *n;
            *pos += 1;
            Ok(val)
        }
        ArithToken::LParen => {
            *pos += 1; // skip '('
            let val = parse_expr(tokens, pos)?;
            if *pos >= tokens.len() || tokens[*pos] != ArithToken::RParen {
                return Err("unmatched '(' in arithmetic expression".to_string());
            }
            *pos += 1; // skip ')'
            Ok(val)
        }
        ArithToken::Minus => {
            // Unary minus before a sub-expression
            *pos += 1;
            let val = parse_factor(tokens, pos)?;
            Ok(-val)
        }
        other => Err(format!(
            "unexpected token '{}' in arithmetic expression",
            other
        )),
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Extract the first whitespace-delimited token from a line.
fn first_token(line: &str) -> &str {
    line.split_whitespace().next().unwrap_or("")
}

/// Strip a leading keyword from a line and return the remainder (trimmed).
fn strip_keyword<'a>(line: &'a str, keyword: &str) -> &'a str {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix(keyword) {
        // Ensure the keyword is followed by whitespace or end
        if rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with(';') {
            rest.trim_start()
        } else {
            trimmed
        }
    } else {
        trimmed
    }
}

/// Split a condition from a trailing "; then" or standalone "then".
///
/// Returns `(condition, has_then)`.
fn split_then(input: &str) -> (String, bool) {
    // Check for "; then" at end
    if let Some(stripped) = input.strip_suffix("then") {
        let stripped = stripped.trim_end();
        if let Some(cond) = stripped.strip_suffix(';') {
            return (cond.trim().to_string(), true);
        }
    }

    // Check for inline "; then" anywhere
    if let Some(pos) = input.find("; then") {
        let cond = input[..pos].trim().to_string();
        return (cond, true);
    }

    (input.trim().to_string(), false)
}

/// Split a condition from a trailing "; do" or standalone "do".
///
/// Returns `(condition, has_do)`.
fn split_do(input: &str) -> (String, bool) {
    // Check for "; do" at end
    if let Some(pos) = input.find("; do") {
        let cond = input[..pos].trim().to_string();
        return (cond, true);
    }

    (input.trim().to_string(), false)
}

/// Parse the header of a `for` loop: "var in word1 word2 ...; do"
///
/// Returns `(var, words, has_do)`.
fn parse_for_header(input: &str) -> Result<(String, Vec<String>, bool), String> {
    let input = input.trim();

    // Check for "; do" suffix
    let (header, has_do) = if let Some(pos) = input.find("; do") {
        (&input[..pos], true)
    } else {
        (input, false)
    };

    let parts: Vec<&str> = header.split_whitespace().collect();

    if parts.is_empty() {
        return Err("syntax error: missing variable in 'for'".to_string());
    }

    let var = parts[0].to_string();

    // Check for "in" keyword
    if parts.len() < 2 || parts[1] != "in" {
        return Err("syntax error: expected 'in' after variable in 'for'".to_string());
    }

    let words: Vec<String> = parts[2..].iter().map(|w| w.to_string()).collect();

    Ok((var, words, has_do))
}

/// Parse the header of a `case` block: "word in"
///
/// Returns `(word, has_in)`.
fn parse_case_header(input: &str) -> (String, bool) {
    let input = input.trim();

    // Look for trailing " in"
    if let Some(stripped) = input.strip_suffix(" in") {
        return (stripped.trim().to_string(), true);
    }

    // "in" on the same line as the last token
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() >= 2 && parts[parts.len() - 1] == "in" {
        let word = parts[..parts.len() - 1].join(" ");
        return (word, true);
    }

    // No "in" found on this line
    if parts.is_empty() {
        (String::new(), false)
    } else {
        (parts.join(" "), false)
    }
}

/// Skip blank lines and comment lines, returning the next meaningful line
/// index.
fn skip_blank_and_comments(lines: &[String], start: usize) -> usize {
    let mut pos = start;
    while pos < lines.len() {
        let line = lines[pos].trim();
        if line.is_empty() || line.starts_with('#') {
            pos += 1;
        } else {
            break;
        }
    }
    pos
}

/// Parse a string as i64 without relying on std.
fn parse_i64(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty number".to_string());
    }

    let (negative, digits) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (false, rest)
    } else {
        (false, s)
    };

    if digits.is_empty() {
        return Err(format!("invalid number: {}", s));
    }

    let mut result: i64 = 0;
    for ch in digits.chars() {
        if !ch.is_ascii_digit() {
            return Err(format!("invalid number: {}", s));
        }
        let digit = (ch as u8 - b'0') as i64;
        result = result
            .checked_mul(10)
            .and_then(|r| r.checked_add(digit))
            .ok_or_else(|| format!("number overflow: {}", s))?;
    }

    if negative {
        Ok(-result)
    } else {
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(input: &[&str]) -> Vec<String> {
        input.iter().map(|s| s.to_string()).collect()
    }

    fn args(input: &[&str]) -> Vec<String> {
        input.iter().map(|s| s.to_string()).collect()
    }

    // ---- parse_i64 ----

    #[test]
    fn test_parse_i64_positive() {
        assert_eq!(parse_i64("42"), Ok(42));
    }

    #[test]
    fn test_parse_i64_negative() {
        assert_eq!(parse_i64("-7"), Ok(-7));
    }

    #[test]
    fn test_parse_i64_zero() {
        assert_eq!(parse_i64("0"), Ok(0));
    }

    #[test]
    fn test_parse_i64_with_plus() {
        assert_eq!(parse_i64("+10"), Ok(10));
    }

    #[test]
    fn test_parse_i64_invalid() {
        assert!(parse_i64("abc").is_err());
    }

    #[test]
    fn test_parse_i64_empty() {
        assert!(parse_i64("").is_err());
    }

    // ---- evaluate_arithmetic ----

    #[test]
    fn test_arith_simple_add() {
        assert_eq!(evaluate_arithmetic("1 + 2"), Ok(3));
    }

    #[test]
    fn test_arith_subtract() {
        assert_eq!(evaluate_arithmetic("10 - 3"), Ok(7));
    }

    #[test]
    fn test_arith_multiply() {
        assert_eq!(evaluate_arithmetic("4 * 5"), Ok(20));
    }

    #[test]
    fn test_arith_divide() {
        assert_eq!(evaluate_arithmetic("20 / 4"), Ok(5));
    }

    #[test]
    fn test_arith_modulo() {
        assert_eq!(evaluate_arithmetic("17 % 5"), Ok(2));
    }

    #[test]
    fn test_arith_precedence() {
        // Multiplication before addition
        assert_eq!(evaluate_arithmetic("2 + 3 * 4"), Ok(14));
    }

    #[test]
    fn test_arith_parentheses() {
        assert_eq!(evaluate_arithmetic("(2 + 3) * 4"), Ok(20));
    }

    #[test]
    fn test_arith_nested_parens() {
        assert_eq!(evaluate_arithmetic("((1 + 2) * (3 + 4))"), Ok(21));
    }

    #[test]
    fn test_arith_negative_number() {
        assert_eq!(evaluate_arithmetic("-5 + 3"), Ok(-2));
    }

    #[test]
    fn test_arith_division_by_zero() {
        assert!(evaluate_arithmetic("10 / 0").is_err());
    }

    #[test]
    fn test_arith_modulo_by_zero() {
        assert!(evaluate_arithmetic("10 % 0").is_err());
    }

    #[test]
    fn test_arith_single_number() {
        assert_eq!(evaluate_arithmetic("42"), Ok(42));
    }

    #[test]
    fn test_arith_complex_expression() {
        // (10 + 5) * 2 - 30 / 6 = 15 * 2 - 5 = 30 - 5 = 25
        assert_eq!(evaluate_arithmetic("(10 + 5) * 2 - 30 / 6"), Ok(25));
    }

    #[test]
    fn test_arith_empty_expression() {
        assert!(evaluate_arithmetic("").is_err());
    }

    #[test]
    fn test_arith_whitespace_only() {
        assert!(evaluate_arithmetic("   ").is_err());
    }

    // ---- evaluate_test ----

    #[test]
    fn test_eval_empty() {
        assert!(!evaluate_test(&[]));
    }

    #[test]
    fn test_eval_string_nonempty() {
        assert!(evaluate_test(&args(&["hello"])));
    }

    #[test]
    fn test_eval_z_empty() {
        assert!(evaluate_test(&args(&["-z", ""])));
    }

    #[test]
    fn test_eval_z_nonempty() {
        assert!(!evaluate_test(&args(&["-z", "hello"])));
    }

    #[test]
    fn test_eval_n_empty() {
        assert!(!evaluate_test(&args(&["-n", ""])));
    }

    #[test]
    fn test_eval_n_nonempty() {
        assert!(evaluate_test(&args(&["-n", "hello"])));
    }

    #[test]
    fn test_eval_string_equal() {
        assert!(evaluate_test(&args(&["foo", "=", "foo"])));
    }

    #[test]
    fn test_eval_string_not_equal() {
        assert!(evaluate_test(&args(&["foo", "!=", "bar"])));
    }

    #[test]
    fn test_eval_string_equal_fails() {
        assert!(!evaluate_test(&args(&["foo", "=", "bar"])));
    }

    #[test]
    fn test_eval_int_eq() {
        assert!(evaluate_test(&args(&["42", "-eq", "42"])));
    }

    #[test]
    fn test_eval_int_ne() {
        assert!(evaluate_test(&args(&["1", "-ne", "2"])));
    }

    #[test]
    fn test_eval_int_lt() {
        assert!(evaluate_test(&args(&["3", "-lt", "10"])));
    }

    #[test]
    fn test_eval_int_gt() {
        assert!(evaluate_test(&args(&["10", "-gt", "3"])));
    }

    #[test]
    fn test_eval_int_lt_false() {
        assert!(!evaluate_test(&args(&["10", "-lt", "3"])));
    }

    #[test]
    fn test_eval_int_gt_false() {
        assert!(!evaluate_test(&args(&["3", "-gt", "10"])));
    }

    #[test]
    fn test_eval_negation() {
        assert!(!evaluate_test(&args(&["!", "hello"])));
        assert!(evaluate_test(&args(&["!", "-z", "hello"])));
    }

    // ---- ScriptNode parsing: simple commands ----

    #[test]
    fn test_parse_simple_command() {
        let engine = ScriptEngine::new();
        let input = lines(&["echo hello"]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], ScriptNode::Simple("echo hello".to_string()));
    }

    #[test]
    fn test_parse_multiple_simple_commands() {
        let engine = ScriptEngine::new();
        let input = lines(&["echo one", "echo two", "echo three"]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_parse_skips_blank_lines() {
        let engine = ScriptEngine::new();
        let input = lines(&["echo a", "", "  ", "echo b"]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_parse_skips_comments() {
        let engine = ScriptEngine::new();
        let input = lines(&["# this is a comment", "echo hello", "# another"]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], ScriptNode::Simple("echo hello".to_string()));
    }

    // ---- if/then/fi ----

    #[test]
    fn test_parse_if_simple() {
        let engine = ScriptEngine::new();
        let input = lines(&["if test -f /etc/config; then", "  echo found", "fi"]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::If {
                condition,
                then_body,
                elif_branches,
                else_body,
            } => {
                assert_eq!(condition, "test -f /etc/config");
                assert_eq!(then_body.len(), 1);
                assert!(elif_branches.is_empty());
                assert!(else_body.is_empty());
            }
            _ => panic!("expected If node"),
        }
    }

    #[test]
    fn test_parse_if_then_separate_line() {
        let engine = ScriptEngine::new();
        let input = lines(&["if test -z \"$VAR\"", "then", "  echo empty", "fi"]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::If {
                condition,
                then_body,
                ..
            } => {
                assert_eq!(condition, "test -z \"$VAR\"");
                assert_eq!(then_body.len(), 1);
            }
            _ => panic!("expected If node"),
        }
    }

    #[test]
    fn test_parse_if_else() {
        let engine = ScriptEngine::new();
        let input = lines(&[
            "if test 1 -eq 2; then",
            "  echo yes",
            "else",
            "  echo no",
            "fi",
        ]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::If {
                then_body,
                else_body,
                ..
            } => {
                assert_eq!(then_body.len(), 1);
                assert_eq!(else_body.len(), 1);
                assert_eq!(else_body[0], ScriptNode::Simple("echo no".to_string()));
            }
            _ => panic!("expected If node"),
        }
    }

    #[test]
    fn test_parse_if_elif() {
        let engine = ScriptEngine::new();
        let input = lines(&[
            "if test $x -eq 1; then",
            "  echo one",
            "elif test $x -eq 2; then",
            "  echo two",
            "else",
            "  echo other",
            "fi",
        ]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::If {
                condition,
                then_body,
                elif_branches,
                else_body,
            } => {
                assert_eq!(condition, "test $x -eq 1");
                assert_eq!(then_body.len(), 1);
                assert_eq!(elif_branches.len(), 1);
                assert_eq!(elif_branches[0].0, "test $x -eq 2");
                assert_eq!(elif_branches[0].1.len(), 1);
                assert_eq!(else_body.len(), 1);
            }
            _ => panic!("expected If node"),
        }
    }

    #[test]
    fn test_parse_if_no_fi_error() {
        let engine = ScriptEngine::new();
        let input = lines(&["if test 1 -eq 1; then", "  echo hello"]);
        let result = engine.parse_script(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("without matching 'fi'"));
    }

    // ---- while/do/done ----

    #[test]
    fn test_parse_while_simple() {
        let engine = ScriptEngine::new();
        let input = lines(&["while test $i -lt 10; do", "  echo $i", "done"]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::While { condition, body } => {
                assert_eq!(condition, "test $i -lt 10");
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected While node"),
        }
    }

    #[test]
    fn test_parse_while_do_separate_line() {
        let engine = ScriptEngine::new();
        let input = lines(&["while test $running = true", "do", "  process_item", "done"]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::While { condition, body } => {
                assert_eq!(condition, "test $running = true");
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected While node"),
        }
    }

    #[test]
    fn test_parse_while_no_done_error() {
        let engine = ScriptEngine::new();
        let input = lines(&["while true; do", "  echo loop"]);
        let result = engine.parse_script(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("without matching 'done'"));
    }

    // ---- for/in/do/done ----

    #[test]
    fn test_parse_for_simple() {
        let engine = ScriptEngine::new();
        let input = lines(&["for f in a b c; do", "  echo $f", "done"]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::For { var, words, body } => {
                assert_eq!(var, "f");
                assert_eq!(words, &["a", "b", "c"]);
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected For node"),
        }
    }

    #[test]
    fn test_parse_for_do_separate_line() {
        let engine = ScriptEngine::new();
        let input = lines(&["for item in x y z", "do", "  process $item", "done"]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::For { var, words, body } => {
                assert_eq!(var, "item");
                assert_eq!(words, &["x", "y", "z"]);
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected For node"),
        }
    }

    #[test]
    fn test_parse_for_no_done_error() {
        let engine = ScriptEngine::new();
        let input = lines(&["for x in 1 2 3; do", "  echo $x"]);
        let result = engine.parse_script(&input);
        assert!(result.is_err());
    }

    // ---- case/in/esac ----

    #[test]
    fn test_parse_case_simple() {
        let engine = ScriptEngine::new();
        let input = lines(&[
            "case $opt in",
            "  a) echo alpha ;;",
            "  b) echo bravo ;;",
            "  *) echo unknown ;;",
            "esac",
        ]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::Case { word, branches } => {
                assert_eq!(word, "$opt");
                assert_eq!(branches.len(), 3);
                assert_eq!(branches[0].0, vec!["a"]);
                assert_eq!(branches[1].0, vec!["b"]);
                assert_eq!(branches[2].0, vec!["*"]);
            }
            _ => panic!("expected Case node"),
        }
    }

    #[test]
    fn test_parse_case_multi_pattern() {
        let engine = ScriptEngine::new();
        let input = lines(&[
            "case $color in",
            "  red|crimson) echo warm ;;",
            "  blue|cyan) echo cool ;;",
            "esac",
        ]);
        let result = engine.parse_script(&input).unwrap();
        match &result[0] {
            ScriptNode::Case { branches, .. } => {
                assert_eq!(branches[0].0, vec!["red", "crimson"]);
                assert_eq!(branches[1].0, vec!["blue", "cyan"]);
            }
            _ => panic!("expected Case node"),
        }
    }

    #[test]
    fn test_parse_case_multiline_branch() {
        let engine = ScriptEngine::new();
        let input = lines(&[
            "case $mode in",
            "  debug)",
            "    echo debug mode",
            "    echo verbose on",
            "  ;;",
            "esac",
        ]);
        let result = engine.parse_script(&input).unwrap();
        match &result[0] {
            ScriptNode::Case { branches, .. } => {
                assert_eq!(branches.len(), 1);
                assert_eq!(branches[0].0, vec!["debug"]);
                assert_eq!(branches[0].1.len(), 2);
            }
            _ => panic!("expected Case node"),
        }
    }

    #[test]
    fn test_parse_case_no_esac_error() {
        let engine = ScriptEngine::new();
        let input = lines(&["case $x in", "  a) echo a ;;"]);
        let result = engine.parse_script(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("without matching 'esac'"));
    }

    // ---- Nested constructs ----

    #[test]
    fn test_parse_nested_if_in_while() {
        let engine = ScriptEngine::new();
        let input = lines(&[
            "while test $i -lt 5; do",
            "  if test $i -eq 3; then",
            "    echo found",
            "  fi",
            "  echo next",
            "done",
        ]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::While { body, .. } => {
                assert_eq!(body.len(), 2);
                assert!(matches!(&body[0], ScriptNode::If { .. }));
                assert_eq!(body[1], ScriptNode::Simple("echo next".to_string()));
            }
            _ => panic!("expected While node"),
        }
    }

    #[test]
    fn test_parse_nested_for_in_if() {
        let engine = ScriptEngine::new();
        let input = lines(&[
            "if test -d /tmp; then",
            "  for f in a b; do",
            "    echo $f",
            "  done",
            "fi",
        ]);
        let result = engine.parse_script(&input).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            ScriptNode::If { then_body, .. } => {
                assert_eq!(then_body.len(), 1);
                assert!(matches!(&then_body[0], ScriptNode::For { .. }));
            }
            _ => panic!("expected If node"),
        }
    }

    // ---- Command substitution extraction ----

    #[test]
    fn test_extract_command_subst_simple() {
        let cmds = extract_command_substitutions("echo $(uname -r)");
        assert_eq!(cmds, vec!["uname -r"]);
    }

    #[test]
    fn test_extract_command_subst_multiple() {
        let cmds = extract_command_substitutions("$(cmd1) and $(cmd2)");
        assert_eq!(cmds, vec!["cmd1", "cmd2"]);
    }

    #[test]
    fn test_extract_command_subst_skips_arithmetic() {
        let cmds = extract_command_substitutions("echo $((1 + 2))");
        assert!(cmds.is_empty());
    }

    #[test]
    fn test_extract_command_subst_none() {
        let cmds = extract_command_substitutions("plain text");
        assert!(cmds.is_empty());
    }

    // ---- execute_script (dry run) ----

    #[test]
    fn test_execute_script_success() {
        let engine = ScriptEngine::new();
        let input = lines(&["echo hello", "echo world"]);
        assert_eq!(engine.execute_script(&input), 0);
    }

    #[test]
    fn test_execute_script_parse_error() {
        let engine = ScriptEngine::new();
        let input = lines(&["if test 1 -eq 1; then"]);
        assert_eq!(engine.execute_script(&input), 1);
    }

    // ---- Helper function tests ----

    #[test]
    fn test_first_token() {
        assert_eq!(first_token("if test -f foo; then"), "if");
        assert_eq!(first_token("  while true"), "while");
        assert_eq!(first_token(""), "");
    }

    #[test]
    fn test_strip_keyword() {
        assert_eq!(strip_keyword("if test -f /foo", "if"), "test -f /foo");
        assert_eq!(strip_keyword("while true; do", "while"), "true; do");
        assert_eq!(strip_keyword("for x in 1 2", "for"), "x in 1 2");
    }

    #[test]
    fn test_split_then() {
        let (cond, has) = split_then("test -f /etc/config; then");
        assert_eq!(cond, "test -f /etc/config");
        assert!(has);

        let (cond2, has2) = split_then("test -z \"$VAR\"");
        assert_eq!(cond2, "test -z \"$VAR\"");
        assert!(!has2);
    }

    #[test]
    fn test_split_do() {
        let (cond, has) = split_do("test $i -lt 10; do");
        assert_eq!(cond, "test $i -lt 10");
        assert!(has);

        let (cond2, has2) = split_do("test $running = true");
        assert_eq!(cond2, "test $running = true");
        assert!(!has2);
    }

    #[test]
    fn test_parse_for_header() {
        let (var, words, has_do) = parse_for_header("x in a b c; do").unwrap();
        assert_eq!(var, "x");
        assert_eq!(words, vec!["a", "b", "c"]);
        assert!(has_do);

        let (var2, words2, has_do2) = parse_for_header("item in 1 2 3").unwrap();
        assert_eq!(var2, "item");
        assert_eq!(words2, vec!["1", "2", "3"]);
        assert!(!has_do2);
    }

    #[test]
    fn test_parse_case_header() {
        let (word, has_in) = parse_case_header("$opt in");
        assert_eq!(word, "$opt");
        assert!(has_in);

        let (word2, has_in2) = parse_case_header("$val");
        assert_eq!(word2, "$val");
        assert!(!has_in2);
    }

    // ---- ScriptEngine default ----

    #[test]
    fn test_script_engine_default() {
        let _engine = ScriptEngine::default();
        // Should not panic
    }
}

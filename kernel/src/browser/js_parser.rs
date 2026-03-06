//! JavaScript Parser
//!
//! Parses a token stream from the lexer into an abstract syntax tree (AST).
//! Uses arena allocation (Vec + index) for AST nodes, Pratt parsing for
//! expressions (precedence climbing), and recursive descent for statements.

#![allow(dead_code)]

use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

use super::js_lexer::{JsLexer, JsNumber, JsToken};

// ---------------------------------------------------------------------------
// AST node types
// ---------------------------------------------------------------------------

/// Arena index for AST nodes
pub type AstNodeId = usize;

/// Sentinel value for "no node"
pub const AST_NONE: AstNodeId = usize::MAX;

/// Binary operator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    StrictEq,
    NotEq,
    StrictNotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
    Instanceof,
    In,
}

/// Unary operator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    Typeof,
    Void,
    Delete,
    BitNot,
}

/// AST node
#[derive(Debug, Clone)]
pub enum AstNode {
    /// Program root: list of top-level statements
    Program(Vec<AstNodeId>),

    /// Variable declaration: (name, optional initializer)
    VarDecl {
        name: String,
        init: Option<AstNodeId>,
        kind: VarKind,
    },

    /// Function declaration
    FuncDecl {
        name: String,
        params: Vec<String>,
        body: AstNodeId,
    },

    /// Return statement
    Return(Option<AstNodeId>),

    /// If statement
    If {
        condition: AstNodeId,
        then_branch: AstNodeId,
        else_branch: Option<AstNodeId>,
    },

    /// While loop
    While {
        condition: AstNodeId,
        body: AstNodeId,
    },

    /// For loop
    For {
        init: Option<AstNodeId>,
        condition: Option<AstNodeId>,
        update: Option<AstNodeId>,
        body: AstNodeId,
    },

    /// Block statement { ... }
    Block(Vec<AstNodeId>),

    /// Expression statement
    ExprStatement(AstNodeId),

    /// Binary expression
    BinaryExpr {
        op: BinOp,
        left: AstNodeId,
        right: AstNodeId,
    },

    /// Unary expression
    UnaryExpr { op: UnaryOp, operand: AstNodeId },

    /// Assignment expression
    AssignExpr { target: AstNodeId, value: AstNodeId },

    /// Compound assignment (+=, -=, etc.)
    CompoundAssign {
        op: BinOp,
        target: AstNodeId,
        value: AstNodeId,
    },

    /// Function call
    CallExpr {
        callee: AstNodeId,
        args: Vec<AstNodeId>,
    },

    /// Member access (obj.prop)
    MemberExpr { object: AstNodeId, property: String },

    /// Index access (obj[expr])
    IndexExpr { object: AstNodeId, index: AstNodeId },

    /// Object literal { key: value, ... }
    ObjectLiteral(Vec<(String, AstNodeId)>),

    /// Array literal [expr, ...]
    ArrayLiteral(Vec<AstNodeId>),

    /// String literal
    StringLit(String),

    /// Number literal
    NumberLit(JsNumber),

    /// Boolean literal
    BoolLit(bool),

    /// Null literal
    NullLit,

    /// Undefined
    UndefinedLit,

    /// Identifier reference
    Identifier(String),

    /// `this` keyword
    This,

    /// Function expression
    FuncExpr {
        params: Vec<String>,
        body: AstNodeId,
    },

    /// Arrow function (params) => body
    ArrowFunc {
        params: Vec<String>,
        body: AstNodeId,
    },

    /// new Foo(args)
    NewExpr {
        callee: AstNodeId,
        args: Vec<AstNodeId>,
    },

    /// typeof expr
    TypeofExpr(AstNodeId),

    /// throw expr
    Throw(AstNodeId),

    /// try/catch/finally
    TryCatch {
        try_body: AstNodeId,
        catch_param: Option<String>,
        catch_body: Option<AstNodeId>,
        finally_body: Option<AstNodeId>,
    },

    /// break
    Break,

    /// continue
    Continue,

    /// Empty statement
    Empty,
}

/// Variable declaration kind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

// ---------------------------------------------------------------------------
// AST Arena
// ---------------------------------------------------------------------------

/// Arena allocator for AST nodes
pub struct AstArena {
    nodes: Vec<AstNode>,
}

impl Default for AstArena {
    fn default() -> Self {
        Self::new()
    }
}

impl AstArena {
    pub fn new() -> Self {
        Self {
            nodes: Vec::with_capacity(256),
        }
    }

    /// Allocate a new AST node, returning its ID
    pub fn alloc(&mut self, node: AstNode) -> AstNodeId {
        let id = self.nodes.len();
        self.nodes.push(node);
        id
    }

    /// Get a reference to a node by ID
    pub fn get(&self, id: AstNodeId) -> Option<&AstNode> {
        self.nodes.get(id)
    }

    /// Get a mutable reference to a node
    pub fn get_mut(&mut self, id: AstNodeId) -> Option<&mut AstNode> {
        self.nodes.get_mut(id)
    }

    /// Number of allocated nodes
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether arena is empty
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// JavaScript parser
pub struct JsParser {
    /// Token stream
    tokens: Vec<JsToken>,
    /// Current position in token stream
    pos: usize,
    /// AST arena
    pub arena: AstArena,
    /// Parse errors
    pub errors: Vec<String>,
}

impl JsParser {
    /// Create a parser from source code
    pub fn from_source(source: &str) -> Self {
        let tokens = JsLexer::new(source).tokenize_all();
        Self {
            tokens,
            pos: 0,
            arena: AstArena::new(),
            errors: Vec::new(),
        }
    }

    /// Create a parser from pre-lexed tokens
    pub fn from_tokens(tokens: Vec<JsToken>) -> Self {
        Self {
            tokens,
            pos: 0,
            arena: AstArena::new(),
            errors: Vec::new(),
        }
    }

    /// Parse the entire program
    pub fn parse(&mut self) -> AstNodeId {
        let mut stmts = Vec::new();
        while !self.at_end() {
            if self.check(&JsToken::Semicolon) {
                self.advance_pos();
                continue;
            }
            let stmt = self.parse_statement();
            stmts.push(stmt);
        }
        self.arena.alloc(AstNode::Program(stmts))
    }

    // -- Token helpers --

    fn current(&self) -> &JsToken {
        self.tokens.get(self.pos).unwrap_or(&JsToken::Eof)
    }

    fn peek_token(&self) -> &JsToken {
        self.tokens.get(self.pos + 1).unwrap_or(&JsToken::Eof)
    }

    fn at_end(&self) -> bool {
        matches!(self.current(), JsToken::Eof)
    }

    fn check(&self, expected: &JsToken) -> bool {
        core::mem::discriminant(self.current()) == core::mem::discriminant(expected)
    }

    fn advance_pos(&mut self) -> JsToken {
        let tok = self.current().clone();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &JsToken) -> bool {
        if self.check(expected) {
            self.advance_pos();
            true
        } else {
            self.errors.push(alloc::format!(
                "Expected {:?}, got {:?}",
                expected,
                self.current()
            ));
            false
        }
    }

    fn eat_semicolon(&mut self) {
        if self.check(&JsToken::Semicolon) {
            self.advance_pos();
        }
    }

    // -- Statement parsing (recursive descent) --

    fn parse_statement(&mut self) -> AstNodeId {
        match self.current() {
            JsToken::Let => self.parse_var_decl(VarKind::Let),
            JsToken::Const => self.parse_var_decl(VarKind::Const),
            JsToken::Var => self.parse_var_decl(VarKind::Var),
            JsToken::Function => self.parse_function_decl(),
            JsToken::Return => self.parse_return(),
            JsToken::If => self.parse_if(),
            JsToken::While => self.parse_while(),
            JsToken::For => self.parse_for(),
            JsToken::OpenBrace => self.parse_block(),
            JsToken::Throw => self.parse_throw(),
            JsToken::Try => self.parse_try(),
            JsToken::Break => {
                self.advance_pos();
                self.eat_semicolon();
                self.arena.alloc(AstNode::Break)
            }
            JsToken::Continue => {
                self.advance_pos();
                self.eat_semicolon();
                self.arena.alloc(AstNode::Continue)
            }
            _ => self.parse_expression_statement(),
        }
    }

    fn parse_var_decl(&mut self, kind: VarKind) -> AstNodeId {
        self.advance_pos(); // let/const/var
        let name = match self.advance_pos() {
            JsToken::Identifier(n) => n,
            _ => {
                self.errors
                    .push("Expected identifier in variable declaration".to_string());
                String::from("_error")
            }
        };
        let init = if self.check(&JsToken::Assign) {
            self.advance_pos();
            Some(self.parse_expression(0))
        } else {
            None
        };
        self.eat_semicolon();
        self.arena.alloc(AstNode::VarDecl { name, init, kind })
    }

    fn parse_function_decl(&mut self) -> AstNodeId {
        self.advance_pos(); // function
        let name = match self.advance_pos() {
            JsToken::Identifier(n) => n,
            _ => {
                self.errors.push("Expected function name".to_string());
                String::from("_error")
            }
        };
        let params = self.parse_param_list();
        let body = self.parse_block();
        self.arena.alloc(AstNode::FuncDecl { name, params, body })
    }

    fn parse_param_list(&mut self) -> Vec<String> {
        let mut params = Vec::new();
        self.expect(&JsToken::OpenParen);
        while !self.check(&JsToken::CloseParen) && !self.at_end() {
            if let JsToken::Identifier(name) = self.advance_pos() {
                params.push(name);
            }
            if self.check(&JsToken::Comma) {
                self.advance_pos();
            }
        }
        self.expect(&JsToken::CloseParen);
        params
    }

    fn parse_return(&mut self) -> AstNodeId {
        self.advance_pos(); // return
        let value =
            if self.check(&JsToken::Semicolon) || self.check(&JsToken::CloseBrace) || self.at_end()
            {
                None
            } else {
                Some(self.parse_expression(0))
            };
        self.eat_semicolon();
        self.arena.alloc(AstNode::Return(value))
    }

    fn parse_if(&mut self) -> AstNodeId {
        self.advance_pos(); // if
        self.expect(&JsToken::OpenParen);
        let condition = self.parse_expression(0);
        self.expect(&JsToken::CloseParen);
        let then_branch = self.parse_statement();
        let else_branch = if self.check(&JsToken::Else) {
            self.advance_pos();
            Some(self.parse_statement())
        } else {
            None
        };
        self.arena.alloc(AstNode::If {
            condition,
            then_branch,
            else_branch,
        })
    }

    fn parse_while(&mut self) -> AstNodeId {
        self.advance_pos(); // while
        self.expect(&JsToken::OpenParen);
        let condition = self.parse_expression(0);
        self.expect(&JsToken::CloseParen);
        let body = self.parse_statement();
        self.arena.alloc(AstNode::While { condition, body })
    }

    fn parse_for(&mut self) -> AstNodeId {
        self.advance_pos(); // for
        self.expect(&JsToken::OpenParen);
        let init = if self.check(&JsToken::Semicolon) {
            None
        } else if self.check(&JsToken::Let)
            || self.check(&JsToken::Var)
            || self.check(&JsToken::Const)
        {
            let kind = match self.current() {
                JsToken::Let => VarKind::Let,
                JsToken::Const => VarKind::Const,
                _ => VarKind::Var,
            };
            Some(self.parse_var_decl(kind))
        } else {
            let expr = self.parse_expression(0);
            Some(self.arena.alloc(AstNode::ExprStatement(expr)))
        };
        // The var_decl parser may have eaten the semicolon
        if self.check(&JsToken::Semicolon) {
            self.advance_pos();
        }
        let condition = if self.check(&JsToken::Semicolon) {
            None
        } else {
            Some(self.parse_expression(0))
        };
        self.expect(&JsToken::Semicolon);
        let update = if self.check(&JsToken::CloseParen) {
            None
        } else {
            Some(self.parse_assignment_expression())
        };
        self.expect(&JsToken::CloseParen);
        let body = self.parse_statement();
        self.arena.alloc(AstNode::For {
            init,
            condition,
            update,
            body,
        })
    }

    fn parse_block(&mut self) -> AstNodeId {
        self.expect(&JsToken::OpenBrace);
        let mut stmts = Vec::new();
        while !self.check(&JsToken::CloseBrace) && !self.at_end() {
            if self.check(&JsToken::Semicolon) {
                self.advance_pos();
                continue;
            }
            stmts.push(self.parse_statement());
        }
        self.expect(&JsToken::CloseBrace);
        self.arena.alloc(AstNode::Block(stmts))
    }

    fn parse_throw(&mut self) -> AstNodeId {
        self.advance_pos(); // throw
        let expr = self.parse_expression(0);
        self.eat_semicolon();
        self.arena.alloc(AstNode::Throw(expr))
    }

    fn parse_try(&mut self) -> AstNodeId {
        self.advance_pos(); // try
        let try_body = self.parse_block();

        let (catch_param, catch_body) = if self.check(&JsToken::Catch) {
            self.advance_pos();
            let param = if self.check(&JsToken::OpenParen) {
                self.advance_pos();
                let name = match self.advance_pos() {
                    JsToken::Identifier(n) => Some(n),
                    _ => None,
                };
                self.expect(&JsToken::CloseParen);
                name
            } else {
                None
            };
            let body = self.parse_block();
            (param, Some(body))
        } else {
            (None, None)
        };

        let finally_body = if self.check(&JsToken::Finally) {
            self.advance_pos();
            Some(self.parse_block())
        } else {
            None
        };

        self.arena.alloc(AstNode::TryCatch {
            try_body,
            catch_param,
            catch_body,
            finally_body,
        })
    }

    /// Parse an expression that may include assignment (used in for-loop
    /// update, etc.)
    fn parse_assignment_expression(&mut self) -> AstNodeId {
        let expr = self.parse_expression(0);
        let node = match self.current() {
            JsToken::Assign => {
                self.advance_pos();
                let value = self.parse_expression(0);
                AstNode::AssignExpr {
                    target: expr,
                    value,
                }
            }
            JsToken::PlusAssign => {
                self.advance_pos();
                let value = self.parse_expression(0);
                AstNode::CompoundAssign {
                    op: BinOp::Add,
                    target: expr,
                    value,
                }
            }
            JsToken::MinusAssign => {
                self.advance_pos();
                let value = self.parse_expression(0);
                AstNode::CompoundAssign {
                    op: BinOp::Sub,
                    target: expr,
                    value,
                }
            }
            JsToken::StarAssign => {
                self.advance_pos();
                let value = self.parse_expression(0);
                AstNode::CompoundAssign {
                    op: BinOp::Mul,
                    target: expr,
                    value,
                }
            }
            JsToken::SlashAssign => {
                self.advance_pos();
                let value = self.parse_expression(0);
                AstNode::CompoundAssign {
                    op: BinOp::Div,
                    target: expr,
                    value,
                }
            }
            _ => return expr,
        };
        self.arena.alloc(node)
    }

    fn parse_expression_statement(&mut self) -> AstNodeId {
        let expr = self.parse_expression(0);

        // Check for assignment
        let node = match self.current() {
            JsToken::Assign => {
                self.advance_pos();
                let value = self.parse_expression(0);
                AstNode::AssignExpr {
                    target: expr,
                    value,
                }
            }
            JsToken::PlusAssign => {
                self.advance_pos();
                let value = self.parse_expression(0);
                AstNode::CompoundAssign {
                    op: BinOp::Add,
                    target: expr,
                    value,
                }
            }
            JsToken::MinusAssign => {
                self.advance_pos();
                let value = self.parse_expression(0);
                AstNode::CompoundAssign {
                    op: BinOp::Sub,
                    target: expr,
                    value,
                }
            }
            JsToken::StarAssign => {
                self.advance_pos();
                let value = self.parse_expression(0);
                AstNode::CompoundAssign {
                    op: BinOp::Mul,
                    target: expr,
                    value,
                }
            }
            JsToken::SlashAssign => {
                self.advance_pos();
                let value = self.parse_expression(0);
                AstNode::CompoundAssign {
                    op: BinOp::Div,
                    target: expr,
                    value,
                }
            }
            _ => AstNode::ExprStatement(expr),
        };

        let id = self.arena.alloc(node);
        self.eat_semicolon();
        id
    }

    // -- Expression parsing (Pratt / precedence climbing) --

    fn parse_expression(&mut self, min_prec: u8) -> AstNodeId {
        let mut left = self.parse_unary();

        while let Some((op, prec)) = self.current_binop() {
            if prec < min_prec {
                break;
            }
            self.advance_pos();
            let right = self.parse_expression(prec + 1);
            left = self.arena.alloc(AstNode::BinaryExpr { op, left, right });
        }

        left
    }

    fn current_binop(&self) -> Option<(BinOp, u8)> {
        match self.current() {
            JsToken::Or => Some((BinOp::Or, 3)),
            JsToken::And => Some((BinOp::And, 4)),
            JsToken::BitOr => Some((BinOp::BitOr, 5)),
            JsToken::BitXor => Some((BinOp::BitXor, 6)),
            JsToken::BitAnd => Some((BinOp::BitAnd, 7)),
            JsToken::EqEq => Some((BinOp::Eq, 8)),
            JsToken::EqEqEq => Some((BinOp::StrictEq, 8)),
            JsToken::NotEq => Some((BinOp::NotEq, 8)),
            JsToken::NotEqEq => Some((BinOp::StrictNotEq, 8)),
            JsToken::Lt => Some((BinOp::Lt, 9)),
            JsToken::Gt => Some((BinOp::Gt, 9)),
            JsToken::LtEq => Some((BinOp::LtEq, 9)),
            JsToken::GtEq => Some((BinOp::GtEq, 9)),
            JsToken::Instanceof => Some((BinOp::Instanceof, 9)),
            JsToken::In => Some((BinOp::In, 9)),
            JsToken::ShiftLeft => Some((BinOp::ShiftLeft, 10)),
            JsToken::ShiftRight => Some((BinOp::ShiftRight, 10)),
            JsToken::Plus => Some((BinOp::Add, 11)),
            JsToken::Minus => Some((BinOp::Sub, 11)),
            JsToken::Star => Some((BinOp::Mul, 12)),
            JsToken::Slash => Some((BinOp::Div, 12)),
            JsToken::Percent => Some((BinOp::Mod, 12)),
            _ => None,
        }
    }

    fn parse_unary(&mut self) -> AstNodeId {
        match self.current() {
            JsToken::Minus => {
                self.advance_pos();
                let operand = self.parse_unary();
                self.arena.alloc(AstNode::UnaryExpr {
                    op: UnaryOp::Neg,
                    operand,
                })
            }
            JsToken::Not => {
                self.advance_pos();
                let operand = self.parse_unary();
                self.arena.alloc(AstNode::UnaryExpr {
                    op: UnaryOp::Not,
                    operand,
                })
            }
            JsToken::Typeof => {
                self.advance_pos();
                let operand = self.parse_unary();
                self.arena.alloc(AstNode::TypeofExpr(operand))
            }
            JsToken::Void => {
                self.advance_pos();
                let operand = self.parse_unary();
                self.arena.alloc(AstNode::UnaryExpr {
                    op: UnaryOp::Void,
                    operand,
                })
            }
            JsToken::Delete => {
                self.advance_pos();
                let operand = self.parse_unary();
                self.arena.alloc(AstNode::UnaryExpr {
                    op: UnaryOp::Delete,
                    operand,
                })
            }
            JsToken::New => self.parse_new_expr(),
            _ => self.parse_call_member(),
        }
    }

    fn parse_new_expr(&mut self) -> AstNodeId {
        self.advance_pos(); // new
        let callee = self.parse_primary();
        let args = if self.check(&JsToken::OpenParen) {
            self.parse_arguments()
        } else {
            Vec::new()
        };
        self.arena.alloc(AstNode::NewExpr { callee, args })
    }

    fn parse_call_member(&mut self) -> AstNodeId {
        let mut node = self.parse_primary();

        loop {
            match self.current() {
                JsToken::Dot => {
                    self.advance_pos();
                    let prop = match self.advance_pos() {
                        JsToken::Identifier(n) => n,
                        _ => {
                            self.errors
                                .push("Expected property name after '.'".to_string());
                            String::from("_error")
                        }
                    };
                    node = self.arena.alloc(AstNode::MemberExpr {
                        object: node,
                        property: prop,
                    });
                }
                JsToken::OpenBracket => {
                    self.advance_pos();
                    let index = self.parse_expression(0);
                    self.expect(&JsToken::CloseBracket);
                    node = self.arena.alloc(AstNode::IndexExpr {
                        object: node,
                        index,
                    });
                }
                JsToken::OpenParen => {
                    let args = self.parse_arguments();
                    node = self.arena.alloc(AstNode::CallExpr { callee: node, args });
                }
                _ => break,
            }
        }

        node
    }

    fn parse_arguments(&mut self) -> Vec<AstNodeId> {
        let mut args = Vec::new();
        self.expect(&JsToken::OpenParen);
        while !self.check(&JsToken::CloseParen) && !self.at_end() {
            args.push(self.parse_expression(0));
            if self.check(&JsToken::Comma) {
                self.advance_pos();
            }
        }
        self.expect(&JsToken::CloseParen);
        args
    }

    fn parse_primary(&mut self) -> AstNodeId {
        let tok = self.current().clone();
        match tok {
            JsToken::Number(n) => {
                self.advance_pos();
                self.arena.alloc(AstNode::NumberLit(n))
            }
            JsToken::StringLiteral(s) => {
                self.advance_pos();
                self.arena.alloc(AstNode::StringLit(s))
            }
            JsToken::True => {
                self.advance_pos();
                self.arena.alloc(AstNode::BoolLit(true))
            }
            JsToken::False => {
                self.advance_pos();
                self.arena.alloc(AstNode::BoolLit(false))
            }
            JsToken::Null => {
                self.advance_pos();
                self.arena.alloc(AstNode::NullLit)
            }
            JsToken::Undefined => {
                self.advance_pos();
                self.arena.alloc(AstNode::UndefinedLit)
            }
            JsToken::This => {
                self.advance_pos();
                self.arena.alloc(AstNode::This)
            }
            JsToken::Identifier(ref name) => {
                let name = name.clone();
                self.advance_pos();

                // Check for arrow function: (ident) => ...
                // Simple case: single-param arrow x => body
                if self.check(&JsToken::Arrow) {
                    self.advance_pos();
                    let body = if self.check(&JsToken::OpenBrace) {
                        self.parse_block()
                    } else {
                        let expr = self.parse_expression(0);
                        self.arena.alloc(AstNode::Return(Some(expr)))
                    };
                    return self.arena.alloc(AstNode::ArrowFunc {
                        params: vec![name],
                        body,
                    });
                }

                self.arena.alloc(AstNode::Identifier(name))
            }
            JsToken::OpenParen => {
                self.advance_pos();

                // Check for arrow function with params
                // Heuristic: if we see identifiers separated by commas,
                // followed by ) =>, it's an arrow function
                if self.is_arrow_params() {
                    let params = self.parse_arrow_params();
                    self.expect(&JsToken::Arrow);
                    let body = if self.check(&JsToken::OpenBrace) {
                        self.parse_block()
                    } else {
                        let expr = self.parse_expression(0);
                        self.arena.alloc(AstNode::Return(Some(expr)))
                    };
                    return self.arena.alloc(AstNode::ArrowFunc { params, body });
                }

                let expr = self.parse_expression(0);
                self.expect(&JsToken::CloseParen);
                expr
            }
            JsToken::OpenBrace => self.parse_object_literal(),
            JsToken::OpenBracket => self.parse_array_literal(),
            JsToken::Function => {
                self.advance_pos();
                // Optional name for function expression
                if let JsToken::Identifier(_) = self.current() {
                    self.advance_pos();
                };
                let params = self.parse_param_list();
                let body = self.parse_block();
                self.arena.alloc(AstNode::FuncExpr { params, body })
            }
            _ => {
                self.errors
                    .push(alloc::format!("Unexpected token in expression: {:?}", tok));
                self.advance_pos();
                self.arena.alloc(AstNode::Empty)
            }
        }
    }

    fn parse_object_literal(&mut self) -> AstNodeId {
        self.advance_pos(); // {
        let mut props = Vec::new();
        while !self.check(&JsToken::CloseBrace) && !self.at_end() {
            let key = match self.advance_pos() {
                JsToken::Identifier(k) => k,
                JsToken::StringLiteral(k) => k,
                _ => {
                    self.errors.push("Expected property key".to_string());
                    String::from("_error")
                }
            };
            self.expect(&JsToken::Colon);
            let value = self.parse_expression(0);
            props.push((key, value));
            if self.check(&JsToken::Comma) {
                self.advance_pos();
            }
        }
        self.expect(&JsToken::CloseBrace);
        self.arena.alloc(AstNode::ObjectLiteral(props))
    }

    fn parse_array_literal(&mut self) -> AstNodeId {
        self.advance_pos(); // [
        let mut elements = Vec::new();
        while !self.check(&JsToken::CloseBracket) && !self.at_end() {
            elements.push(self.parse_expression(0));
            if self.check(&JsToken::Comma) {
                self.advance_pos();
            }
        }
        self.expect(&JsToken::CloseBracket);
        self.arena.alloc(AstNode::ArrayLiteral(elements))
    }

    /// Heuristic: check if current position starts arrow function params
    fn is_arrow_params(&self) -> bool {
        // Look ahead for ) =>
        let mut depth = 0;
        let mut i = self.pos;
        while i < self.tokens.len() {
            match &self.tokens[i] {
                JsToken::CloseParen if depth == 0 => {
                    // Check for => after )
                    return matches!(self.tokens.get(i + 1), Some(JsToken::Arrow));
                }
                JsToken::OpenParen => depth += 1,
                JsToken::CloseParen => depth -= 1,
                JsToken::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// Parse arrow function parameters (already past opening paren)
    fn parse_arrow_params(&mut self) -> Vec<String> {
        let mut params = Vec::new();
        while !self.check(&JsToken::CloseParen) && !self.at_end() {
            if let JsToken::Identifier(name) = self.advance_pos() {
                params.push(name);
            }
            if self.check(&JsToken::Comma) {
                self.advance_pos();
            }
        }
        self.expect(&JsToken::CloseParen);
        params
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> JsParser {
        let mut parser = JsParser::from_source(src);
        parser.parse();
        parser
    }

    #[test]
    fn test_empty_program() {
        let p = parse("");
        assert_eq!(p.arena.len(), 1);
        matches!(p.arena.get(0), Some(AstNode::Program(stmts)) if stmts.is_empty());
    }

    #[test]
    fn test_number_literal() {
        let p = parse("42;");
        assert!(p.errors.is_empty());
        assert!(p.arena.len() >= 2); // Program + NumberLit + ExprStmt
    }

    #[test]
    fn test_string_literal() {
        let p = parse("\"hello\";");
        assert!(p.errors.is_empty());
    }

    #[test]
    fn test_var_decl_let() {
        let p = parse("let x = 10;");
        assert!(p.errors.is_empty());
        // Find VarDecl node
        let has_var = (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::VarDecl { name, kind: VarKind::Let, .. }) if name == "x"));
        assert!(has_var);
    }

    #[test]
    fn test_var_decl_const() {
        let p = parse("const y = 'hello';");
        assert!(p.errors.is_empty());
        let has_const = (0..p.arena.len()).any(|i| {
            matches!(
                p.arena.get(i),
                Some(AstNode::VarDecl {
                    kind: VarKind::Const,
                    ..
                })
            )
        });
        assert!(has_const);
    }

    #[test]
    fn test_function_decl() {
        let p = parse("function add(a, b) { return a + b; }");
        assert!(p.errors.is_empty());
        let has_func = (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::FuncDecl { name, params, .. }) if name == "add" && params.len() == 2));
        assert!(has_func);
    }

    #[test]
    fn test_if_else() {
        let p = parse("if (x > 0) { y = 1; } else { y = 2; }");
        assert!(p.errors.is_empty());
        let has_if = (0..p.arena.len()).any(|i| {
            matches!(
                p.arena.get(i),
                Some(AstNode::If {
                    else_branch: Some(_),
                    ..
                })
            )
        });
        assert!(has_if);
    }

    #[test]
    fn test_while_loop() {
        let p = parse("while (i < 10) { i = i + 1; }");
        assert!(p.errors.is_empty());
        let has_while =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::While { .. })));
        assert!(has_while);
    }

    #[test]
    fn test_for_loop() {
        let p = parse("for (let i = 0; i < 10; i = i + 1) { x = i; }");
        assert!(p.errors.is_empty());
        let has_for =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::For { .. })));
        assert!(has_for);
    }

    #[test]
    fn test_binary_expr() {
        let p = parse("1 + 2 * 3;");
        assert!(p.errors.is_empty());
        // Should have BinaryExpr nodes
        let bin_count = (0..p.arena.len())
            .filter(|&i| matches!(p.arena.get(i), Some(AstNode::BinaryExpr { .. })))
            .count();
        assert_eq!(bin_count, 2);
    }

    #[test]
    fn test_precedence() {
        let p = parse("1 + 2 * 3;");
        // The Mul should be nested inside the Add
        // Find the Add node
        for i in 0..p.arena.len() {
            if let Some(AstNode::BinaryExpr {
                op: BinOp::Add,
                right,
                ..
            }) = p.arena.get(i)
            {
                // right should be a Mul
                assert!(matches!(
                    p.arena.get(*right),
                    Some(AstNode::BinaryExpr { op: BinOp::Mul, .. })
                ));
            }
        }
    }

    #[test]
    fn test_unary_neg() {
        let p = parse("-x;");
        assert!(p.errors.is_empty());
        let has_unary = (0..p.arena.len()).any(|i| {
            matches!(
                p.arena.get(i),
                Some(AstNode::UnaryExpr {
                    op: UnaryOp::Neg,
                    ..
                })
            )
        });
        assert!(has_unary);
    }

    #[test]
    fn test_call_expr() {
        let p = parse("foo(1, 2, 3);");
        assert!(p.errors.is_empty());
        let has_call = (0..p.arena.len()).any(
            |i| matches!(p.arena.get(i), Some(AstNode::CallExpr { args, .. }) if args.len() == 3),
        );
        assert!(has_call);
    }

    #[test]
    fn test_member_expr() {
        let p = parse("obj.prop;");
        assert!(p.errors.is_empty());
        let has_member = (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::MemberExpr { property, .. }) if property == "prop"));
        assert!(has_member);
    }

    #[test]
    fn test_index_expr() {
        let p = parse("arr[0];");
        assert!(p.errors.is_empty());
        let has_index =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::IndexExpr { .. })));
        assert!(has_index);
    }

    #[test]
    fn test_object_literal() {
        let p = parse("let o = {a: 1, b: 2};");
        assert!(p.errors.is_empty());
        let has_obj = (0..p.arena.len()).any(
            |i| matches!(p.arena.get(i), Some(AstNode::ObjectLiteral(props)) if props.len() == 2),
        );
        assert!(has_obj);
    }

    #[test]
    fn test_array_literal() {
        let p = parse("let a = [1, 2, 3];");
        assert!(p.errors.is_empty());
        let has_arr = (0..p.arena.len()).any(
            |i| matches!(p.arena.get(i), Some(AstNode::ArrayLiteral(elems)) if elems.len() == 3),
        );
        assert!(has_arr);
    }

    #[test]
    fn test_assignment() {
        let p = parse("x = 5;");
        assert!(p.errors.is_empty());
        let has_assign =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::AssignExpr { .. })));
        assert!(has_assign);
    }

    #[test]
    fn test_compound_assign() {
        let p = parse("x += 1;");
        assert!(p.errors.is_empty());
        let has_compound = (0..p.arena.len()).any(|i| {
            matches!(
                p.arena.get(i),
                Some(AstNode::CompoundAssign { op: BinOp::Add, .. })
            )
        });
        assert!(has_compound);
    }

    #[test]
    fn test_try_catch() {
        let p = parse("try { x(); } catch (e) { log(e); }");
        assert!(p.errors.is_empty());
        let has_try = (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::TryCatch { catch_param: Some(ref name), .. }) if name == "e"));
        assert!(has_try);
    }

    #[test]
    fn test_try_finally() {
        let p = parse("try { x(); } finally { cleanup(); }");
        assert!(p.errors.is_empty());
        let has_finally = (0..p.arena.len()).any(|i| {
            matches!(
                p.arena.get(i),
                Some(AstNode::TryCatch {
                    finally_body: Some(_),
                    ..
                })
            )
        });
        assert!(has_finally);
    }

    #[test]
    fn test_throw() {
        let p = parse("throw new Error();");
        assert!(p.errors.is_empty());
        let has_throw =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::Throw(_))));
        assert!(has_throw);
    }

    #[test]
    fn test_new_expr() {
        let p = parse("new Foo(1);");
        assert!(p.errors.is_empty());
        let has_new =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::NewExpr { .. })));
        assert!(has_new);
    }

    #[test]
    fn test_arrow_function() {
        let p = parse("let f = x => x + 1;");
        assert!(p.errors.is_empty());
        let has_arrow = (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::ArrowFunc { params, .. }) if params.len() == 1));
        assert!(has_arrow);
    }

    #[test]
    fn test_function_expr() {
        let p = parse("let f = function(a) { return a; };");
        assert!(p.errors.is_empty());
        let has_func_expr =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::FuncExpr { .. })));
        assert!(has_func_expr);
    }

    #[test]
    fn test_break_continue() {
        let p = parse("while (true) { break; continue; }");
        assert!(p.errors.is_empty());
        let has_break = (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::Break)));
        let has_continue =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::Continue)));
        assert!(has_break);
        assert!(has_continue);
    }

    #[test]
    fn test_chained_member() {
        let p = parse("a.b.c;");
        assert!(p.errors.is_empty());
        let member_count = (0..p.arena.len())
            .filter(|&i| matches!(p.arena.get(i), Some(AstNode::MemberExpr { .. })))
            .count();
        assert_eq!(member_count, 2);
    }

    #[test]
    fn test_nested_calls() {
        let p = parse("a(b(c()));");
        assert!(p.errors.is_empty());
        let call_count = (0..p.arena.len())
            .filter(|&i| matches!(p.arena.get(i), Some(AstNode::CallExpr { .. })))
            .count();
        assert_eq!(call_count, 3);
    }

    #[test]
    fn test_typeof() {
        let p = parse("typeof x;");
        assert!(p.errors.is_empty());
        let has_typeof =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::TypeofExpr(_))));
        assert!(has_typeof);
    }

    #[test]
    fn test_this() {
        let p = parse("this.x;");
        assert!(p.errors.is_empty());
        let has_this = (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::This)));
        assert!(has_this);
    }

    #[test]
    fn test_boolean_literals() {
        let p = parse("true; false;");
        assert!(p.errors.is_empty());
        let has_true =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::BoolLit(true))));
        let has_false =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::BoolLit(false))));
        assert!(has_true);
        assert!(has_false);
    }

    #[test]
    fn test_null_undefined() {
        let p = parse("null; undefined;");
        assert!(p.errors.is_empty());
    }

    #[test]
    fn test_logical_and_or() {
        let p = parse("a && b || c;");
        assert!(p.errors.is_empty());
    }

    #[test]
    fn test_comparison_ops() {
        let p = parse("a < b; c >= d; e === f; g !== h;");
        assert!(p.errors.is_empty());
    }

    #[test]
    fn test_return_no_value() {
        let p = parse("function f() { return; }");
        assert!(p.errors.is_empty());
        let has_return =
            (0..p.arena.len()).any(|i| matches!(p.arena.get(i), Some(AstNode::Return(None))));
        assert!(has_return);
    }

    #[test]
    fn test_if_no_else() {
        let p = parse("if (x) { y; }");
        assert!(p.errors.is_empty());
        let has_if = (0..p.arena.len()).any(|i| {
            matches!(
                p.arena.get(i),
                Some(AstNode::If {
                    else_branch: None,
                    ..
                })
            )
        });
        assert!(has_if);
    }

    #[test]
    fn test_empty_block() {
        let p = parse("{}");
        assert!(p.errors.is_empty());
        let has_block = (0..p.arena.len())
            .any(|i| matches!(p.arena.get(i), Some(AstNode::Block(stmts)) if stmts.is_empty()));
        assert!(has_block);
    }

    #[test]
    fn test_method_call() {
        let p = parse("console.log('hello');");
        assert!(p.errors.is_empty());
    }

    #[test]
    fn test_ast_arena_basic() {
        let mut arena = AstArena::new();
        let id = arena.alloc(AstNode::NullLit);
        assert_eq!(id, 0);
        assert_eq!(arena.len(), 1);
        assert!(!arena.is_empty());
    }

    #[test]
    fn test_not_operator() {
        let p = parse("!x;");
        assert!(p.errors.is_empty());
        let has_not = (0..p.arena.len()).any(|i| {
            matches!(
                p.arena.get(i),
                Some(AstNode::UnaryExpr {
                    op: UnaryOp::Not,
                    ..
                })
            )
        });
        assert!(has_not);
    }

    #[test]
    fn test_multiple_statements() {
        let p = parse("let a = 1; let b = 2; let c = a + b;");
        assert!(p.errors.is_empty());
        let var_count = (0..p.arena.len())
            .filter(|&i| matches!(p.arena.get(i), Some(AstNode::VarDecl { .. })))
            .count();
        assert_eq!(var_count, 3);
    }
}

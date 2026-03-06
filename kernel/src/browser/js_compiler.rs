//! JavaScript Bytecode Compiler
//!
//! Compiles AST nodes into bytecode for the JS virtual machine. Uses a
//! stack-based instruction set with ~40 opcodes. Functions are compiled
//! into FunctionTemplates with their own constant pools.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use super::{
    js_lexer::JsNumber,
    js_parser::{AstArena, AstNode, AstNodeId, BinOp, UnaryOp},
};

// ---------------------------------------------------------------------------
// Opcodes
// ---------------------------------------------------------------------------

/// Bytecode opcodes (single-byte encoding)
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    /// Push constant from pool onto stack
    LoadConst = 0,
    /// Push local variable onto stack
    LoadLocal = 1,
    /// Pop stack into local variable
    StoreLocal = 2,
    /// Push global variable onto stack
    LoadGlobal = 3,
    /// Pop stack into global variable
    StoreGlobal = 4,
    /// Pop object+key, push property value
    LoadProperty = 5,
    /// Pop object+key+value, set property
    StoreProperty = 6,
    /// Pop array+index, push element
    LoadIndex = 7,
    /// Pop array+index+value, set element
    StoreIndex = 8,

    // Arithmetic
    Add = 10,
    Sub = 11,
    Mul = 12,
    Div = 13,
    Mod = 14,
    Neg = 15,

    // Logic / bitwise
    Not = 20,
    BitAnd = 21,
    BitOr = 22,
    BitXor = 23,
    ShiftLeft = 24,
    ShiftRight = 25,

    // Comparison
    Eq = 30,
    StrictEq = 31,
    NotEq = 32,
    StrictNotEq = 33,
    Lt = 34,
    Gt = 35,
    LtEq = 36,
    GtEq = 37,

    // Logical short-circuit
    LogicalAnd = 38,
    LogicalOr = 39,

    // Control flow
    Jump = 40,
    JumpIfFalse = 41,
    JumpIfTrue = 42,

    // Functions
    Call = 50,
    Return = 51,
    CreateClosure = 52,

    // Object/array
    CreateObject = 60,
    CreateArray = 61,

    // Misc
    GetThis = 70,
    Typeof = 71,
    Instanceof = 72,
    In = 73,
    Throw = 74,
    EnterTry = 75,
    LeaveTry = 76,

    // Stack
    Pop = 80,
    Dup = 81,

    // Halt
    Halt = 255,
}

impl Opcode {
    /// Decode from byte
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::LoadConst),
            1 => Some(Self::LoadLocal),
            2 => Some(Self::StoreLocal),
            3 => Some(Self::LoadGlobal),
            4 => Some(Self::StoreGlobal),
            5 => Some(Self::LoadProperty),
            6 => Some(Self::StoreProperty),
            7 => Some(Self::LoadIndex),
            8 => Some(Self::StoreIndex),
            10 => Some(Self::Add),
            11 => Some(Self::Sub),
            12 => Some(Self::Mul),
            13 => Some(Self::Div),
            14 => Some(Self::Mod),
            15 => Some(Self::Neg),
            20 => Some(Self::Not),
            21 => Some(Self::BitAnd),
            22 => Some(Self::BitOr),
            23 => Some(Self::BitXor),
            24 => Some(Self::ShiftLeft),
            25 => Some(Self::ShiftRight),
            30 => Some(Self::Eq),
            31 => Some(Self::StrictEq),
            32 => Some(Self::NotEq),
            33 => Some(Self::StrictNotEq),
            34 => Some(Self::Lt),
            35 => Some(Self::Gt),
            36 => Some(Self::LtEq),
            37 => Some(Self::GtEq),
            38 => Some(Self::LogicalAnd),
            39 => Some(Self::LogicalOr),
            40 => Some(Self::Jump),
            41 => Some(Self::JumpIfFalse),
            42 => Some(Self::JumpIfTrue),
            50 => Some(Self::Call),
            51 => Some(Self::Return),
            52 => Some(Self::CreateClosure),
            60 => Some(Self::CreateObject),
            61 => Some(Self::CreateArray),
            70 => Some(Self::GetThis),
            71 => Some(Self::Typeof),
            72 => Some(Self::Instanceof),
            73 => Some(Self::In),
            74 => Some(Self::Throw),
            75 => Some(Self::EnterTry),
            76 => Some(Self::LeaveTry),
            80 => Some(Self::Pop),
            81 => Some(Self::Dup),
            255 => Some(Self::Halt),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Constants and templates
// ---------------------------------------------------------------------------

/// Constant pool entry
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Constant {
    Number(JsNumber),
    Str(String),
    Bool(bool),
    Null,
    Undefined,
    Function(FunctionTemplate),
}

/// Compiled function template
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct FunctionTemplate {
    /// Function name (empty for anonymous)
    pub name: String,
    /// Number of parameters
    pub param_count: usize,
    /// Bytecode
    pub bytecode: Vec<u8>,
    /// Constant pool
    pub constants: Vec<Constant>,
    /// Number of local variables
    pub local_count: usize,
    /// Number of upvalues (captured variables)
    pub upvalue_count: usize,
    /// Source line numbers (one per bytecode byte, for debugging)
    pub line_numbers: Vec<u32>,
}

/// Bytecode chunk (top-level compilation unit)
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct Chunk {
    pub bytecode: Vec<u8>,
    pub constants: Vec<Constant>,
    pub line_numbers: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Local variable tracking
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct Local {
    name: String,
    depth: u32,
    index: u16,
}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

/// Bytecode compiler
#[allow(dead_code)]
pub struct Compiler {
    /// Current chunk being compiled
    chunk: Chunk,
    /// Scope depth (0 = global)
    scope_depth: u32,
    /// Local variables
    locals: Vec<Local>,
    /// Next local index
    next_local: u16,
    /// Current source line
    current_line: u32,
    /// Nested function templates
    functions: Vec<FunctionTemplate>,
    /// Loop break patch points
    break_patches: Vec<Vec<usize>>,
    /// Loop continue targets
    continue_targets: Vec<usize>,
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            chunk: Chunk::default(),
            scope_depth: 0,
            locals: Vec::new(),
            next_local: 0,
            current_line: 1,
            functions: Vec::new(),
            break_patches: Vec::new(),
            continue_targets: Vec::new(),
        }
    }

    /// Compile an AST into a Chunk
    pub fn compile(&mut self, arena: &AstArena, root: AstNodeId) -> Chunk {
        self.compile_node(arena, root);
        self.emit_op(Opcode::Halt);
        core::mem::take(&mut self.chunk)
    }

    /// Compile an AST into a FunctionTemplate
    pub fn compile_function(
        &mut self,
        arena: &AstArena,
        name: &str,
        params: &[String],
        body: AstNodeId,
    ) -> FunctionTemplate {
        let prev_chunk = core::mem::take(&mut self.chunk);
        let prev_locals = core::mem::take(&mut self.locals);
        let prev_next_local = self.next_local;
        let prev_depth = self.scope_depth;

        self.scope_depth = 1;
        self.next_local = 0;
        self.locals.clear();

        // Define parameters as locals
        for param in params {
            self.define_local(param);
        }

        self.compile_node(arena, body);
        // Implicit return undefined
        let undef_idx = self.add_constant(Constant::Undefined);
        self.emit_op(Opcode::LoadConst);
        self.emit_u16(undef_idx);
        self.emit_op(Opcode::Return);

        let func = FunctionTemplate {
            name: name.to_string(),
            param_count: params.len(),
            bytecode: core::mem::take(&mut self.chunk.bytecode),
            constants: core::mem::take(&mut self.chunk.constants),
            local_count: self.next_local as usize,
            upvalue_count: 0,
            line_numbers: core::mem::take(&mut self.chunk.line_numbers),
        };

        self.chunk = prev_chunk;
        self.locals = prev_locals;
        self.next_local = prev_next_local;
        self.scope_depth = prev_depth;

        func
    }

    // -- Compilation --

    fn compile_node(&mut self, arena: &AstArena, id: AstNodeId) {
        let node = match arena.get(id) {
            Some(n) => n.clone(),
            None => return,
        };

        match node {
            AstNode::Program(stmts) => {
                for stmt in &stmts {
                    self.compile_node(arena, *stmt);
                }
            }

            AstNode::VarDecl {
                name,
                init,
                kind: _,
            } => {
                if self.scope_depth > 0 {
                    let idx = self.define_local(&name);
                    if let Some(init_id) = init {
                        self.compile_node(arena, init_id);
                        self.emit_op(Opcode::StoreLocal);
                        self.emit_u16(idx);
                    }
                } else {
                    if let Some(init_id) = init {
                        self.compile_node(arena, init_id);
                    } else {
                        let c = self.add_constant(Constant::Undefined);
                        self.emit_op(Opcode::LoadConst);
                        self.emit_u16(c);
                    }
                    let name_idx = self.add_constant(Constant::Str(name));
                    self.emit_op(Opcode::StoreGlobal);
                    self.emit_u16(name_idx);
                }
            }

            AstNode::FuncDecl { name, params, body } => {
                let func = self.compile_function(arena, &name, &params, body);
                let func_idx = self.add_constant(Constant::Function(func));
                self.emit_op(Opcode::LoadConst);
                self.emit_u16(func_idx);
                if self.scope_depth > 0 {
                    let local = self.define_local(&name);
                    self.emit_op(Opcode::StoreLocal);
                    self.emit_u16(local);
                } else {
                    let name_idx = self.add_constant(Constant::Str(name));
                    self.emit_op(Opcode::StoreGlobal);
                    self.emit_u16(name_idx);
                }
            }

            AstNode::Return(value) => {
                if let Some(val_id) = value {
                    self.compile_node(arena, val_id);
                } else {
                    let c = self.add_constant(Constant::Undefined);
                    self.emit_op(Opcode::LoadConst);
                    self.emit_u16(c);
                }
                self.emit_op(Opcode::Return);
            }

            AstNode::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.compile_node(arena, condition);
                let else_jump = self.emit_jump(Opcode::JumpIfFalse);
                self.compile_node(arena, then_branch);
                if let Some(else_id) = else_branch {
                    let end_jump = self.emit_jump(Opcode::Jump);
                    self.patch_jump(else_jump);
                    self.compile_node(arena, else_id);
                    self.patch_jump(end_jump);
                } else {
                    self.patch_jump(else_jump);
                }
            }

            AstNode::While { condition, body } => {
                let loop_start = self.chunk.bytecode.len();
                self.continue_targets.push(loop_start);
                self.break_patches.push(Vec::new());

                self.compile_node(arena, condition);
                let exit_jump = self.emit_jump(Opcode::JumpIfFalse);
                self.compile_node(arena, body);
                self.emit_loop(loop_start);
                self.patch_jump(exit_jump);

                let breaks = self.break_patches.pop().unwrap_or_default();
                for bp in breaks {
                    self.patch_jump(bp);
                }
                self.continue_targets.pop();
            }

            AstNode::For {
                init,
                condition,
                update,
                body,
            } => {
                if let Some(init_id) = init {
                    self.compile_node(arena, init_id);
                }
                let loop_start = self.chunk.bytecode.len();
                self.continue_targets.push(loop_start);
                self.break_patches.push(Vec::new());

                let exit_jump = if let Some(cond_id) = condition {
                    self.compile_node(arena, cond_id);
                    Some(self.emit_jump(Opcode::JumpIfFalse))
                } else {
                    None
                };

                self.compile_node(arena, body);

                if let Some(upd_id) = update {
                    self.compile_node(arena, upd_id);
                    self.emit_op(Opcode::Pop);
                }

                self.emit_loop(loop_start);

                if let Some(ej) = exit_jump {
                    self.patch_jump(ej);
                }

                let breaks = self.break_patches.pop().unwrap_or_default();
                for bp in breaks {
                    self.patch_jump(bp);
                }
                self.continue_targets.pop();
            }

            AstNode::Block(stmts) => {
                self.scope_depth += 1;
                for stmt in &stmts {
                    self.compile_node(arena, *stmt);
                }
                self.scope_depth -= 1;
                // Pop locals from this scope
                while let Some(local) = self.locals.last() {
                    if local.depth > self.scope_depth {
                        self.locals.pop();
                    } else {
                        break;
                    }
                }
            }

            AstNode::ExprStatement(expr) => {
                self.compile_node(arena, expr);
                self.emit_op(Opcode::Pop);
            }

            AstNode::BinaryExpr { op, left, right } => {
                self.compile_node(arena, left);
                self.compile_node(arena, right);
                let opcode = match op {
                    BinOp::Add => Opcode::Add,
                    BinOp::Sub => Opcode::Sub,
                    BinOp::Mul => Opcode::Mul,
                    BinOp::Div => Opcode::Div,
                    BinOp::Mod => Opcode::Mod,
                    BinOp::Eq => Opcode::Eq,
                    BinOp::StrictEq => Opcode::StrictEq,
                    BinOp::NotEq => Opcode::NotEq,
                    BinOp::StrictNotEq => Opcode::StrictNotEq,
                    BinOp::Lt => Opcode::Lt,
                    BinOp::Gt => Opcode::Gt,
                    BinOp::LtEq => Opcode::LtEq,
                    BinOp::GtEq => Opcode::GtEq,
                    BinOp::And => Opcode::LogicalAnd,
                    BinOp::Or => Opcode::LogicalOr,
                    BinOp::BitAnd => Opcode::BitAnd,
                    BinOp::BitOr => Opcode::BitOr,
                    BinOp::BitXor => Opcode::BitXor,
                    BinOp::ShiftLeft => Opcode::ShiftLeft,
                    BinOp::ShiftRight => Opcode::ShiftRight,
                    BinOp::Instanceof => Opcode::Instanceof,
                    BinOp::In => Opcode::In,
                };
                self.emit_op(opcode);
            }

            AstNode::UnaryExpr { op, operand } => {
                self.compile_node(arena, operand);
                match op {
                    UnaryOp::Neg => self.emit_op(Opcode::Neg),
                    UnaryOp::Not => self.emit_op(Opcode::Not),
                    UnaryOp::Typeof => self.emit_op(Opcode::Typeof),
                    UnaryOp::Void => {
                        self.emit_op(Opcode::Pop);
                        let c = self.add_constant(Constant::Undefined);
                        self.emit_op(Opcode::LoadConst);
                        self.emit_u16(c);
                    }
                    UnaryOp::Delete => self.emit_op(Opcode::Pop), // simplified
                    UnaryOp::BitNot => self.emit_op(Opcode::BitXor), // simplified
                }
            }

            AstNode::AssignExpr { target, value } => {
                self.compile_node(arena, value);
                self.emit_op(Opcode::Dup);
                self.compile_store(arena, target);
            }

            AstNode::CompoundAssign { op, target, value } => {
                self.compile_load(arena, target);
                self.compile_node(arena, value);
                let opcode = match op {
                    BinOp::Add => Opcode::Add,
                    BinOp::Sub => Opcode::Sub,
                    BinOp::Mul => Opcode::Mul,
                    BinOp::Div => Opcode::Div,
                    _ => Opcode::Add,
                };
                self.emit_op(opcode);
                self.emit_op(Opcode::Dup);
                self.compile_store(arena, target);
            }

            AstNode::CallExpr { callee, args } => {
                self.compile_node(arena, callee);
                let argc = args.len();
                for arg in &args {
                    self.compile_node(arena, *arg);
                }
                self.emit_op(Opcode::Call);
                self.emit_byte(argc as u8);
            }

            AstNode::MemberExpr { object, property } => {
                self.compile_node(arena, object);
                let prop_idx = self.add_constant(Constant::Str(property));
                self.emit_op(Opcode::LoadProperty);
                self.emit_u16(prop_idx);
            }

            AstNode::IndexExpr { object, index } => {
                self.compile_node(arena, object);
                self.compile_node(arena, index);
                self.emit_op(Opcode::LoadIndex);
            }

            AstNode::ObjectLiteral(props) => {
                self.emit_op(Opcode::CreateObject);
                for (key, val_id) in &props {
                    self.emit_op(Opcode::Dup);
                    self.compile_node(arena, *val_id);
                    let key_idx = self.add_constant(Constant::Str(key.clone()));
                    self.emit_op(Opcode::StoreProperty);
                    self.emit_u16(key_idx);
                }
            }

            AstNode::ArrayLiteral(elems) => {
                let count = elems.len();
                for elem in &elems {
                    self.compile_node(arena, *elem);
                }
                self.emit_op(Opcode::CreateArray);
                self.emit_byte(count as u8);
            }

            AstNode::NumberLit(n) => {
                let idx = self.add_constant(Constant::Number(n));
                self.emit_op(Opcode::LoadConst);
                self.emit_u16(idx);
            }
            AstNode::StringLit(s) => {
                let idx = self.add_constant(Constant::Str(s));
                self.emit_op(Opcode::LoadConst);
                self.emit_u16(idx);
            }
            AstNode::BoolLit(b) => {
                let idx = self.add_constant(Constant::Bool(b));
                self.emit_op(Opcode::LoadConst);
                self.emit_u16(idx);
            }
            AstNode::NullLit => {
                let idx = self.add_constant(Constant::Null);
                self.emit_op(Opcode::LoadConst);
                self.emit_u16(idx);
            }
            AstNode::UndefinedLit => {
                let idx = self.add_constant(Constant::Undefined);
                self.emit_op(Opcode::LoadConst);
                self.emit_u16(idx);
            }

            AstNode::Identifier(name) => {
                if let Some(idx) = self.resolve_local(&name) {
                    self.emit_op(Opcode::LoadLocal);
                    self.emit_u16(idx);
                } else {
                    let name_idx = self.add_constant(Constant::Str(name));
                    self.emit_op(Opcode::LoadGlobal);
                    self.emit_u16(name_idx);
                }
            }

            AstNode::This => {
                self.emit_op(Opcode::GetThis);
            }

            AstNode::FuncExpr { params, body } | AstNode::ArrowFunc { params, body } => {
                let func = self.compile_function(arena, "", &params, body);
                let idx = self.add_constant(Constant::Function(func));
                self.emit_op(Opcode::CreateClosure);
                self.emit_u16(idx);
            }

            AstNode::NewExpr { callee, args } => {
                self.compile_node(arena, callee);
                let argc = args.len();
                for arg in &args {
                    self.compile_node(arena, *arg);
                }
                self.emit_op(Opcode::Call);
                self.emit_byte(argc as u8);
            }

            AstNode::TypeofExpr(operand) => {
                self.compile_node(arena, operand);
                self.emit_op(Opcode::Typeof);
            }

            AstNode::Throw(expr) => {
                self.compile_node(arena, expr);
                self.emit_op(Opcode::Throw);
            }

            AstNode::TryCatch {
                try_body,
                catch_body,
                finally_body,
                catch_param: _,
            } => {
                let try_jump = self.emit_jump(Opcode::EnterTry);
                self.compile_node(arena, try_body);
                self.emit_op(Opcode::LeaveTry);
                let end_jump = self.emit_jump(Opcode::Jump);
                self.patch_jump(try_jump);
                if let Some(catch_id) = catch_body {
                    self.compile_node(arena, catch_id);
                }
                self.patch_jump(end_jump);
                if let Some(finally_id) = finally_body {
                    self.compile_node(arena, finally_id);
                }
            }

            AstNode::Break => {
                let jump = self.emit_jump(Opcode::Jump);
                if let Some(patches) = self.break_patches.last_mut() {
                    patches.push(jump);
                }
            }

            AstNode::Continue => {
                if let Some(&target) = self.continue_targets.last() {
                    self.emit_loop(target);
                }
            }

            AstNode::Empty => {}
        }
    }

    /// Compile a load from an assignment target
    fn compile_load(&mut self, arena: &AstArena, id: AstNodeId) {
        self.compile_node(arena, id);
    }

    /// Compile a store to an assignment target
    fn compile_store(&mut self, arena: &AstArena, target: AstNodeId) {
        match arena.get(target) {
            Some(AstNode::Identifier(name)) => {
                if let Some(idx) = self.resolve_local(name) {
                    self.emit_op(Opcode::StoreLocal);
                    self.emit_u16(idx);
                } else {
                    let name_idx = self.add_constant(Constant::Str(name.clone()));
                    self.emit_op(Opcode::StoreGlobal);
                    self.emit_u16(name_idx);
                }
            }
            Some(AstNode::MemberExpr { object, property }) => {
                let object = *object;
                let property = property.clone();
                self.compile_node(arena, object);
                let prop_idx = self.add_constant(Constant::Str(property));
                self.emit_op(Opcode::StoreProperty);
                self.emit_u16(prop_idx);
            }
            _ => {
                // Cannot store to this target; discard value
                self.emit_op(Opcode::Pop);
            }
        }
    }

    // -- Local variable management --

    fn define_local(&mut self, name: &str) -> u16 {
        let idx = self.next_local;
        self.locals.push(Local {
            name: name.to_string(),
            depth: self.scope_depth,
            index: idx,
        });
        self.next_local += 1;
        idx
    }

    fn resolve_local(&self, name: &str) -> Option<u16> {
        for local in self.locals.iter().rev() {
            if local.name == name {
                return Some(local.index);
            }
        }
        None
    }

    // -- Bytecode emission --

    fn emit_op(&mut self, op: Opcode) {
        self.chunk.bytecode.push(op as u8);
        self.chunk.line_numbers.push(self.current_line);
    }

    fn emit_byte(&mut self, byte: u8) {
        self.chunk.bytecode.push(byte);
        self.chunk.line_numbers.push(self.current_line);
    }

    fn emit_u16(&mut self, val: u16) {
        self.chunk.bytecode.push((val >> 8) as u8);
        self.chunk.bytecode.push(val as u8);
        self.chunk.line_numbers.push(self.current_line);
        self.chunk.line_numbers.push(self.current_line);
    }

    fn emit_jump(&mut self, op: Opcode) -> usize {
        self.emit_op(op);
        let offset = self.chunk.bytecode.len();
        self.emit_u16(0xFFFF); // placeholder
        offset
    }

    fn patch_jump(&mut self, offset: usize) {
        let target = self.chunk.bytecode.len() as u16;
        self.chunk.bytecode[offset] = (target >> 8) as u8;
        self.chunk.bytecode[offset + 1] = target as u8;
    }

    fn emit_loop(&mut self, target: usize) {
        self.emit_op(Opcode::Jump);
        self.emit_u16(target as u16);
    }

    fn add_constant(&mut self, constant: Constant) -> u16 {
        let idx = self.chunk.constants.len();
        self.chunk.constants.push(constant);
        idx as u16
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{js_lexer::js_int, js_parser::JsParser};

    fn compile_src(src: &str) -> Chunk {
        let mut parser = JsParser::from_source(src);
        let root = parser.parse();
        let mut compiler = Compiler::new();
        compiler.compile(&parser.arena, root)
    }

    #[test]
    fn test_empty_program() {
        let chunk = compile_src("");
        // Should just have Halt
        assert_eq!(*chunk.bytecode.last().unwrap(), Opcode::Halt as u8);
    }

    #[test]
    fn test_number_literal() {
        let chunk = compile_src("42;");
        assert!(chunk.bytecode.contains(&(Opcode::LoadConst as u8)));
        assert!(chunk
            .constants
            .iter()
            .any(|c| matches!(c, Constant::Number(n) if *n == js_int(42))));
    }

    #[test]
    fn test_string_literal() {
        let chunk = compile_src("\"hello\";");
        assert!(chunk
            .constants
            .iter()
            .any(|c| matches!(c, Constant::Str(s) if s == "hello")));
    }

    #[test]
    fn test_var_decl_global() {
        let chunk = compile_src("let x = 10;");
        assert!(chunk.bytecode.contains(&(Opcode::StoreGlobal as u8)));
    }

    #[test]
    fn test_binary_add() {
        let chunk = compile_src("1 + 2;");
        assert!(chunk.bytecode.contains(&(Opcode::Add as u8)));
    }

    #[test]
    fn test_binary_mul() {
        let chunk = compile_src("3 * 4;");
        assert!(chunk.bytecode.contains(&(Opcode::Mul as u8)));
    }

    #[test]
    fn test_unary_neg() {
        let chunk = compile_src("-5;");
        assert!(chunk.bytecode.contains(&(Opcode::Neg as u8)));
    }

    #[test]
    fn test_if_statement() {
        let chunk = compile_src("if (true) { 1; }");
        assert!(chunk.bytecode.contains(&(Opcode::JumpIfFalse as u8)));
    }

    #[test]
    fn test_if_else() {
        let chunk = compile_src("if (false) { 1; } else { 2; }");
        let jumps = chunk
            .bytecode
            .iter()
            .filter(|&&b| b == Opcode::JumpIfFalse as u8 || b == Opcode::Jump as u8)
            .count();
        assert!(jumps >= 2);
    }

    #[test]
    fn test_while_loop() {
        let chunk = compile_src("while (true) { 1; }");
        assert!(chunk.bytecode.contains(&(Opcode::JumpIfFalse as u8)));
        assert!(chunk.bytecode.contains(&(Opcode::Jump as u8)));
    }

    #[test]
    fn test_function_decl() {
        let chunk = compile_src("function add(a, b) { return a + b; }");
        let has_func = chunk
            .constants
            .iter()
            .any(|c| matches!(c, Constant::Function(f) if f.name == "add" && f.param_count == 2));
        assert!(has_func);
    }

    #[test]
    fn test_function_call() {
        let chunk = compile_src("foo(1);");
        assert!(chunk.bytecode.contains(&(Opcode::Call as u8)));
    }

    #[test]
    fn test_member_access() {
        let chunk = compile_src("obj.x;");
        assert!(chunk.bytecode.contains(&(Opcode::LoadProperty as u8)));
    }

    #[test]
    fn test_index_access() {
        let chunk = compile_src("arr[0];");
        assert!(chunk.bytecode.contains(&(Opcode::LoadIndex as u8)));
    }

    #[test]
    fn test_object_literal() {
        let chunk = compile_src("let o = {a: 1};");
        assert!(chunk.bytecode.contains(&(Opcode::CreateObject as u8)));
    }

    #[test]
    fn test_array_literal() {
        let chunk = compile_src("let a = [1, 2];");
        assert!(chunk.bytecode.contains(&(Opcode::CreateArray as u8)));
    }

    #[test]
    fn test_assignment() {
        let chunk = compile_src("x = 5;");
        assert!(chunk.bytecode.contains(&(Opcode::StoreGlobal as u8)));
    }

    #[test]
    fn test_return_value() {
        let chunk = compile_src("function f() { return 42; }");
        let has_func = chunk.constants.iter().any(
            |c| matches!(c, Constant::Function(f) if f.bytecode.contains(&(Opcode::Return as u8))),
        );
        assert!(has_func);
    }

    #[test]
    fn test_comparison() {
        let chunk = compile_src("1 < 2;");
        assert!(chunk.bytecode.contains(&(Opcode::Lt as u8)));
    }

    #[test]
    fn test_strict_eq() {
        let chunk = compile_src("a === b;");
        assert!(chunk.bytecode.contains(&(Opcode::StrictEq as u8)));
    }

    #[test]
    fn test_logical_and() {
        let chunk = compile_src("a && b;");
        assert!(chunk.bytecode.contains(&(Opcode::LogicalAnd as u8)));
    }

    #[test]
    fn test_typeof() {
        let chunk = compile_src("typeof x;");
        assert!(chunk.bytecode.contains(&(Opcode::Typeof as u8)));
    }

    #[test]
    fn test_throw() {
        let chunk = compile_src("throw 'error';");
        assert!(chunk.bytecode.contains(&(Opcode::Throw as u8)));
    }

    #[test]
    fn test_try_catch() {
        let chunk = compile_src("try { x(); } catch (e) { y(); }");
        assert!(chunk.bytecode.contains(&(Opcode::EnterTry as u8)));
        assert!(chunk.bytecode.contains(&(Opcode::LeaveTry as u8)));
    }

    #[test]
    fn test_this() {
        let chunk = compile_src("this;");
        assert!(chunk.bytecode.contains(&(Opcode::GetThis as u8)));
    }

    #[test]
    fn test_not_operator() {
        let chunk = compile_src("!x;");
        assert!(chunk.bytecode.contains(&(Opcode::Not as u8)));
    }

    #[test]
    fn test_opcode_from_byte() {
        assert_eq!(Opcode::from_byte(0), Some(Opcode::LoadConst));
        assert_eq!(Opcode::from_byte(255), Some(Opcode::Halt));
        assert_eq!(Opcode::from_byte(99), None);
    }

    #[test]
    fn test_for_loop_compile() {
        let chunk = compile_src("for (let i = 0; i < 10; i = i + 1) { x; }");
        assert!(chunk.bytecode.contains(&(Opcode::Jump as u8)));
    }

    #[test]
    fn test_compound_assign() {
        let chunk = compile_src("x += 1;");
        assert!(chunk.bytecode.contains(&(Opcode::Add as u8)));
    }

    #[test]
    fn test_bool_null_undefined_constants() {
        let chunk = compile_src("true; false; null; undefined;");
        let has_bool_true = chunk
            .constants
            .iter()
            .any(|c| matches!(c, Constant::Bool(true)));
        let has_null = chunk.constants.iter().any(|c| matches!(c, Constant::Null));
        let has_undef = chunk
            .constants
            .iter()
            .any(|c| matches!(c, Constant::Undefined));
        assert!(has_bool_true);
        assert!(has_null);
        assert!(has_undef);
    }

    #[test]
    fn test_closure_compile() {
        let chunk = compile_src("let f = function(x) { return x; };");
        assert!(chunk.bytecode.contains(&(Opcode::CreateClosure as u8)));
    }
}

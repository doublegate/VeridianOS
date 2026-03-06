//! JavaScript Virtual Machine
//!
//! A stack-based bytecode interpreter for JavaScript. Uses arena allocation
//! for objects and 32.32 fixed-point arithmetic for numbers (no floating
//! point).

#![allow(dead_code)]

use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

use super::{
    js_compiler::{Chunk, Constant, FunctionTemplate, Opcode},
    js_lexer::{JsNumber, JS_FRAC_BITS, JS_NAN, JS_ONE, JS_ZERO},
};

// ---------------------------------------------------------------------------
// JS VM Error Type
// ---------------------------------------------------------------------------

/// Errors produced by the JavaScript virtual machine
#[derive(Debug, Clone)]
pub enum JsVmError {
    /// Execution step limit exceeded
    ExecutionLimitExceeded,
    /// Unknown bytecode opcode
    UnknownOpcode { byte: u8 },
    /// Uncaught exception from user code
    UncaughtException { message: String },
    /// Operand stack overflow
    StackOverflow,
    /// Operand stack underflow (pop from empty stack)
    StackUnderflow,
    /// No active call frame
    NoCallFrame,
    /// Instruction pointer out of bounds
    IpOutOfBounds,
    /// Attempted to call a non-callable value
    NotCallable,
    /// Invalid function ID
    InvalidFunctionId,
}

impl core::fmt::Display for JsVmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ExecutionLimitExceeded => write!(f, "Execution limit exceeded"),
            Self::UnknownOpcode { byte } => write!(f, "Unknown opcode: {}", byte),
            Self::UncaughtException { message } => write!(f, "Uncaught: {}", message),
            Self::StackOverflow => write!(f, "Stack overflow"),
            Self::StackUnderflow => write!(f, "Stack underflow"),
            Self::NoCallFrame => write!(f, "No call frame"),
            Self::IpOutOfBounds => write!(f, "IP out of bounds"),
            Self::NotCallable => write!(f, "Not callable"),
            Self::InvalidFunctionId => write!(f, "Invalid function ID"),
        }
    }
}

// ---------------------------------------------------------------------------
// JS Values
// ---------------------------------------------------------------------------

/// Arena index for objects
pub type ObjectId = usize;

/// Arena index for functions
pub type FunctionId = usize;

/// JavaScript value (no floating point)
#[derive(Debug, Clone, Default)]
pub enum JsValue {
    #[default]
    Undefined,
    Null,
    Boolean(bool),
    Number(JsNumber),
    String(String),
    Object(ObjectId),
    Function(FunctionId),
}

impl JsValue {
    /// Convert to boolean (JavaScript truthiness)
    pub fn to_boolean(&self) -> bool {
        match self {
            Self::Undefined | Self::Null => false,
            Self::Boolean(b) => *b,
            Self::Number(n) => *n != JS_ZERO && *n != JS_NAN,
            Self::String(s) => !s.is_empty(),
            Self::Object(_) | Self::Function(_) => true,
        }
    }

    /// Convert to number
    pub fn to_number(&self) -> JsNumber {
        match self {
            Self::Undefined => JS_NAN,
            Self::Null => JS_ZERO,
            Self::Boolean(true) => JS_ONE,
            Self::Boolean(false) => JS_ZERO,
            Self::Number(n) => *n,
            Self::String(s) => parse_js_number(s),
            Self::Object(_) | Self::Function(_) => JS_NAN,
        }
    }

    /// Convert to string
    pub fn to_js_string(&self) -> String {
        match self {
            Self::Undefined => "undefined".to_string(),
            Self::Null => "null".to_string(),
            Self::Boolean(b) => if *b { "true" } else { "false" }.to_string(),
            Self::Number(n) => format_js_number(*n),
            Self::String(s) => s.clone(),
            Self::Object(_) => "[object Object]".to_string(),
            Self::Function(_) => "function".to_string(),
        }
    }

    /// JavaScript typeof
    pub fn js_typeof(&self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::Null => "object", // historic JS quirk
            Self::Boolean(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Object(_) => "object",
            Self::Function(_) => "function",
        }
    }

    /// Strict equality (===)
    pub fn strict_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Undefined, Self::Undefined) => true,
            (Self::Null, Self::Null) => true,
            (Self::Boolean(a), Self::Boolean(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => a == b,
            (Self::Function(a), Self::Function(b)) => a == b,
            _ => false,
        }
    }

    /// Abstract equality (==), simplified
    pub fn abstract_eq(&self, other: &Self) -> bool {
        if self.strict_eq(other) {
            return true;
        }
        match (self, other) {
            (Self::Null, Self::Undefined) | (Self::Undefined, Self::Null) => true,
            (Self::Number(a), Self::String(b)) => *a == parse_js_number(b),
            (Self::String(a), Self::Number(b)) => parse_js_number(a) == *b,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// JS Object (arena-managed)
// ---------------------------------------------------------------------------

/// Object internal type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectType {
    #[default]
    Ordinary,
    Array,
    Function,
    Error,
}

/// A JavaScript object stored in the arena
#[derive(Debug, Clone, Default)]
pub struct JsObject {
    /// Properties
    pub properties: BTreeMap<String, JsValue>,
    /// Prototype (arena index)
    pub prototype: Option<ObjectId>,
    /// Internal type
    pub internal_type: ObjectType,
}

impl JsObject {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn array() -> Self {
        Self {
            internal_type: ObjectType::Array,
            ..Default::default()
        }
    }

    pub fn get(&self, key: &str) -> JsValue {
        self.properties
            .get(key)
            .cloned()
            .unwrap_or(JsValue::Undefined)
    }

    pub fn set(&mut self, key: &str, value: JsValue) {
        self.properties.insert(key.to_string(), value);
    }
}

// ---------------------------------------------------------------------------
// Call frame
// ---------------------------------------------------------------------------

/// A function call frame on the call stack
#[derive(Debug, Clone)]
pub struct CallFrame {
    /// Function template being executed
    pub function_id: FunctionId,
    /// Instruction pointer within the function's bytecode
    pub ip: usize,
    /// Base slot on the operand stack
    pub base_slot: usize,
    /// Local variables
    pub locals: Vec<JsValue>,
    /// Bytecode reference (copied from template for execution)
    pub bytecode: Vec<u8>,
    /// Constants reference
    pub constants: Vec<Constant>,
}

// ---------------------------------------------------------------------------
// Try/catch state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TryState {
    catch_ip: usize,
    stack_depth: usize,
    call_depth: usize,
}

// ---------------------------------------------------------------------------
// VM
// ---------------------------------------------------------------------------

/// JavaScript virtual machine
pub struct JsVm {
    /// Operand stack
    pub stack: Vec<JsValue>,
    /// Call stack
    pub call_stack: Vec<CallFrame>,
    /// Global variables
    pub globals: BTreeMap<String, JsValue>,
    /// Object arena
    pub object_arena: Vec<JsObject>,
    /// Function templates
    pub function_arena: Vec<FunctionTemplate>,
    /// Try/catch stack
    try_stack: Vec<TryState>,
    /// Output buffer (from console.log)
    pub output: Vec<String>,
    /// Maximum stack depth
    max_stack: usize,
    /// Maximum execution steps
    max_steps: usize,
}

impl Default for JsVm {
    fn default() -> Self {
        Self::new()
    }
}

impl JsVm {
    pub fn new() -> Self {
        let mut vm = Self {
            stack: Vec::with_capacity(256),
            call_stack: Vec::new(),
            globals: BTreeMap::new(),
            object_arena: Vec::new(),
            function_arena: Vec::new(),
            try_stack: Vec::new(),
            output: Vec::new(),
            max_stack: 1024,
            max_steps: 100_000,
        };
        vm.init_builtins();
        vm
    }

    /// Sentinel ID for native built-in objects (high value to avoid collision
    /// with GC heap indices)
    const NATIVE_CONSOLE_ID: ObjectId = usize::MAX;

    /// Register built-in functions and objects
    fn init_builtins(&mut self) {
        // Use a sentinel ID for native objects so the GC's mark phase
        // never collides with GC-heap-allocated objects at low indices.
        self.globals.insert(
            "console".to_string(),
            JsValue::Object(Self::NATIVE_CONSOLE_ID),
        );
    }

    /// Allocate an object in the arena
    pub fn alloc_object(&mut self, obj: JsObject) -> ObjectId {
        let id = self.object_arena.len();
        self.object_arena.push(obj);
        id
    }

    /// Register a function template
    pub fn register_function(&mut self, template: FunctionTemplate) -> FunctionId {
        let id = self.function_arena.len();
        self.function_arena.push(template);
        id
    }

    /// Execute a compiled chunk (top-level code)
    pub fn run_chunk(&mut self, chunk: &Chunk) -> Result<JsValue, JsVmError> {
        // Create a top-level function template from the chunk
        let template = FunctionTemplate {
            name: "<main>".to_string(),
            param_count: 0,
            bytecode: chunk.bytecode.clone(),
            constants: chunk.constants.clone(),
            local_count: 0,
            upvalue_count: 0,
            line_numbers: chunk.line_numbers.clone(),
        };

        let frame = CallFrame {
            function_id: usize::MAX,
            ip: 0,
            base_slot: 0,
            locals: Vec::new(),
            bytecode: template.bytecode,
            constants: template.constants,
        };

        self.call_stack.push(frame);
        self.run_loop()
    }

    /// Main interpreter loop
    fn run_loop(&mut self) -> Result<JsValue, JsVmError> {
        let mut steps = 0usize;

        loop {
            steps += 1;
            if steps > self.max_steps {
                return Err(JsVmError::ExecutionLimitExceeded);
            }

            let frame = match self.call_stack.last_mut() {
                Some(f) => f,
                None => {
                    return Ok(self.stack.pop().unwrap_or(JsValue::Undefined));
                }
            };

            if frame.ip >= frame.bytecode.len() {
                self.call_stack.pop();
                continue;
            }

            let op_byte = frame.bytecode[frame.ip];
            frame.ip += 1;

            let op = match Opcode::from_byte(op_byte) {
                Some(o) => o,
                None => {
                    return Err(JsVmError::UnknownOpcode { byte: op_byte });
                }
            };

            match op {
                Opcode::Halt => {
                    return Ok(self.stack.pop().unwrap_or(JsValue::Undefined));
                }

                Opcode::LoadConst => {
                    let idx = self.read_u16()? as usize;
                    let frame = self.call_stack.last().unwrap();
                    let val = match frame.constants.get(idx) {
                        Some(Constant::Number(n)) => JsValue::Number(*n),
                        Some(Constant::Str(s)) => JsValue::String(s.clone()),
                        Some(Constant::Bool(b)) => JsValue::Boolean(*b),
                        Some(Constant::Null) => JsValue::Null,
                        Some(Constant::Undefined) => JsValue::Undefined,
                        Some(Constant::Function(f)) => {
                            let fid = self.register_function(f.clone());
                            JsValue::Function(fid)
                        }
                        None => JsValue::Undefined,
                    };
                    self.push(val)?;
                }

                Opcode::LoadLocal => {
                    let idx = self.read_u16()? as usize;
                    let val = self
                        .call_stack
                        .last()
                        .and_then(|f| f.locals.get(idx))
                        .cloned()
                        .unwrap_or(JsValue::Undefined);
                    self.push(val)?;
                }

                Opcode::StoreLocal => {
                    let idx = self.read_u16()? as usize;
                    let val = self.pop()?;
                    if let Some(frame) = self.call_stack.last_mut() {
                        while frame.locals.len() <= idx {
                            frame.locals.push(JsValue::Undefined);
                        }
                        frame.locals[idx] = val;
                    }
                }

                Opcode::LoadGlobal => {
                    let idx = self.read_u16()? as usize;
                    let name = self.get_string_constant(idx)?;
                    let val = self
                        .globals
                        .get(&name)
                        .cloned()
                        .unwrap_or(JsValue::Undefined);
                    self.push(val)?;
                }

                Opcode::StoreGlobal => {
                    let idx = self.read_u16()? as usize;
                    let name = self.get_string_constant(idx)?;
                    let val = self.pop()?;
                    self.globals.insert(name, val);
                }

                Opcode::LoadProperty => {
                    let idx = self.read_u16()? as usize;
                    let prop = self.get_string_constant(idx)?;
                    let obj_val = self.pop()?;
                    let val = self.get_property(&obj_val, &prop);
                    self.push(val)?;
                }

                Opcode::StoreProperty => {
                    let idx = self.read_u16()? as usize;
                    let prop = self.get_string_constant(idx)?;
                    let val = self.pop()?;
                    let obj_val = self.pop()?;
                    self.set_property(&obj_val, &prop, val);
                }

                Opcode::LoadIndex => {
                    let index = self.pop()?;
                    let obj = self.pop()?;
                    let key = index.to_js_string();
                    let val = self.get_property(&obj, &key);
                    self.push(val)?;
                }

                Opcode::StoreIndex => {
                    let val = self.pop()?;
                    let index = self.pop()?;
                    let obj = self.pop()?;
                    let key = index.to_js_string();
                    self.set_property(&obj, &key, val);
                }

                // Arithmetic
                Opcode::Add => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = self.js_add(&a, &b);
                    self.push(result)?;
                }
                Opcode::Sub => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = JsValue::Number(a.to_number().wrapping_sub(b.to_number()));
                    self.push(result)?;
                }
                Opcode::Mul => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = js_mul(a.to_number(), b.to_number());
                    self.push(JsValue::Number(result))?;
                }
                Opcode::Div => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let result = js_div(a.to_number(), b.to_number());
                    self.push(JsValue::Number(result))?;
                }
                Opcode::Mod => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    let bn = b.to_number();
                    let result = if bn == 0 {
                        JS_NAN
                    } else {
                        a.to_number().wrapping_rem(bn)
                    };
                    self.push(JsValue::Number(result))?;
                }
                Opcode::Neg => {
                    let a = self.pop()?;
                    self.push(JsValue::Number(-a.to_number()))?;
                }
                Opcode::Not => {
                    let a = self.pop()?;
                    self.push(JsValue::Boolean(!a.to_boolean()))?;
                }

                // Bitwise
                Opcode::BitAnd => {
                    let b = self.pop()?.to_number() >> JS_FRAC_BITS;
                    let a = self.pop()?.to_number() >> JS_FRAC_BITS;
                    self.push(JsValue::Number((a & b) << JS_FRAC_BITS))?;
                }
                Opcode::BitOr => {
                    let b = self.pop()?.to_number() >> JS_FRAC_BITS;
                    let a = self.pop()?.to_number() >> JS_FRAC_BITS;
                    self.push(JsValue::Number((a | b) << JS_FRAC_BITS))?;
                }
                Opcode::BitXor => {
                    let b = self.pop()?.to_number() >> JS_FRAC_BITS;
                    let a = self.pop()?.to_number() >> JS_FRAC_BITS;
                    self.push(JsValue::Number((a ^ b) << JS_FRAC_BITS))?;
                }
                Opcode::ShiftLeft => {
                    let b = (self.pop()?.to_number() >> JS_FRAC_BITS) as u32;
                    let a = self.pop()?.to_number() >> JS_FRAC_BITS;
                    let shift = b & 31;
                    self.push(JsValue::Number((a << shift) << JS_FRAC_BITS))?;
                }
                Opcode::ShiftRight => {
                    let b = (self.pop()?.to_number() >> JS_FRAC_BITS) as u32;
                    let a = self.pop()?.to_number() >> JS_FRAC_BITS;
                    let shift = b & 31;
                    self.push(JsValue::Number((a >> shift) << JS_FRAC_BITS))?;
                }

                // Comparison
                Opcode::Eq => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(JsValue::Boolean(a.abstract_eq(&b)))?;
                }
                Opcode::StrictEq => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(JsValue::Boolean(a.strict_eq(&b)))?;
                }
                Opcode::NotEq => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(JsValue::Boolean(!a.abstract_eq(&b)))?;
                }
                Opcode::StrictNotEq => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(JsValue::Boolean(!a.strict_eq(&b)))?;
                }
                Opcode::Lt => {
                    let b = self.pop()?.to_number();
                    let a = self.pop()?.to_number();
                    self.push(JsValue::Boolean(a < b))?;
                }
                Opcode::Gt => {
                    let b = self.pop()?.to_number();
                    let a = self.pop()?.to_number();
                    self.push(JsValue::Boolean(a > b))?;
                }
                Opcode::LtEq => {
                    let b = self.pop()?.to_number();
                    let a = self.pop()?.to_number();
                    self.push(JsValue::Boolean(a <= b))?;
                }
                Opcode::GtEq => {
                    let b = self.pop()?.to_number();
                    let a = self.pop()?.to_number();
                    self.push(JsValue::Boolean(a >= b))?;
                }

                // Logical short-circuit
                Opcode::LogicalAnd => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if a.to_boolean() {
                        self.push(b)?;
                    } else {
                        self.push(a)?;
                    }
                }
                Opcode::LogicalOr => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    if a.to_boolean() {
                        self.push(a)?;
                    } else {
                        self.push(b)?;
                    }
                }

                // Control flow
                Opcode::Jump => {
                    let target = self.read_u16()? as usize;
                    if let Some(frame) = self.call_stack.last_mut() {
                        frame.ip = target;
                    }
                }
                Opcode::JumpIfFalse => {
                    let target = self.read_u16()? as usize;
                    let val = self.pop()?;
                    if !val.to_boolean() {
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.ip = target;
                        }
                    }
                }
                Opcode::JumpIfTrue => {
                    let target = self.read_u16()? as usize;
                    let val = self.pop()?;
                    if val.to_boolean() {
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.ip = target;
                        }
                    }
                }

                // Functions
                Opcode::Call => {
                    let argc = self.read_byte()? as usize;
                    self.call_function(argc)?;
                }
                Opcode::Return => {
                    let val = self.pop()?;
                    self.call_stack.pop();
                    self.push(val)?;
                }
                Opcode::CreateClosure => {
                    let idx = self.read_u16()? as usize;
                    let frame = self.call_stack.last().unwrap();
                    if let Some(Constant::Function(f)) = frame.constants.get(idx) {
                        let fid = self.register_function(f.clone());
                        self.push(JsValue::Function(fid))?;
                    } else {
                        self.push(JsValue::Undefined)?;
                    }
                }

                // Object/array
                Opcode::CreateObject => {
                    let oid = self.alloc_object(JsObject::new());
                    self.push(JsValue::Object(oid))?;
                }
                Opcode::CreateArray => {
                    let count = self.read_byte()? as usize;
                    let mut arr = JsObject::array();
                    let start = self.stack.len().saturating_sub(count);
                    for i in 0..count {
                        let idx = start + i;
                        let val = if idx < self.stack.len() {
                            self.stack[idx].clone()
                        } else {
                            JsValue::Undefined
                        };
                        arr.set(&format!("{}", i), val);
                    }
                    arr.set("length", JsValue::Number((count as i64) << JS_FRAC_BITS));
                    self.stack.truncate(start);
                    let oid = self.alloc_object(arr);
                    self.push(JsValue::Object(oid))?;
                }

                // Misc
                Opcode::GetThis => {
                    self.push(JsValue::Undefined)?; // simplified
                }
                Opcode::Typeof => {
                    let val = self.pop()?;
                    self.push(JsValue::String(val.js_typeof().to_string()))?;
                }
                Opcode::Instanceof | Opcode::In => {
                    let _b = self.pop()?;
                    let _a = self.pop()?;
                    self.push(JsValue::Boolean(false))?; // simplified
                }
                Opcode::Throw => {
                    let val = self.pop()?;
                    if let Some(try_state) = self.try_stack.pop() {
                        while self.call_stack.len() > try_state.call_depth {
                            self.call_stack.pop();
                        }
                        self.stack.truncate(try_state.stack_depth);
                        self.push(val)?;
                        if let Some(frame) = self.call_stack.last_mut() {
                            frame.ip = try_state.catch_ip;
                        }
                    } else {
                        return Err(JsVmError::UncaughtException {
                            message: val.to_js_string(),
                        });
                    }
                }
                Opcode::EnterTry => {
                    let catch_ip = self.read_u16()? as usize;
                    self.try_stack.push(TryState {
                        catch_ip,
                        stack_depth: self.stack.len(),
                        call_depth: self.call_stack.len(),
                    });
                }
                Opcode::LeaveTry => {
                    self.try_stack.pop();
                }

                // Stack
                Opcode::Pop => {
                    self.pop()?;
                }
                Opcode::Dup => {
                    let val = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    self.push(val)?;
                }
            }
        }
    }

    // -- Helpers --

    fn push(&mut self, val: JsValue) -> Result<(), JsVmError> {
        if self.stack.len() >= self.max_stack {
            return Err(JsVmError::StackOverflow);
        }
        self.stack.push(val);
        Ok(())
    }

    fn pop(&mut self) -> Result<JsValue, JsVmError> {
        self.stack.pop().ok_or(JsVmError::StackUnderflow)
    }

    fn read_byte(&mut self) -> Result<u8, JsVmError> {
        let frame = self.call_stack.last_mut().ok_or(JsVmError::NoCallFrame)?;
        if frame.ip >= frame.bytecode.len() {
            return Err(JsVmError::IpOutOfBounds);
        }
        let b = frame.bytecode[frame.ip];
        frame.ip += 1;
        Ok(b)
    }

    fn read_u16(&mut self) -> Result<u16, JsVmError> {
        let hi = self.read_byte()? as u16;
        let lo = self.read_byte()? as u16;
        Ok((hi << 8) | lo)
    }

    fn get_string_constant(&self, idx: usize) -> Result<String, JsVmError> {
        let frame = self.call_stack.last().ok_or(JsVmError::NoCallFrame)?;
        match frame.constants.get(idx) {
            Some(Constant::Str(s)) => Ok(s.clone()),
            _ => Ok(format!("{}", idx)),
        }
    }

    fn get_property(&self, obj: &JsValue, key: &str) -> JsValue {
        match obj {
            JsValue::Object(oid) => {
                if let Some(obj) = self.object_arena.get(*oid) {
                    if key == "length" && obj.internal_type == ObjectType::Array {
                        return obj.get("length");
                    }
                    obj.get(key)
                } else {
                    JsValue::Undefined
                }
            }
            JsValue::String(s) => {
                if key == "length" {
                    JsValue::Number((s.len() as i64) << JS_FRAC_BITS)
                } else {
                    JsValue::Undefined
                }
            }
            _ => JsValue::Undefined,
        }
    }

    fn set_property(&mut self, obj: &JsValue, key: &str, val: JsValue) {
        if let JsValue::Object(oid) = obj {
            if let Some(o) = self.object_arena.get_mut(*oid) {
                if o.internal_type == ObjectType::Array {
                    if let Ok(idx) = key.parse::<usize>() {
                        let current_len = match o.get("length") {
                            JsValue::Number(n) => (n >> JS_FRAC_BITS) as usize,
                            _ => 0,
                        };
                        if idx >= current_len {
                            o.set(
                                "length",
                                JsValue::Number(((idx + 1) as i64) << JS_FRAC_BITS),
                            );
                        }
                    }
                }
                o.set(key, val);
            }
        }
    }

    /// JavaScript addition: string concatenation or numeric add
    fn js_add(&self, a: &JsValue, b: &JsValue) -> JsValue {
        if matches!(a, JsValue::String(_)) || matches!(b, JsValue::String(_)) {
            let mut s = a.to_js_string();
            s.push_str(&b.to_js_string());
            JsValue::String(s)
        } else {
            JsValue::Number(a.to_number().wrapping_add(b.to_number()))
        }
    }

    /// Call a function value
    fn call_function(&mut self, argc: usize) -> Result<(), JsVmError> {
        let mut args = Vec::with_capacity(argc);
        for _ in 0..argc {
            args.push(self.pop()?);
        }
        args.reverse();

        let callee = self.pop()?;

        match callee {
            JsValue::Function(fid) => {
                let template = self
                    .function_arena
                    .get(fid)
                    .cloned()
                    .ok_or(JsVmError::InvalidFunctionId)?;

                let mut locals = Vec::with_capacity(template.local_count);
                for i in 0..template.param_count {
                    locals.push(args.get(i).cloned().unwrap_or(JsValue::Undefined));
                }
                while locals.len() < template.local_count {
                    locals.push(JsValue::Undefined);
                }

                let frame = CallFrame {
                    function_id: fid,
                    ip: 0,
                    base_slot: self.stack.len(),
                    locals,
                    bytecode: template.bytecode.clone(),
                    constants: template.constants.clone(),
                };
                self.call_stack.push(frame);
            }
            JsValue::Object(_oid) => {
                // Builtin object call (console.log style)
                let msg: Vec<String> = args.iter().map(|a| a.to_js_string()).collect();
                self.output.push(msg.join(" "));
                self.push(JsValue::Undefined)?;
            }
            _ => {
                return Err(JsVmError::NotCallable);
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Number helpers (32.32 fixed-point, no float)
// ---------------------------------------------------------------------------

/// Multiply two 32.32 fixed-point numbers
fn js_mul(a: JsNumber, b: JsNumber) -> JsNumber {
    if a == JS_NAN || b == JS_NAN {
        return JS_NAN;
    }
    ((a as i128 * b as i128) >> JS_FRAC_BITS) as i64
}

/// Divide two 32.32 fixed-point numbers
fn js_div(a: JsNumber, b: JsNumber) -> JsNumber {
    if b == JS_ZERO || a == JS_NAN || b == JS_NAN {
        return JS_NAN;
    }
    (((a as i128) << JS_FRAC_BITS) / b as i128) as i64
}

/// Parse a string to JsNumber (simplified integer-only)
fn parse_js_number(s: &str) -> JsNumber {
    let s = s.trim();
    if s.is_empty() {
        return JS_ZERO;
    }
    let (neg, s) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };
    let mut result: i64 = 0;
    for &b in s.as_bytes() {
        if b.is_ascii_digit() {
            result = result.wrapping_mul(10).wrapping_add((b - b'0') as i64);
        } else {
            return JS_NAN;
        }
    }
    let fixed = result << JS_FRAC_BITS;
    if neg {
        -fixed
    } else {
        fixed
    }
}

/// Format a JsNumber as a string
fn format_js_number(n: JsNumber) -> String {
    if n == JS_NAN {
        return "NaN".to_string();
    }
    let int_part = n >> JS_FRAC_BITS;
    let frac_mask = (1i64 << JS_FRAC_BITS) - 1;
    let frac_part = n & frac_mask;

    if frac_part == 0 {
        format!("{}", int_part)
    } else {
        let frac_decimal = (frac_part.unsigned_abs() * 1_000_000) >> JS_FRAC_BITS;
        let frac_str = format!("{:06}", frac_decimal);
        let trimmed = frac_str.trim_end_matches('0');
        if int_part < 0 || (int_part == 0 && n < 0) {
            format!("-{}.{}", int_part.unsigned_abs(), trimmed)
        } else {
            format!("{}.{}", int_part, trimmed)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{js_compiler::Compiler, js_lexer::js_int, js_parser::JsParser};

    fn run_js(src: &str) -> Result<JsValue, JsVmError> {
        let mut parser = JsParser::from_source(src);
        let root = parser.parse();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&parser.arena, root);
        let mut vm = JsVm::new();
        vm.run_chunk(&chunk)
    }

    fn run_js_global(src: &str, name: &str) -> JsValue {
        let mut parser = JsParser::from_source(src);
        let root = parser.parse();
        let mut compiler = Compiler::new();
        let chunk = compiler.compile(&parser.arena, root);
        let mut vm = JsVm::new();
        let _ = vm.run_chunk(&chunk);
        vm.globals.get(name).cloned().unwrap_or(JsValue::Undefined)
    }

    #[test]
    fn test_number_literal() {
        let _result = run_js("42;").unwrap();
    }

    #[test]
    fn test_var_and_load() {
        let val = run_js_global("let x = 10;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(10)));
    }

    #[test]
    fn test_addition() {
        let val = run_js_global("let x = 1 + 2;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(3)));
    }

    #[test]
    fn test_subtraction() {
        let val = run_js_global("let x = 10 - 3;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(7)));
    }

    #[test]
    fn test_multiplication() {
        let val = run_js_global("let x = 3 * 4;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(12)));
    }

    #[test]
    fn test_division() {
        let val = run_js_global("let x = 10 / 2;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(5)));
    }

    #[test]
    fn test_string_concat() {
        let val = run_js_global("let x = 'hello' + ' ' + 'world';", "x");
        assert!(matches!(val, JsValue::String(ref s) if s == "hello world"));
    }

    #[test]
    fn test_comparison_lt() {
        let val = run_js_global("let x = 1 < 2;", "x");
        assert!(matches!(val, JsValue::Boolean(true)));
    }

    #[test]
    fn test_comparison_gt() {
        let val = run_js_global("let x = 5 > 3;", "x");
        assert!(matches!(val, JsValue::Boolean(true)));
    }

    #[test]
    fn test_strict_eq() {
        let val = run_js_global("let x = 1 === 1;", "x");
        assert!(matches!(val, JsValue::Boolean(true)));
    }

    #[test]
    fn test_strict_neq() {
        let val = run_js_global("let x = 1 !== 2;", "x");
        assert!(matches!(val, JsValue::Boolean(true)));
    }

    #[test]
    fn test_if_true() {
        let val = run_js_global("let x = 0; if (true) { x = 1; }", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(1)));
    }

    #[test]
    fn test_if_false() {
        let val = run_js_global("let x = 0; if (false) { x = 1; } else { x = 2; }", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(2)));
    }

    #[test]
    fn test_while_loop() {
        let val = run_js_global("let x = 0; while (x < 5) { x = x + 1; }", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(5)));
    }

    #[test]
    fn test_for_loop() {
        let val = run_js_global(
            "let sum = 0; for (let i = 1; i < 4; i = i + 1) { sum = sum + i; }",
            "sum",
        );
        assert!(matches!(val, JsValue::Number(n) if n == js_int(6)));
    }

    #[test]
    fn test_function_call() {
        let val = run_js_global(
            "function add(a, b) { return a + b; } let x = add(3, 4);",
            "x",
        );
        assert!(matches!(val, JsValue::Number(n) if n == js_int(7)));
    }

    #[test]
    fn test_nested_function() {
        let val = run_js_global(
            "function f(x) { return x * 2; } function g(x) { return f(x) + 1; } let r = g(5);",
            "r",
        );
        assert!(matches!(val, JsValue::Number(n) if n == js_int(11)));
    }

    #[test]
    fn test_object_property() {
        let val = run_js_global("let o = {x: 42}; let v = o.x;", "v");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(42)));
    }

    #[test]
    fn test_array_access() {
        let val = run_js_global("let a = [10, 20, 30]; let v = a[1];", "v");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(20)));
    }

    #[test]
    fn test_string_length() {
        let val = run_js_global("let s = 'hello'; let v = s.length;", "v");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(5)));
    }

    #[test]
    fn test_boolean_not() {
        let val = run_js_global("let x = !true;", "x");
        assert!(matches!(val, JsValue::Boolean(false)));
    }

    #[test]
    fn test_negation() {
        let val = run_js_global("let x = -5;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(-5)));
    }

    #[test]
    fn test_typeof() {
        let val = run_js_global("let x = typeof 42;", "x");
        assert!(matches!(val, JsValue::String(ref s) if s == "number"));
    }

    #[test]
    fn test_null_undefined() {
        let val = run_js_global("let x = null;", "x");
        assert!(matches!(val, JsValue::Null));
        let val2 = run_js_global("let x = undefined;", "x");
        assert!(matches!(val2, JsValue::Undefined));
    }

    #[test]
    fn test_logical_and() {
        let val = run_js_global("let x = true && 42;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(42)));
    }

    #[test]
    fn test_logical_or() {
        let val = run_js_global("let x = false || 'fallback';", "x");
        assert!(matches!(val, JsValue::String(ref s) if s == "fallback"));
    }

    #[test]
    fn test_compound_assign() {
        let val = run_js_global("let x = 5; x += 3;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(8)));
    }

    #[test]
    fn test_js_value_truthiness() {
        assert!(!JsValue::Undefined.to_boolean());
        assert!(!JsValue::Null.to_boolean());
        assert!(!JsValue::Boolean(false).to_boolean());
        assert!(!JsValue::Number(JS_ZERO).to_boolean());
        assert!(!JsValue::String(String::new()).to_boolean());
        assert!(JsValue::Boolean(true).to_boolean());
        assert!(JsValue::Number(JS_ONE).to_boolean());
        assert!(JsValue::String("x".to_string()).to_boolean());
    }

    #[test]
    fn test_js_value_to_string() {
        assert_eq!(JsValue::Undefined.to_js_string(), "undefined");
        assert_eq!(JsValue::Null.to_js_string(), "null");
        assert_eq!(JsValue::Boolean(true).to_js_string(), "true");
        assert_eq!(JsValue::Number(js_int(42)).to_js_string(), "42");
    }

    #[test]
    fn test_abstract_eq_null_undefined() {
        assert!(JsValue::Null.abstract_eq(&JsValue::Undefined));
        assert!(JsValue::Undefined.abstract_eq(&JsValue::Null));
    }

    #[test]
    fn test_js_mul_overflow_safe() {
        let r = js_mul(js_int(1000), js_int(1000));
        assert_eq!(r >> JS_FRAC_BITS, 1_000_000);
    }

    #[test]
    fn test_js_div_by_zero() {
        assert_eq!(js_div(js_int(1), JS_ZERO), JS_NAN);
    }

    #[test]
    fn test_format_js_number_integer() {
        assert_eq!(format_js_number(js_int(42)), "42");
        assert_eq!(format_js_number(js_int(-5)), "-5");
        assert_eq!(format_js_number(js_int(0)), "0");
    }

    #[test]
    fn test_format_js_number_nan() {
        assert_eq!(format_js_number(JS_NAN), "NaN");
    }

    #[test]
    fn test_parse_js_number() {
        assert_eq!(parse_js_number("42"), js_int(42));
        assert_eq!(parse_js_number("-10"), js_int(-10));
        assert_eq!(parse_js_number(""), JS_ZERO);
        assert_eq!(parse_js_number("abc"), JS_NAN);
    }

    #[test]
    fn test_execution_limit() {
        let result = run_js("while (true) { }");
        assert!(result.is_err());
    }

    #[test]
    fn test_object_type_default() {
        assert_eq!(ObjectType::default(), ObjectType::Ordinary);
    }

    #[test]
    fn test_js_object_array() {
        let arr = JsObject::array();
        assert_eq!(arr.internal_type, ObjectType::Array);
    }

    #[test]
    fn test_recursive_function() {
        let val = run_js_global(
            "function fib(n) { if (n < 2) { return n; } return fib(n - 1) + fib(n - 2); } let x = \
             fib(6);",
            "x",
        );
        assert!(matches!(val, JsValue::Number(n) if n == js_int(8)));
    }

    #[test]
    fn test_multiple_vars() {
        run_js_global("let a = 1; let b = 2; let c = 3;", "c");
    }

    #[test]
    fn test_empty_function() {
        run_js_global("function f() {} let x = f();", "x");
    }

    #[test]
    fn test_return_no_value() {
        let val = run_js_global("function f() { return; } let x = f();", "x");
        assert!(matches!(val, JsValue::Undefined));
    }

    #[test]
    fn test_bitwise_and() {
        let val = run_js_global("let x = 6 & 3;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(2)));
    }

    #[test]
    fn test_bitwise_or() {
        let val = run_js_global("let x = 4 | 2;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(6)));
    }

    #[test]
    fn test_modulo() {
        let val = run_js_global("let x = 10 % 3;", "x");
        assert!(matches!(val, JsValue::Number(n) if n == js_int(1)));
    }
}

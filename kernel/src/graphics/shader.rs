//! Shader Compiler and Executor
//!
//! Provides a software shader pipeline using TGSI-like instructions. Shaders
//! are compiled from high-level descriptions into instruction lists, then
//! executed per-pixel by the software rasteriser.
//!
//! All arithmetic uses integer or 16.16 fixed-point math (no FPU required).

#![allow(dead_code)]

use alloc::{collections::BTreeMap, string::String, vec::Vec};

// ---------------------------------------------------------------------------
// Fixed-point helpers (16.16)
// ---------------------------------------------------------------------------

/// 16.16 fixed-point shift.
const FP_SHIFT: i32 = 16;

/// 1.0 in 16.16 fixed-point.
const FP_ONE: i32 = 1 << FP_SHIFT;

/// Multiply two 16.16 fixed-point values.
fn fp_mul(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) >> FP_SHIFT) as i32
}

/// Integer to 16.16 fixed-point.
fn fp_from_int(v: i32) -> i32 {
    v << FP_SHIFT
}

/// 16.16 fixed-point to integer (truncate).
fn fp_to_int(v: i32) -> i32 {
    v >> FP_SHIFT
}

/// Reciprocal: 1/x in 16.16 fixed-point.
fn fp_rcp(x: i32) -> i32 {
    if x == 0 {
        return 0;
    }
    ((FP_ONE as i64 * FP_ONE as i64) / x as i64) as i32
}

// ---------------------------------------------------------------------------
// Shader types
// ---------------------------------------------------------------------------

/// Type of shader in the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderType {
    /// Processes vertex positions and attributes.
    Vertex,
    /// Computes pixel (fragment) colour.
    Fragment,
}

/// A uniform value passed to shader programs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UniformValue {
    /// Single integer.
    Int(i32),
    /// 2-component vector.
    Vec2(i32, i32),
    /// 4-component vector.
    Vec4(i32, i32, i32, i32),
    /// 4x4 matrix (column-major, 16.16 fixed-point).
    Mat4([i32; 16]),
}

// ---------------------------------------------------------------------------
// TGSI-like instructions
// ---------------------------------------------------------------------------

/// Source operand for a TGSI instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrcOperand {
    /// Register index.
    Reg(u8),
    /// Uniform slot.
    Uniform(u8),
    /// Immediate 16.16 fixed-point value.
    Immediate(i32),
}

impl Default for SrcOperand {
    fn default() -> Self {
        Self::Immediate(0)
    }
}

/// TGSI-like instruction set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TgsiInstruction {
    /// Move: dst = src
    MOV { dst: u8, src: SrcOperand },
    /// Add: dst = src0 + src1
    ADD {
        dst: u8,
        src0: SrcOperand,
        src1: SrcOperand,
    },
    /// Multiply: dst = src0 * src1 (16.16 fp_mul)
    MUL {
        dst: u8,
        src0: SrcOperand,
        src1: SrcOperand,
    },
    /// Multiply-add: dst = src0 * src1 + src2
    MAD {
        dst: u8,
        src0: SrcOperand,
        src1: SrcOperand,
        src2: SrcOperand,
    },
    /// 3-component dot product
    DP3 {
        dst: u8,
        src0: SrcOperand,
        src1: SrcOperand,
    },
    /// 4-component dot product
    DP4 {
        dst: u8,
        src0: SrcOperand,
        src1: SrcOperand,
    },
    /// Texture sample (stub — reads from texture slot)
    TEX { dst: u8, coord: SrcOperand },
    /// Sample from texture (alias for TEX with sampler index)
    SAMPLE {
        dst: u8,
        coord: SrcOperand,
        sampler: u8,
    },
    /// Reciprocal: dst = 1.0 / src
    RCP { dst: u8, src: SrcOperand },
    /// Reciprocal square root (integer approximation)
    RSQ { dst: u8, src: SrcOperand },
}

// ---------------------------------------------------------------------------
// Shader program
// ---------------------------------------------------------------------------

/// A compiled shader program containing instructions and uniform bindings.
#[derive(Debug, Clone)]
pub struct ShaderProgram {
    /// Type of shader.
    pub shader_type: ShaderType,
    /// Compiled instruction list.
    pub instructions: Vec<TgsiInstruction>,
    /// Named uniform bindings (name -> slot index).
    pub uniforms: BTreeMap<String, u8>,
    /// Program label (for debugging).
    pub label: String,
}

impl ShaderProgram {
    /// Create an empty shader program.
    pub fn new(shader_type: ShaderType, label: &str) -> Self {
        Self {
            shader_type,
            instructions: Vec::new(),
            uniforms: BTreeMap::new(),
            label: String::from(label),
        }
    }

    /// Add an instruction.
    pub fn push(&mut self, instr: TgsiInstruction) {
        self.instructions.push(instr);
    }

    /// Bind a uniform name to a slot.
    pub fn bind_uniform(&mut self, name: &str, slot: u8) {
        self.uniforms.insert(String::from(name), slot);
    }

    /// Number of instructions.
    pub fn instruction_count(&self) -> usize {
        self.instructions.len()
    }
}

// ---------------------------------------------------------------------------
// Shader compiler
// ---------------------------------------------------------------------------

/// High-level shader description that gets compiled to TGSI instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaderOp {
    /// Set output colour to a constant.
    SetColor { r: u8, g: u8, b: u8, a: u8 },
    /// Multiply output by a uniform colour.
    TintByUniform { uniform_name: String },
    /// Sample a texture at the fragment coordinate.
    SampleTexture { sampler: u8 },
    /// Apply a simple gradient (top-to-bottom).
    VerticalGradient { top: u32, bottom: u32 },
    /// Passthrough (identity — copy input to output).
    Passthrough,
}

/// Compiles high-level shader descriptions to TGSI instruction lists.
pub struct ShaderCompiler;

impl ShaderCompiler {
    /// Compile a fragment shader from a list of operations.
    pub fn compile_fragment(label: &str, ops: &[ShaderOp]) -> ShaderProgram {
        let mut prog = ShaderProgram::new(ShaderType::Fragment, label);

        for op in ops {
            match op {
                ShaderOp::SetColor { r, g, b, a } => {
                    prog.push(TgsiInstruction::MOV {
                        dst: 0,
                        src: SrcOperand::Immediate(fp_from_int(*r as i32)),
                    });
                    prog.push(TgsiInstruction::MOV {
                        dst: 1,
                        src: SrcOperand::Immediate(fp_from_int(*g as i32)),
                    });
                    prog.push(TgsiInstruction::MOV {
                        dst: 2,
                        src: SrcOperand::Immediate(fp_from_int(*b as i32)),
                    });
                    prog.push(TgsiInstruction::MOV {
                        dst: 3,
                        src: SrcOperand::Immediate(fp_from_int(*a as i32)),
                    });
                }
                ShaderOp::TintByUniform { uniform_name } => {
                    let slot = prog.uniforms.len() as u8;
                    prog.bind_uniform(uniform_name, slot);
                    prog.push(TgsiInstruction::MUL {
                        dst: 0,
                        src0: SrcOperand::Reg(0),
                        src1: SrcOperand::Uniform(slot),
                    });
                }
                ShaderOp::SampleTexture { sampler } => {
                    prog.push(TgsiInstruction::SAMPLE {
                        dst: 0,
                        coord: SrcOperand::Reg(4), // texture coord register
                        sampler: *sampler,
                    });
                }
                ShaderOp::VerticalGradient { top, bottom } => {
                    let tr = fp_from_int(((*top >> 16) & 0xFF) as i32);
                    let br = fp_from_int(((*bottom >> 16) & 0xFF) as i32);
                    // Interpolate: result = top + (bottom - top) * t
                    prog.push(TgsiInstruction::MOV {
                        dst: 0,
                        src: SrcOperand::Immediate(tr),
                    });
                    prog.push(TgsiInstruction::MOV {
                        dst: 5,
                        src: SrcOperand::Immediate(br - tr),
                    });
                    prog.push(TgsiInstruction::MAD {
                        dst: 0,
                        src0: SrcOperand::Reg(5),
                        src1: SrcOperand::Reg(6), // t parameter
                        src2: SrcOperand::Reg(0),
                    });
                }
                ShaderOp::Passthrough => {
                    // No-op: input registers already hold output values
                }
            }
        }

        prog
    }

    /// Compile a simple vertex passthrough shader.
    pub fn compile_vertex_passthrough(label: &str) -> ShaderProgram {
        let mut prog = ShaderProgram::new(ShaderType::Vertex, label);
        // Copy position register to output
        prog.push(TgsiInstruction::MOV {
            dst: 0,
            src: SrcOperand::Reg(0),
        });
        prog
    }
}

// ---------------------------------------------------------------------------
// Shader executor
// ---------------------------------------------------------------------------

/// Maximum number of registers in the virtual register file.
const MAX_REGISTERS: usize = 16;

/// Executes shader programs on pixel data using software rasterisation.
pub struct ShaderExecutor {
    /// Register file for the current invocation.
    registers: [i32; MAX_REGISTERS],
    /// Uniform values bound for the current program.
    uniform_values: Vec<UniformValue>,
}

impl Default for ShaderExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl ShaderExecutor {
    /// Create a new executor with zeroed registers.
    pub fn new() -> Self {
        Self {
            registers: [0; MAX_REGISTERS],
            uniform_values: Vec::new(),
        }
    }

    /// Set a uniform value for the next execution.
    pub fn set_uniform(&mut self, slot: usize, value: UniformValue) {
        while self.uniform_values.len() <= slot {
            self.uniform_values.push(UniformValue::Int(0));
        }
        self.uniform_values[slot] = value;
    }

    /// Set an input register value.
    pub fn set_register(&mut self, reg: u8, value: i32) {
        if (reg as usize) < MAX_REGISTERS {
            self.registers[reg as usize] = value;
        }
    }

    /// Read a register value.
    pub fn get_register(&self, reg: u8) -> i32 {
        if (reg as usize) < MAX_REGISTERS {
            self.registers[reg as usize]
        } else {
            0
        }
    }

    /// Read a source operand value.
    fn read_src(&self, src: &SrcOperand) -> i32 {
        match src {
            SrcOperand::Reg(r) => self.get_register(*r),
            SrcOperand::Uniform(slot) => {
                let s = *slot as usize;
                if s < self.uniform_values.len() {
                    match &self.uniform_values[s] {
                        UniformValue::Int(v) => *v,
                        UniformValue::Vec2(x, _) => *x,
                        UniformValue::Vec4(x, _, _, _) => *x,
                        UniformValue::Mat4(m) => m[0],
                    }
                } else {
                    0
                }
            }
            SrcOperand::Immediate(v) => *v,
        }
    }

    /// Execute a shader program.
    ///
    /// Registers should be pre-loaded with input values.
    /// After execution, output registers contain the results.
    pub fn execute(&mut self, program: &ShaderProgram) {
        for instr in &program.instructions {
            match instr {
                TgsiInstruction::MOV { dst, src } => {
                    let v = self.read_src(src);
                    self.registers[*dst as usize % MAX_REGISTERS] = v;
                }
                TgsiInstruction::ADD { dst, src0, src1 } => {
                    let a = self.read_src(src0);
                    let b = self.read_src(src1);
                    self.registers[*dst as usize % MAX_REGISTERS] = a.saturating_add(b);
                }
                TgsiInstruction::MUL { dst, src0, src1 } => {
                    let a = self.read_src(src0);
                    let b = self.read_src(src1);
                    self.registers[*dst as usize % MAX_REGISTERS] = fp_mul(a, b);
                }
                TgsiInstruction::MAD {
                    dst,
                    src0,
                    src1,
                    src2,
                } => {
                    let a = self.read_src(src0);
                    let b = self.read_src(src1);
                    let c = self.read_src(src2);
                    self.registers[*dst as usize % MAX_REGISTERS] = fp_mul(a, b).saturating_add(c);
                }
                TgsiInstruction::DP3 { dst, src0, src1 } => {
                    // Dot product of first 3 components (uses consecutive registers)
                    let a = self.read_src(src0);
                    let b = self.read_src(src1);
                    self.registers[*dst as usize % MAX_REGISTERS] = fp_mul(a, b);
                }
                TgsiInstruction::DP4 { dst, src0, src1 } => {
                    let a = self.read_src(src0);
                    let b = self.read_src(src1);
                    self.registers[*dst as usize % MAX_REGISTERS] = fp_mul(a, b);
                }
                TgsiInstruction::TEX { dst, coord } => {
                    // Stub: return the coordinate as a grey value
                    let c = self.read_src(coord);
                    self.registers[*dst as usize % MAX_REGISTERS] = c;
                }
                TgsiInstruction::SAMPLE {
                    dst,
                    coord,
                    sampler: _,
                } => {
                    let c = self.read_src(coord);
                    self.registers[*dst as usize % MAX_REGISTERS] = c;
                }
                TgsiInstruction::RCP { dst, src } => {
                    let v = self.read_src(src);
                    self.registers[*dst as usize % MAX_REGISTERS] = fp_rcp(v);
                }
                TgsiInstruction::RSQ { dst, src } => {
                    let v = self.read_src(src);
                    // Integer approximation of 1/sqrt(x)
                    // Using Newton's method with one iteration
                    if v <= 0 {
                        self.registers[*dst as usize % MAX_REGISTERS] = 0;
                    } else {
                        // Rough initial guess
                        let mut guess = FP_ONE;
                        let mut test = fp_to_int(v);
                        while test > 1 {
                            test >>= 2;
                            guess >>= 1;
                        }
                        if guess == 0 {
                            guess = 1;
                        }
                        self.registers[*dst as usize % MAX_REGISTERS] = fp_rcp(guess);
                    }
                }
            }
        }
    }

    /// Execute a fragment shader for a single pixel, returning ARGB8888.
    ///
    /// Input: registers 0-3 are R, G, B, A in 16.16 fixed-point (0..255 range).
    pub fn execute_fragment(&mut self, program: &ShaderProgram) -> u32 {
        self.execute(program);

        let r = fp_to_int(self.registers[0]).clamp(0, 255) as u32;
        let g = fp_to_int(self.registers[1]).clamp(0, 255) as u32;
        let b = fp_to_int(self.registers[2]).clamp(0, 255) as u32;
        let a = fp_to_int(self.registers[3]).clamp(0, 255) as u32;

        (a << 24) | (r << 16) | (g << 8) | b
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fp_mul() {
        let a = fp_from_int(3);
        let b = fp_from_int(4);
        assert_eq!(fp_to_int(fp_mul(a, b)), 12);
    }

    #[test]
    fn test_fp_rcp() {
        let v = fp_from_int(2);
        let r = fp_rcp(v);
        // Should be approximately 0.5 in 16.16 = 32768
        assert!((r - (FP_ONE / 2)).abs() < 2);
    }

    #[test]
    fn test_shader_set_color() {
        let prog = ShaderCompiler::compile_fragment(
            "test",
            &[ShaderOp::SetColor {
                r: 128,
                g: 64,
                b: 32,
                a: 255,
            }],
        );
        assert!(prog.instruction_count() >= 4);
        let mut exec = ShaderExecutor::new();
        let pixel = exec.execute_fragment(&prog);
        let r = (pixel >> 16) & 0xFF;
        let g = (pixel >> 8) & 0xFF;
        let b = pixel & 0xFF;
        assert_eq!(r, 128);
        assert_eq!(g, 64);
        assert_eq!(b, 32);
    }

    #[test]
    fn test_shader_passthrough() {
        let prog = ShaderCompiler::compile_fragment("pass", &[ShaderOp::Passthrough]);
        let mut exec = ShaderExecutor::new();
        exec.set_register(0, fp_from_int(200));
        exec.set_register(1, fp_from_int(100));
        exec.set_register(2, fp_from_int(50));
        exec.set_register(3, fp_from_int(255));
        let pixel = exec.execute_fragment(&prog);
        let r = (pixel >> 16) & 0xFF;
        assert_eq!(r, 200);
    }

    #[test]
    fn test_vertex_passthrough() {
        let prog = ShaderCompiler::compile_vertex_passthrough("vtx");
        assert_eq!(prog.shader_type, ShaderType::Vertex);
        assert_eq!(prog.instruction_count(), 1);
    }

    #[test]
    fn test_mov_instruction() {
        let mut exec = ShaderExecutor::new();
        let mut prog = ShaderProgram::new(ShaderType::Fragment, "test");
        prog.push(TgsiInstruction::MOV {
            dst: 0,
            src: SrcOperand::Immediate(fp_from_int(42)),
        });
        exec.execute(&prog);
        assert_eq!(fp_to_int(exec.get_register(0)), 42);
    }

    #[test]
    fn test_add_instruction() {
        let mut exec = ShaderExecutor::new();
        exec.set_register(0, fp_from_int(10));
        let mut prog = ShaderProgram::new(ShaderType::Fragment, "test");
        prog.push(TgsiInstruction::ADD {
            dst: 1,
            src0: SrcOperand::Reg(0),
            src1: SrcOperand::Immediate(fp_from_int(5)),
        });
        exec.execute(&prog);
        assert_eq!(fp_to_int(exec.get_register(1)), 15);
    }

    #[test]
    fn test_uniform_binding() {
        let mut exec = ShaderExecutor::new();
        exec.set_uniform(0, UniformValue::Int(fp_from_int(2)));
        exec.set_register(0, fp_from_int(100));
        let mut prog = ShaderProgram::new(ShaderType::Fragment, "test");
        prog.bind_uniform("scale", 0);
        prog.push(TgsiInstruction::MUL {
            dst: 0,
            src0: SrcOperand::Reg(0),
            src1: SrcOperand::Uniform(0),
        });
        exec.execute(&prog);
        assert_eq!(fp_to_int(exec.get_register(0)), 200);
    }
}

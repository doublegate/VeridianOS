//! Classic BPF as Linux checks and runs it (`net/core/filter.c`'s
//! bpf_check_classic, `kernel/seccomp.c`'s seccomp_check_filter, and the
//! classic-to-eBPF conversion's semantics), for seccomp filters.
//!
//! The kernel's seccomp and the user-space Linux runtime (Track M, linuxd)
//! share this crate, so a filter means the same thing on both sides of
//! the move. It is `no_std` (with `alloc`) and has no `unsafe`: a checked
//! program runs in bounded time (every jump goes forward) and cannot read
//! outside its input or scratch memory, which the Kani harnesses planned
//! for Track S are to prove.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{vec, vec::Vec};

/// One classic BPF instruction (`struct sock_filter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insn {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

impl Insn {
    /// An instruction from its fields.
    pub const fn new(code: u16, jt: u8, jf: u8, k: u32) -> Self {
        Self { code, jt, jf, k }
    }

    /// The instruction in its 8-byte user layout (code, jt, jf, k; native
    /// endian, as `struct sock_filter` is).
    pub fn from_bytes(b: [u8; 8]) -> Self {
        Self {
            code: u16::from_ne_bytes([b[0], b[1]]),
            jt: b[2],
            jf: b[3],
            k: u32::from_ne_bytes([b[4], b[5], b[6], b[7]]),
        }
    }
}

/// Instruction classes and fields (`include/uapi/linux/bpf_common.h`).
pub mod op {
    pub const LD: u16 = 0x00;
    pub const LDX: u16 = 0x01;
    pub const ST: u16 = 0x02;
    pub const STX: u16 = 0x03;
    pub const ALU: u16 = 0x04;
    pub const JMP: u16 = 0x05;
    pub const RET: u16 = 0x06;
    pub const MISC: u16 = 0x07;

    pub const W: u16 = 0x00;
    pub const H: u16 = 0x08;
    pub const B: u16 = 0x10;

    pub const IMM: u16 = 0x00;
    pub const ABS: u16 = 0x20;
    pub const IND: u16 = 0x40;
    pub const MEM: u16 = 0x60;
    pub const LEN: u16 = 0x80;
    pub const MSH: u16 = 0xa0;

    pub const ADD: u16 = 0x00;
    pub const SUB: u16 = 0x10;
    pub const MUL: u16 = 0x20;
    pub const DIV: u16 = 0x30;
    pub const OR: u16 = 0x40;
    pub const AND: u16 = 0x50;
    pub const LSH: u16 = 0x60;
    pub const RSH: u16 = 0x70;
    pub const NEG: u16 = 0x80;
    pub const MOD: u16 = 0x90;
    pub const XOR: u16 = 0xa0;

    pub const JA: u16 = 0x00;
    pub const JEQ: u16 = 0x10;
    pub const JGT: u16 = 0x20;
    pub const JGE: u16 = 0x30;
    pub const JSET: u16 = 0x40;

    pub const K: u16 = 0x00;
    pub const X: u16 = 0x08;
    pub const A: u16 = 0x10;

    pub const TAX: u16 = 0x00;
    pub const TXA: u16 = 0x80;
}

use op::*;

/// The most instructions one program holds (BPF_MAXINSNS).
pub const MAX_INSNS: usize = 4096;
/// Scratch memory words (BPF_MEMWORDS).
pub const MEM_WORDS: u32 = 16;
/// Where the socket filter's ancillary loads start (SKF_AD_OFF); seccomp
/// has none.
const SKF_AD_OFF: u32 = 0xffff_f000;
/// `struct seccomp_data`'s size: nr, arch, instruction_pointer, args[6].
pub const SECCOMP_DATA_SIZE: u32 = 64;

/// Why a program was refused: always EINVAL to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckError {
    /// Empty or longer than [`MAX_INSNS`].
    Length,
    /// An instruction, operand or jump target the checker refuses.
    Invalid { pc: usize },
    /// The last instruction is not a return.
    NoReturn,
    /// Scratch memory read before any path wrote it.
    UninitializedMemory { pc: usize },
}

/// Whether `code` is a classic BPF instruction at all (chk_code_allowed).
fn code_allowed(code: u16) -> bool {
    matches!(
        code,
        c if c == ALU | ADD | K || c == ALU | ADD | X
            || c == ALU | SUB | K || c == ALU | SUB | X
            || c == ALU | MUL | K || c == ALU | MUL | X
            || c == ALU | DIV | K || c == ALU | DIV | X
            || c == ALU | MOD | K || c == ALU | MOD | X
            || c == ALU | AND | K || c == ALU | AND | X
            || c == ALU | OR | K || c == ALU | OR | X
            || c == ALU | XOR | K || c == ALU | XOR | X
            || c == ALU | LSH | K || c == ALU | LSH | X
            || c == ALU | RSH | K || c == ALU | RSH | X
            || c == ALU | NEG
            || c == LD | W | ABS || c == LD | H | ABS || c == LD | B | ABS
            || c == LD | W | LEN
            || c == LD | W | IND || c == LD | H | IND || c == LD | B | IND
            || c == LD | IMM || c == LD | MEM
            || c == LDX | W | LEN || c == LDX | B | MSH || c == LDX | IMM || c == LDX | MEM
            || c == ST || c == STX
            || c == MISC | TAX || c == MISC | TXA
            || c == RET | K || c == RET | A
            || c == JMP | JA
            || c == JMP | JEQ | K || c == JMP | JEQ | X
            || c == JMP | JGE | K || c == JMP | JGE | X
            || c == JMP | JGT | K || c == JMP | JGT | X
            || c == JMP | JSET | K || c == JMP | JSET | X
    )
}

fn is_cond_jump(code: u16) -> bool {
    code & 0x07 == JMP && code != JMP | JA
}

/// bpf_check_classic: every instruction known, no division by a zero
/// constant, no shift by 32 or more, scratch memory in range, jumps
/// forward and inside the program, a return last, and no scratch word
/// read before every path to the read wrote it.
pub fn check_classic(prog: &[Insn]) -> Result<(), CheckError> {
    let len = prog.len();
    if len == 0 || len > MAX_INSNS {
        return Err(CheckError::Length);
    }
    for (pc, i) in prog.iter().enumerate() {
        let bad = Err(CheckError::Invalid { pc });
        if !code_allowed(i.code) {
            return bad;
        }
        match i.code {
            c if c == ALU | DIV | K || c == ALU | MOD | K => {
                if i.k == 0 {
                    return bad;
                }
            }
            c if c == ALU | LSH | K || c == ALU | RSH | K => {
                if i.k >= 32 {
                    return bad;
                }
            }
            c if c == LD | MEM || c == LDX | MEM || c == ST || c == STX => {
                if i.k >= MEM_WORDS {
                    return bad;
                }
            }
            c if c == JMP | JA => {
                if i.k as usize >= len - pc - 1 {
                    return bad;
                }
            }
            c if is_cond_jump(c) => {
                if pc + i.jt as usize + 1 >= len || pc + i.jf as usize + 1 >= len {
                    return bad;
                }
            }
            c if c == LD | W | ABS || c == LD | H | ABS || c == LD | B | ABS => {
                // No ancillary loads are offered.
                if i.k >= SKF_AD_OFF {
                    return bad;
                }
            }
            _ => {}
        }
    }
    if !matches!(prog[len - 1].code, c if c == RET | K || c == RET | A) {
        return Err(CheckError::NoReturn);
    }
    check_memory(prog)
}

/// check_load_and_stores: a scratch word is read only where every path
/// to the read stored it first.
fn check_memory(prog: &[Insn]) -> Result<(), CheckError> {
    let mut masks = vec![0xffffu16; prog.len()];
    let mut valid: u16 = 0;
    for (pc, i) in prog.iter().enumerate() {
        valid &= masks[pc];
        match i.code {
            c if c == ST || c == STX => valid |= 1 << i.k,
            c if c == LD | MEM || c == LDX | MEM => {
                if valid & (1 << i.k) == 0 {
                    return Err(CheckError::UninitializedMemory { pc });
                }
            }
            c if c == JMP | JA => {
                masks[pc + 1 + i.k as usize] &= valid;
                valid = !0;
            }
            c if is_cond_jump(c) => {
                masks[pc + 1 + i.jt as usize] &= valid;
                masks[pc + 1 + i.jf as usize] &= valid;
                valid = !0;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Linux's private code for seccomp's word load from `struct
/// seccomp_data` (seccomp_check_filter turns LD|W|ABS into it).
const SECCOMP_LOAD: u16 = LDX | W | ABS;

/// A checked seccomp program: [`check_classic`] and then
/// seccomp_check_filter, which allows only 32-bit aligned word loads
/// inside `struct seccomp_data`, makes LEN its size, and refuses the
/// half-word, byte, indirect and MSH loads and MOD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeccompProgram {
    insns: Vec<Insn>,
}

/// Check `prog` as a seccomp filter and return it rewritten for
/// [`run_seccomp`].
pub fn check_seccomp(prog: &[Insn]) -> Result<SeccompProgram, CheckError> {
    check_classic(prog)?;
    let mut out = SeccompProgram {
        insns: Vec::with_capacity(prog.len()),
    };
    for (pc, i) in prog.iter().enumerate() {
        let mut i = *i;
        match i.code {
            c if c == LD | W | ABS => {
                if i.k >= SECCOMP_DATA_SIZE || i.k & 3 != 0 {
                    return Err(CheckError::Invalid { pc });
                }
                i.code = SECCOMP_LOAD;
            }
            c if c == LD | W | LEN => {
                i.code = LD | IMM;
                i.k = SECCOMP_DATA_SIZE;
            }
            c if c == LDX | W | LEN => {
                i.code = LDX | IMM;
                i.k = SECCOMP_DATA_SIZE;
            }
            c if c == RET | K
                || c == RET | A
                || (c & 0x07 == ALU && c & 0xf0 != MOD)
                || c == LD | IMM
                || c == LDX | IMM
                || c == MISC | TAX
                || c == MISC | TXA
                || c == LD | MEM
                || c == LDX | MEM
                || c == ST
                || c == STX
                || c & 0x07 == JMP => {}
            _ => return Err(CheckError::Invalid { pc }),
        }
        out.insns.push(i);
    }
    Ok(out)
}

impl SeccompProgram {
    /// The instructions, as checked and rewritten.
    pub fn insns(&self) -> &[Insn] {
        &self.insns
    }

    /// How many instructions it has (each counts 4 more against the
    /// per-task total, MAX_INSNS_PER_PATH, with the filter overhead).
    pub fn len(&self) -> usize {
        self.insns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.insns.is_empty()
    }
}

/// Run a checked seccomp program on `data` (a `struct seccomp_data` in
/// native byte order) and return its 32-bit action. A division or modulo
/// by an X of zero ends the program with 0, as Linux's conversion does
/// (0 is SECCOMP_RET_KILL_THREAD).
pub fn run_seccomp(prog: &SeccompProgram, data: &[u8; SECCOMP_DATA_SIZE as usize]) -> u32 {
    let insns = prog.insns();
    let (mut a, mut x): (u32, u32) = (0, 0);
    let mut mem = [0u32; MEM_WORDS as usize];
    let mut pc = 0usize;
    while pc < insns.len() {
        let i = insns[pc];
        pc += 1;
        match i.code {
            c if c == SECCOMP_LOAD => {
                let k = i.k as usize;
                a = u32::from_ne_bytes([data[k], data[k + 1], data[k + 2], data[k + 3]]);
            }
            c if c == LD | IMM => a = i.k,
            c if c == LDX | IMM => x = i.k,
            c if c == LD | MEM => a = mem[i.k as usize],
            c if c == LDX | MEM => x = mem[i.k as usize],
            c if c == ST => mem[i.k as usize] = a,
            c if c == STX => mem[i.k as usize] = x,
            c if c == MISC | TAX => x = a,
            c if c == MISC | TXA => a = x,
            c if c == RET | K => return i.k,
            c if c == RET | A => return a,
            c if c == ALU | NEG => a = a.wrapping_neg(),
            c if c & 0x07 == ALU => {
                let v = if c & X != 0 { x } else { i.k };
                a = match c & 0xf0 {
                    ADD => a.wrapping_add(v),
                    SUB => a.wrapping_sub(v),
                    MUL => a.wrapping_mul(v),
                    DIV => {
                        if v == 0 {
                            return 0;
                        }
                        a / v
                    }
                    MOD => {
                        if v == 0 {
                            return 0;
                        }
                        a % v
                    }
                    AND => a & v,
                    OR => a | v,
                    XOR => a ^ v,
                    // A shift by X of 32 or more is 0 (eBPF masks it).
                    LSH => a.checked_shl(v).unwrap_or(0),
                    RSH => a.checked_shr(v).unwrap_or(0),
                    _ => return 0,
                };
            }
            c if c == JMP | JA => pc += i.k as usize,
            c if c & 0x07 == JMP => {
                let v = if c & X != 0 { x } else { i.k };
                let taken = match c & 0xf0 {
                    JEQ => a == v,
                    JGT => a > v,
                    JGE => a >= v,
                    JSET => a & v != 0,
                    _ => false,
                };
                pc += if taken { i.jt } else { i.jf } as usize;
            }
            _ => return 0,
        }
    }
    // A checked program ends with a return; nothing reaches here.
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALLOW: u32 = 0x7fff_0000;
    const KILL: u32 = 0;

    fn data(nr: u32, arch: u32, args: [u64; 6]) -> [u8; 64] {
        let mut d = [0u8; 64];
        d[0..4].copy_from_slice(&nr.to_ne_bytes());
        d[4..8].copy_from_slice(&arch.to_ne_bytes());
        for (n, a) in args.iter().enumerate() {
            d[16 + 8 * n..24 + 8 * n].copy_from_slice(&a.to_ne_bytes());
        }
        d
    }

    /// The usual libseccomp shape: check the architecture, then the
    /// system call number.
    fn deny_getpid() -> [Insn; 6] {
        [
            Insn::new(LD | W | ABS, 0, 0, 4),
            Insn::new(JMP | JEQ | K, 1, 0, 0xc000_003e),
            Insn::new(RET | K, 0, 0, KILL),
            Insn::new(LD | W | ABS, 0, 0, 0),
            Insn::new(JMP | JEQ | K, 0, 1, 39),
            Insn::new(RET | K, 0, 0, 0x0005_0001), // ERRNO(EPERM)
        ]
    }

    #[test]
    fn a_filter_runs_as_linux_runs_it() {
        let mut p = deny_getpid().to_vec();
        p.push(Insn::new(RET | K, 0, 0, ALLOW));
        let prog = check_seccomp(&p).unwrap();
        assert_eq!(
            run_seccomp(&prog, &data(39, 0xc000_003e, [0; 6])),
            0x0005_0001
        );
        assert_eq!(run_seccomp(&prog, &data(60, 0xc000_003e, [0; 6])), ALLOW);
        assert_eq!(run_seccomp(&prog, &data(39, 0x4000_00b7, [0; 6])), KILL);
    }

    #[test]
    fn the_checker_refuses_what_linux_refuses() {
        let ret = Insn::new(RET | K, 0, 0, ALLOW);
        assert_eq!(check_seccomp(&[]), Err(CheckError::Length));
        // A jump past the end, a constant division by zero, a shift of 32.
        assert!(check_seccomp(&[Insn::new(JMP | JA, 0, 0, 1), ret]).is_err());
        assert!(check_seccomp(&[Insn::new(ALU | DIV | K, 0, 0, 0), ret]).is_err());
        assert!(check_seccomp(&[Insn::new(ALU | LSH | K, 0, 0, 32), ret]).is_err());
        // Unaligned, past the data, a byte load, MOD (seccomp's own rules).
        assert!(check_seccomp(&[Insn::new(LD | W | ABS, 0, 0, 2), ret]).is_err());
        assert!(check_seccomp(&[Insn::new(LD | W | ABS, 0, 0, 64), ret]).is_err());
        assert!(check_seccomp(&[Insn::new(LD | B | ABS, 0, 0, 0), ret]).is_err());
        assert!(check_seccomp(&[Insn::new(ALU | MOD | K, 0, 0, 3), ret]).is_err());
        // No return at the end; an unknown code.
        assert_eq!(
            check_seccomp(&[Insn::new(LD | IMM, 0, 0, 1)]),
            Err(CheckError::NoReturn)
        );
        assert!(check_seccomp(&[Insn::new(0xffff, 0, 0, 0), ret]).is_err());
        // Scratch memory read on a path that did not store it.
        assert_eq!(
            check_seccomp(&[
                Insn::new(LD | IMM, 0, 0, 1),
                Insn::new(JMP | JEQ | K, 0, 1, 1),
                Insn::new(ST, 0, 0, 0),
                Insn::new(LD | MEM, 0, 0, 0),
                ret,
            ]),
            Err(CheckError::UninitializedMemory { pc: 3 })
        );
    }

    #[test]
    fn arithmetic_and_memory_are_classic_bpfs() {
        let prog = check_seccomp(&[
            Insn::new(LD | W | LEN, 0, 0, 0),
            Insn::new(ST, 0, 0, 3),
            Insn::new(LDX | IMM, 0, 0, 0),
            Insn::new(LD | MEM, 0, 0, 3),
            Insn::new(ALU | DIV | X, 0, 0, 0),
            Insn::new(RET | A, 0, 0, 0),
        ])
        .unwrap();
        // LEN is the data's size; dividing by an X of 0 ends with 0.
        assert_eq!(run_seccomp(&prog, &[0; 64]), 0);
        let prog = check_seccomp(&[
            Insn::new(LD | W | LEN, 0, 0, 0),
            Insn::new(ALU | RSH | K, 0, 0, 2),
            Insn::new(MISC | TAX, 0, 0, 0),
            Insn::new(LD | W | ABS, 0, 0, 16),
            Insn::new(ALU | ADD | X, 0, 0, 0),
            Insn::new(JMP | JSET | K, 0, 1, 1),
            Insn::new(RET | K, 0, 0, 7),
            Insn::new(RET | A, 0, 0, 0),
        ])
        .unwrap();
        // args[0]'s low word 5 + 64/4 = 21: odd, JSET 1 taken.
        assert_eq!(run_seccomp(&prog, &data(0, 0, [5, 0, 0, 0, 0, 0])), 7);
        assert_eq!(run_seccomp(&prog, &data(0, 0, [6, 0, 0, 0, 0, 0])), 22);
    }
}

//! Seccomp BPF - filter instructions, syscall filtering, arg inspection,
//! inheritance.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::error::KernelError;

/// BPF instruction opcodes for seccomp filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum BpfOpcode {
    /// Load word at absolute offset.
    LdAbsW = 0x20,
    /// Load half-word at absolute offset.
    LdAbsH = 0x28,
    /// Load byte at absolute offset.
    LdAbsB = 0x30,
    /// Jump if equal (immediate).
    JmpJeqK = 0x15,
    /// Jump if greater or equal (immediate).
    JmpJgeK = 0x35,
    /// Jump if set (bitwise AND, immediate).
    JmpJsetK = 0x45,
    /// Unconditional jump.
    JmpJa = 0x05,
    /// Return (action).
    Ret = 0x06,
    /// ALU AND (immediate).
    AluAndK = 0x54,
}

/// Seccomp return action values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SeccompAction {
    /// Allow the syscall.
    Allow = 0x7fff_0000,
    /// Kill the thread.
    KillThread = 0x0000_0000,
    /// Kill the process.
    KillProcess = 0x8000_0000,
    /// Trigger a SIGSYS and deliver a signal.
    Trap = 0x0003_0000,
    /// Return an errno value (low 16 bits).
    Errno = 0x0005_0000,
    /// Notify a tracing process.
    Trace = 0x7ff0_0000,
    /// Log the syscall and allow it.
    Log = 0x7ffc_0000,
}

impl SeccompAction {
    /// Create an Errno action with a specific errno value.
    pub fn errno(errno: u16) -> u32 {
        Self::Errno as u32 | (errno as u32)
    }
}

/// A single BPF instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BpfInstruction {
    /// Opcode.
    pub code: u16,
    /// Jump target if condition is true.
    pub jt: u8,
    /// Jump target if condition is false.
    pub jf: u8,
    /// Immediate value.
    pub k: u32,
}

impl BpfInstruction {
    /// Create a load-word instruction at the given offset.
    pub fn load_word(offset: u32) -> Self {
        Self {
            code: BpfOpcode::LdAbsW as u16,
            jt: 0,
            jf: 0,
            k: offset,
        }
    }

    /// Create a jump-if-equal instruction.
    pub fn jump_eq(value: u32, jt: u8, jf: u8) -> Self {
        Self {
            code: BpfOpcode::JmpJeqK as u16,
            jt,
            jf,
            k: value,
        }
    }

    /// Create a jump-if-greater-or-equal instruction.
    pub fn jump_ge(value: u32, jt: u8, jf: u8) -> Self {
        Self {
            code: BpfOpcode::JmpJgeK as u16,
            jt,
            jf,
            k: value,
        }
    }

    /// Create a bitwise AND test (jump if set) instruction.
    pub fn jump_set(mask: u32, jt: u8, jf: u8) -> Self {
        Self {
            code: BpfOpcode::JmpJsetK as u16,
            jt,
            jf,
            k: mask,
        }
    }

    /// Create an unconditional jump.
    pub fn jump(offset: u32) -> Self {
        Self {
            code: BpfOpcode::JmpJa as u16,
            jt: 0,
            jf: 0,
            k: offset,
        }
    }

    /// Create a return instruction.
    pub fn ret(action: u32) -> Self {
        Self {
            code: BpfOpcode::Ret as u16,
            jt: 0,
            jf: 0,
            k: action,
        }
    }

    /// Create an ALU AND instruction.
    pub fn alu_and(mask: u32) -> Self {
        Self {
            code: BpfOpcode::AluAndK as u16,
            jt: 0,
            jf: 0,
            k: mask,
        }
    }
}

/// Seccomp data offsets (for x86_64 struct seccomp_data layout).
pub mod seccomp_offsets {
    /// Offset of syscall number (nr field).
    pub const NR: u32 = 0;
    /// Offset of architecture (arch field).
    pub const ARCH: u32 = 4;
    /// Offset of instruction pointer (instruction_pointer field).
    pub const IP_LO: u32 = 8;
    pub const IP_HI: u32 = 12;
    /// Offset of syscall arguments (args[0..5]).
    pub const ARG0_LO: u32 = 16;
    pub const ARG0_HI: u32 = 20;
    pub const ARG1_LO: u32 = 24;
    pub const ARG1_HI: u32 = 28;
    pub const ARG2_LO: u32 = 32;
    pub const ARG2_HI: u32 = 36;
    pub const ARG3_LO: u32 = 40;
    pub const ARG3_HI: u32 = 44;
    pub const ARG4_LO: u32 = 48;
    pub const ARG4_HI: u32 = 52;
    pub const ARG5_LO: u32 = 56;
    pub const ARG5_HI: u32 = 60;
}

/// Audit architecture values.
pub mod audit_arch {
    pub const X86_64: u32 = 0xC000_003E;
    pub const AARCH64: u32 = 0xC000_00B7;
    pub const RISCV64: u32 = 0xC000_00F3;
}

/// Seccomp operating modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeccompMode {
    /// No filtering (disabled).
    Disabled,
    /// Strict mode: only read, write, exit, sigreturn allowed.
    Strict,
    /// Filter mode: BPF program decides.
    Filter,
}

/// A seccomp BPF filter program.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct SeccompFilter {
    /// BPF instructions.
    pub instructions: Vec<BpfInstruction>,
    /// Whether this filter should be inherited on fork.
    pub inherit_on_fork: bool,
    /// Filter ID for tracking.
    pub filter_id: u64,
}

static NEXT_FILTER_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "alloc")]
impl SeccompFilter {
    /// Create a new empty filter.
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            inherit_on_fork: true,
            filter_id: NEXT_FILTER_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Add an instruction to the filter.
    pub fn push(&mut self, insn: BpfInstruction) {
        self.instructions.push(insn);
    }

    /// Get the number of instructions.
    pub fn len(&self) -> usize {
        self.instructions.len()
    }

    /// Check if the filter is empty.
    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    /// Validate the filter program.
    pub fn validate(&self) -> Result<(), KernelError> {
        if self.instructions.is_empty() {
            return Err(KernelError::InvalidArgument {
                name: "seccomp filter",
                value: "empty program",
            });
        }
        // Max 4096 instructions (Linux limit)
        if self.instructions.len() > 4096 {
            return Err(KernelError::InvalidArgument {
                name: "seccomp filter",
                value: "exceeds 4096 instructions",
            });
        }
        // Last instruction must be a return
        if let Some(last) = self.instructions.last() {
            if last.code != BpfOpcode::Ret as u16 {
                return Err(KernelError::InvalidArgument {
                    name: "seccomp filter",
                    value: "must end with RET",
                });
            }
        }
        // Validate jump targets
        let len = self.instructions.len();
        for (i, insn) in self.instructions.iter().enumerate() {
            let code = insn.code;
            if code == BpfOpcode::JmpJeqK as u16
                || code == BpfOpcode::JmpJgeK as u16
                || code == BpfOpcode::JmpJsetK as u16
            {
                let jt_target = i + 1 + insn.jt as usize;
                let jf_target = i + 1 + insn.jf as usize;
                if jt_target >= len || jf_target >= len {
                    return Err(KernelError::InvalidArgument {
                        name: "seccomp filter",
                        value: "jump target out of bounds",
                    });
                }
            }
            if code == BpfOpcode::JmpJa as u16 {
                let target = i + 1 + insn.k as usize;
                if target >= len {
                    return Err(KernelError::InvalidArgument {
                        name: "seccomp filter",
                        value: "jump target out of bounds",
                    });
                }
            }
        }
        Ok(())
    }

    /// Execute the filter against a seccomp_data structure.
    /// Returns the action (SeccompAction value | errno).
    pub fn evaluate(&self, data: &SeccompData) -> u32 {
        let mut accumulator: u32 = 0;
        let mut pc: usize = 0;
        let data_bytes = data.as_bytes();

        while pc < self.instructions.len() {
            let insn = &self.instructions[pc];
            match insn.code {
                c if c == BpfOpcode::LdAbsW as u16 => {
                    let off = insn.k as usize;
                    if off + 4 <= data_bytes.len() {
                        accumulator = u32::from_ne_bytes([
                            data_bytes[off],
                            data_bytes[off + 1],
                            data_bytes[off + 2],
                            data_bytes[off + 3],
                        ]);
                    }
                    pc += 1;
                }
                c if c == BpfOpcode::LdAbsH as u16 => {
                    let off = insn.k as usize;
                    if off + 2 <= data_bytes.len() {
                        accumulator =
                            u16::from_ne_bytes([data_bytes[off], data_bytes[off + 1]]) as u32;
                    }
                    pc += 1;
                }
                c if c == BpfOpcode::LdAbsB as u16 => {
                    let off = insn.k as usize;
                    if off < data_bytes.len() {
                        accumulator = data_bytes[off] as u32;
                    }
                    pc += 1;
                }
                c if c == BpfOpcode::JmpJeqK as u16 => {
                    if accumulator == insn.k {
                        pc += 1 + insn.jt as usize;
                    } else {
                        pc += 1 + insn.jf as usize;
                    }
                }
                c if c == BpfOpcode::JmpJgeK as u16 => {
                    if accumulator >= insn.k {
                        pc += 1 + insn.jt as usize;
                    } else {
                        pc += 1 + insn.jf as usize;
                    }
                }
                c if c == BpfOpcode::JmpJsetK as u16 => {
                    if accumulator & insn.k != 0 {
                        pc += 1 + insn.jt as usize;
                    } else {
                        pc += 1 + insn.jf as usize;
                    }
                }
                c if c == BpfOpcode::JmpJa as u16 => {
                    pc += 1 + insn.k as usize;
                }
                c if c == BpfOpcode::Ret as u16 => {
                    return insn.k;
                }
                c if c == BpfOpcode::AluAndK as u16 => {
                    accumulator &= insn.k;
                    pc += 1;
                }
                _ => {
                    // Unknown opcode: kill
                    return SeccompAction::KillThread as u32;
                }
            }

            // Safety: prevent infinite loops
            if pc >= self.instructions.len() {
                return SeccompAction::KillThread as u32;
            }
        }

        SeccompAction::KillThread as u32
    }

    /// Build a filter that checks architecture and denies a set of syscall
    /// numbers.
    pub fn deny_syscalls(arch: u32, denied: &[u32], errno_val: u16) -> Self {
        let mut filter = Self::new();
        let num_denied = denied.len();

        // Load architecture
        filter.push(BpfInstruction::load_word(seccomp_offsets::ARCH));
        // If arch doesn't match, kill
        filter.push(BpfInstruction::jump_eq(arch, 1, 0));
        filter.push(BpfInstruction::ret(SeccompAction::KillProcess as u32));

        // Load syscall number
        filter.push(BpfInstruction::load_word(seccomp_offsets::NR));

        // For each denied syscall, check and return errno
        for (i, &nr) in denied.iter().enumerate() {
            let remaining = num_denied - i - 1;
            // jt = jump to errno return (which is at the end of deny checks)
            // jf = check next deny or fall through to allow
            // jt must skip remaining deny checks + the allow return to reach errno return
            let jt = (remaining as u8).saturating_add(1);
            filter.push(BpfInstruction::jump_eq(nr, jt, 0));
        }

        // Default: allow
        filter.push(BpfInstruction::ret(SeccompAction::Allow as u32));

        // Errno return
        filter.push(BpfInstruction::ret(SeccompAction::errno(errno_val)));

        filter
    }

    /// Build a filter that only allows a whitelist of syscalls.
    pub fn allow_syscalls(arch: u32, allowed: &[u32]) -> Self {
        let mut filter = Self::new();
        let num_allowed = allowed.len();

        // Load architecture
        filter.push(BpfInstruction::load_word(seccomp_offsets::ARCH));
        filter.push(BpfInstruction::jump_eq(arch, 1, 0));
        filter.push(BpfInstruction::ret(SeccompAction::KillProcess as u32));

        // Load syscall number
        filter.push(BpfInstruction::load_word(seccomp_offsets::NR));

        // For each allowed syscall, jump to allow
        for (i, &nr) in allowed.iter().enumerate() {
            let remaining = num_allowed - i - 1;
            // jt = jump to allow (which is `remaining` checks + 1 kill instruction away)
            let jt = (remaining as u8).saturating_add(1);
            filter.push(BpfInstruction::jump_eq(nr, jt, 0));
        }

        // Default: kill
        filter.push(BpfInstruction::ret(SeccompAction::KillProcess as u32));

        // Allow return
        filter.push(BpfInstruction::ret(SeccompAction::Allow as u32));

        filter
    }
}

#[cfg(feature = "alloc")]
impl Default for SeccompFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Seccomp data structure matching the kernel's struct seccomp_data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SeccompData {
    /// Syscall number.
    pub nr: u32,
    /// Architecture (AUDIT_ARCH_*).
    pub arch: u32,
    /// Instruction pointer.
    pub instruction_pointer: u64,
    /// Syscall arguments (up to 6).
    pub args: [u64; 6],
}

impl SeccompData {
    pub fn new(nr: u32, arch: u32, args: [u64; 6]) -> Self {
        Self {
            nr,
            arch,
            instruction_pointer: 0,
            args,
        }
    }

    /// Convert to a byte representation for BPF evaluation.
    pub fn as_bytes(&self) -> [u8; 64] {
        let mut buf = [0u8; 64];
        // nr at offset 0
        buf[0..4].copy_from_slice(&self.nr.to_ne_bytes());
        // arch at offset 4
        buf[4..8].copy_from_slice(&self.arch.to_ne_bytes());
        // instruction_pointer at offset 8
        buf[8..16].copy_from_slice(&self.instruction_pointer.to_ne_bytes());
        // args at offset 16
        for (i, &arg) in self.args.iter().enumerate() {
            let off = 16 + i * 8;
            buf[off..off + 8].copy_from_slice(&arg.to_ne_bytes());
        }
        buf
    }
}

/// Per-process seccomp state.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone)]
pub struct SeccompState {
    /// Current mode.
    pub mode: SeccompMode,
    /// Stack of filters (all evaluated, most restrictive wins).
    pub filters: Vec<SeccompFilter>,
}

#[cfg(feature = "alloc")]
impl SeccompState {
    pub fn new() -> Self {
        Self {
            mode: SeccompMode::Disabled,
            filters: Vec::new(),
        }
    }

    /// Install a new filter. Mode transitions to Filter.
    pub fn install_filter(&mut self, filter: SeccompFilter) -> Result<(), KernelError> {
        filter.validate()?;
        self.mode = SeccompMode::Filter;
        self.filters.push(filter);
        Ok(())
    }

    /// Evaluate all filters against the given syscall data.
    /// Returns the most restrictive action (lowest value wins per Linux
    /// semantics).
    pub fn evaluate(&self, data: &SeccompData) -> u32 {
        match self.mode {
            SeccompMode::Disabled => SeccompAction::Allow as u32,
            SeccompMode::Strict => {
                // Only allow read(0), write(1), exit(60), sigreturn(15)
                match data.nr {
                    0 | 1 | 15 | 60 => SeccompAction::Allow as u32,
                    _ => SeccompAction::KillThread as u32,
                }
            }
            SeccompMode::Filter => {
                let mut result = SeccompAction::Allow as u32;
                for filter in &self.filters {
                    let action = filter.evaluate(data);
                    // Most restrictive wins (lower value = more restrictive)
                    if action < result {
                        result = action;
                    }
                }
                result
            }
        }
    }

    /// Create a copy for a forked process (inherits filters marked for
    /// inheritance).
    pub fn fork_inherit(&self) -> Self {
        Self {
            mode: self.mode,
            filters: self
                .filters
                .iter()
                .filter(|f| f.inherit_on_fork)
                .cloned()
                .collect(),
        }
    }

    pub fn filter_count(&self) -> usize {
        self.filters.len()
    }
}

#[cfg(feature = "alloc")]
impl Default for SeccompState {
    fn default() -> Self {
        Self::new()
    }
}

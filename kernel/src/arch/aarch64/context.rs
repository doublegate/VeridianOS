//! AArch64 context switching implementation

use core::arch::asm;

/// AArch64 CPU context
#[repr(C)]
#[derive(Debug, Clone)]
pub struct AArch64Context {
    /// General purpose registers (x0-x30)
    pub x: [u64; 31],

    /// Stack pointer
    pub sp: u64,

    /// Program counter
    pub pc: u64,

    /// Saved program status register
    pub spsr: u64,

    /// Exception link register
    pub elr: u64,

    /// Thread pointer registers
    pub tpidr_el0: u64,
    pub tpidr_el1: u64,
    /// Saved TLS base for EL0
    pub tls_base: u64,

    /// Translation table base register
    pub ttbr0_el1: u64,

    /// Floating point registers
    pub fp_regs: FpuState,
}

/// AArch64 FPU state (NEON/SVE)
#[repr(C, align(16))]
#[derive(Debug, Clone)]
pub struct FpuState {
    /// SIMD&FP registers (v0-v31)
    pub v: [[u64; 2]; 32],
    /// Floating-point control register
    pub fpcr: u32,
    /// Floating-point status register
    pub fpsr: u32,
}

impl AArch64Context {
    /// Take the user registers of `frame` (a live exception frame) and the
    /// TLS register `tls`: a fork or clone child resumes where its parent's
    /// system call returns to, not at the program's entry.
    pub fn load_user_frame(&mut self, frame: &crate::arch::aarch64::switch::UserFrame, tls: u64) {
        self.x = frame.x;
        self.sp = frame.sp;
        self.pc = frame.elr;
        self.elr = frame.elr;
        self.tpidr_el0 = tls;
        self.tls_base = tls;
    }

    /// Create new context for a task
    pub fn new(entry_point: usize, stack_pointer: usize) -> Self {
        Self {
            // Clear all general purpose registers
            x: [0; 31],

            // Set stack pointer
            sp: stack_pointer as u64,

            // Set program counter to entry point
            pc: entry_point as u64,

            // Default SPSR for EL1 (interrupts enabled)
            spsr: 0x3c5, // EL1h, DAIF clear

            // Exception link register
            elr: entry_point as u64,

            // Thread pointers
            tpidr_el0: 0,
            tpidr_el1: 0,
            tls_base: 0,

            // Will be set to actual page table
            ttbr0_el1: 0,

            // Clear FPU state
            fp_regs: FpuState {
                v: [[0; 2]; 32],
                fpcr: 0,
                fpsr: 0,
            },
        }
    }
}

impl Default for AArch64Context {
    fn default() -> Self {
        Self {
            x: [0; 31],
            sp: 0,
            pc: 0,
            spsr: 0x3c5,
            elr: 0,
            tpidr_el0: 0,
            tpidr_el1: 0,
            tls_base: 0,
            ttbr0_el1: 0,
            fp_regs: FpuState {
                v: [[0; 2]; 32],
                fpcr: 0,
                fpsr: 0,
            },
        }
    }
}

impl crate::arch::context::ThreadContext for AArch64Context {
    fn new() -> Self {
        Self::default()
    }

    fn init(&mut self, entry_point: usize, stack_pointer: usize, _kernel_stack: usize) {
        self.pc = entry_point as u64;
        self.elr = entry_point as u64;
        self.sp = stack_pointer as u64;
    }

    fn get_instruction_pointer(&self) -> usize {
        self.pc as usize
    }

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.pc = ip as u64;
        self.elr = ip as u64;
    }

    fn get_stack_pointer(&self) -> usize {
        self.sp as usize
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.sp = sp as u64;
    }

    fn get_kernel_stack(&self) -> usize {
        // Kernel stack is stored in TPIDR_EL1
        self.tpidr_el1 as usize
    }

    fn set_kernel_stack(&mut self, sp: usize) {
        // Store kernel stack in TPIDR_EL1 for quick access
        self.tpidr_el1 = sp as u64;
    }

    fn set_return_value(&mut self, value: usize) {
        self.x[0] = value as u64; // x0 is return register
    }

    fn set_tls_base(&mut self, base: u64) {
        self.tls_base = base;
        self.tpidr_el0 = base;
    }

    fn tls_base(&self) -> u64 {
        self.tls_base
    }

    fn clone_from(&mut self, other: &Self) {
        *self = other.clone();
    }
}

/// Initialize FPU for current CPU
pub fn init_fpu() {
    // SAFETY: Sets the FPEN field in CPACR_EL1 to enable FPU/SIMD access from
    // EL1 (kernel mode). The ISB ensures the change takes effect before any
    // subsequent FPU instructions are executed. Always available in EL1.
    unsafe {
        // Enable FPU access from EL1
        asm!(
            "mrs x0, CPACR_EL1",
            "orr x0, x0, #(0x3 << 20)",  // FPEN = 11
            "msr CPACR_EL1, x0",
            "isb",
            out("x0") _,
        );
    }
}

/// The FP/SIMD state a new program starts with: every register zero, FPCR
/// zero (round to nearest, no traps, no flush-to-zero), FPSR zero.
pub fn initial_fpu() -> alloc::boxed::Box<FpuState> {
    alloc::boxed::Box::new(FpuState {
        v: [[0; 2]; 32],
        fpcr: 0,
        fpsr: 0,
    })
}

/// Save FPU/NEON state to the given FpuState buffer
///
/// Saves all 32 SIMD registers (Q0-Q31) plus FPCR and FPSR.
pub fn save_fpu_state(state: &mut FpuState) {
    // SAFETY: Saves all NEON/FP registers to the provided FpuState structure.
    // The structure is repr(C, align(16)) ensuring proper alignment for STP.
    // FPCR and FPSR are system registers read via MRS.
    unsafe {
        let base = state.v.as_mut_ptr() as *mut u64;
        asm!(
            // The kernel is built without FP/SIMD (N-178); only this
            // save/restore of user state uses those registers.
            ".arch_extension fp",
            "stp q0,  q1,  [{base}]",
            "stp q2,  q3,  [{base}, #32]",
            "stp q4,  q5,  [{base}, #64]",
            "stp q6,  q7,  [{base}, #96]",
            "stp q8,  q9,  [{base}, #128]",
            "stp q10, q11, [{base}, #160]",
            "stp q12, q13, [{base}, #192]",
            "stp q14, q15, [{base}, #224]",
            "stp q16, q17, [{base}, #256]",
            "stp q18, q19, [{base}, #288]",
            "stp q20, q21, [{base}, #320]",
            "stp q22, q23, [{base}, #352]",
            "stp q24, q25, [{base}, #384]",
            "stp q26, q27, [{base}, #416]",
            "stp q28, q29, [{base}, #448]",
            "stp q30, q31, [{base}, #480]",
            base = in(reg) base,
        );
        let mut fpcr: u32;
        let mut fpsr: u32;
        asm!(".arch_extension fp", "mrs {fpcr:x}, FPCR", fpcr = out(reg) fpcr);
        asm!(".arch_extension fp", "mrs {fpsr:x}, FPSR", fpsr = out(reg) fpsr);
        state.fpcr = fpcr;
        state.fpsr = fpsr;
    }
}

/// Restore FPU/NEON state from the given FpuState buffer
///
/// Restores all 32 SIMD registers (Q0-Q31) plus FPCR and FPSR.
pub fn restore_fpu_state(state: &FpuState) {
    // SAFETY: Restores all NEON/FP registers from the provided FpuState structure.
    // The structure is repr(C, align(16)) ensuring proper alignment for LDP.
    // FPCR and FPSR are system registers written via MSR.
    unsafe {
        let fpcr = state.fpcr;
        let fpsr = state.fpsr;
        asm!(".arch_extension fp", "msr FPCR, {fpcr:x}", fpcr = in(reg) fpcr);
        asm!(".arch_extension fp", "msr FPSR, {fpsr:x}", fpsr = in(reg) fpsr);
        let base = state.v.as_ptr() as *const u64;
        asm!(
            // The kernel is built without FP/SIMD (N-178); only this
            // save/restore of user state uses those registers.
            ".arch_extension fp",
            "ldp q0,  q1,  [{base}]",
            "ldp q2,  q3,  [{base}, #32]",
            "ldp q4,  q5,  [{base}, #64]",
            "ldp q6,  q7,  [{base}, #96]",
            "ldp q8,  q9,  [{base}, #128]",
            "ldp q10, q11, [{base}, #160]",
            "ldp q12, q13, [{base}, #192]",
            "ldp q14, q15, [{base}, #224]",
            "ldp q16, q17, [{base}, #256]",
            "ldp q18, q19, [{base}, #288]",
            "ldp q20, q21, [{base}, #320]",
            "ldp q22, q23, [{base}, #352]",
            "ldp q24, q25, [{base}, #384]",
            "ldp q26, q27, [{base}, #416]",
            "ldp q28, q29, [{base}, #448]",
            "ldp q30, q31, [{base}, #480]",
            base = in(reg) base,
        );
    }
}

/// Check if CPU supports SVE
#[allow(dead_code)] // CPU feature detection API
pub fn has_sve() -> bool {
    // SAFETY: Reading ID_AA64PFR0_EL1 is a read-only operation that reports
    // processor feature information. Bits [35:32] indicate SVE support.
    unsafe {
        let mut id_aa64pfr0: u64;
        asm!("mrs {}, ID_AA64PFR0_EL1", out(reg) id_aa64pfr0);
        ((id_aa64pfr0 >> 32) & 0xF) != 0
    }
}

/// Enable SVE if supported
#[allow(dead_code)] // CPU feature detection API
pub fn enable_sve() {
    if has_sve() {
        // SAFETY: Sets the ZEN field in CPACR_EL1 to enable SVE access from EL1.
        // Only called after has_sve() confirms SVE support. The ISB ensures the
        // change takes effect before any subsequent SVE instructions.
        unsafe {
            asm!(
                "mrs x0, CPACR_EL1",
                "orr x0, x0, #(0x3 << 16)",  // ZEN = 11
                "msr CPACR_EL1, x0",
                "isb",
                out("x0") _,
            );
        }
    }
}

/// Get current exception level
#[allow(dead_code)] // CPU feature detection API
pub fn current_el() -> u8 {
    // SAFETY: Reading CurrentEL is a read-only operation that returns the
    // current exception level. Bits [3:2] encode the EL (0-3). Always accessible.
    unsafe {
        let mut current_el: u64;
        asm!("mrs {}, CurrentEL", out(reg) current_el);
        ((current_el >> 2) & 0x3) as u8
    }
}

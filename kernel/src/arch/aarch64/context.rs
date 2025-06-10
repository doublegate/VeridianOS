//! AArch64 context switching implementation

use core::arch::asm;

use crate::sched::task::TaskContext;

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

impl crate::arch::context::ThreadContext for AArch64Context {
    fn new() -> Self {
        Self {
            x: [0; 31],
            sp: 0,
            pc: 0,
            spsr: 0x3c5,
            elr: 0,
            tpidr_el0: 0,
            tpidr_el1: 0,
            ttbr0_el1: 0,
            fp_regs: FpuState {
                v: [[0; 2]; 32],
                fpcr: 0,
                fpsr: 0,
            },
        }
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

    fn clone_from(&mut self, other: &Self) {
        *self = other.clone();
    }

    fn to_task_context(&self) -> TaskContext {
        TaskContext::AArch64(self.clone())
    }
}

/// Switch context using the ThreadContext interface
pub fn switch_context(from: &mut AArch64Context, to: &AArch64Context) {
    unsafe {
        context_switch(from as *mut _, to as *const _);
    }
}

/// Switch from current context to new context
///
/// # Safety
/// This function manipulates CPU state directly and must be called
/// with interrupts disabled.
#[naked]
#[no_mangle]
pub unsafe extern "C" fn context_switch(
    _current: *mut AArch64Context,
    _next: *const AArch64Context,
) {
    asm!(
        // Save current context
        // x0 = current context pointer
        // x1 = next context pointer

        // Save general purpose registers x2-x30 (x0-x1 are parameters)
        "stp x2, x3, [x0, #16]",
        "stp x4, x5, [x0, #32]",
        "stp x6, x7, [x0, #48]",
        "stp x8, x9, [x0, #64]",
        "stp x10, x11, [x0, #80]",
        "stp x12, x13, [x0, #96]",
        "stp x14, x15, [x0, #112]",
        "stp x16, x17, [x0, #128]",
        "stp x18, x19, [x0, #144]",
        "stp x20, x21, [x0, #160]",
        "stp x22, x23, [x0, #176]",
        "stp x24, x25, [x0, #192]",
        "stp x26, x27, [x0, #208]",
        "stp x28, x29, [x0, #224]",
        "str x30, [x0, #240]", // x30 is link register
        // Save stack pointer
        "mov x2, sp",
        "str x2, [x0, #248]",
        // Save return address as PC
        "str x30, [x0, #256]",
        // Save SPSR and ELR
        "mrs x2, SPSR_EL1",
        "mrs x3, ELR_EL1",
        "stp x2, x3, [x0, #264]",
        // Save thread pointers
        "mrs x2, TPIDR_EL0",
        "mrs x3, TPIDR_EL1",
        "stp x2, x3, [x0, #280]",
        // Save translation table base
        "mrs x2, TTBR0_EL1",
        "str x2, [x0, #296]",
        // Load new context
        // Load TTBR0_EL1 first (if different)
        "ldr x3, [x1, #296]", // New TTBR0
        "cmp x2, x3",         // Compare with current
        "b.eq 1f",            // Skip if same
        "msr TTBR0_EL1, x3",  // Set new page table
        "isb",                // Ensure completion
        "1:",
        // Load thread pointers
        "ldp x2, x3, [x1, #280]",
        "msr TPIDR_EL0, x2",
        "msr TPIDR_EL1, x3",
        // Load SPSR and ELR
        "ldp x2, x3, [x1, #264]",
        "msr SPSR_EL1, x2",
        "msr ELR_EL1, x3",
        // Load general purpose registers
        "ldp x2, x3, [x1, #16]",
        "ldp x4, x5, [x1, #32]",
        "ldp x6, x7, [x1, #48]",
        "ldp x8, x9, [x1, #64]",
        "ldp x10, x11, [x1, #80]",
        "ldp x12, x13, [x1, #96]",
        "ldp x14, x15, [x1, #112]",
        "ldp x16, x17, [x1, #128]",
        "ldp x18, x19, [x1, #144]",
        "ldp x20, x21, [x1, #160]",
        "ldp x22, x23, [x1, #176]",
        "ldp x24, x25, [x1, #192]",
        "ldp x26, x27, [x1, #208]",
        "ldp x28, x29, [x1, #224]",
        "ldr x30, [x1, #240]",
        // Load stack pointer
        "ldr x0, [x1, #248]",
        "mov sp, x0",
        // Load x0 and x1 last
        "ldp x0, x1, [x1, #0]",
        // Return to new context
        "ret",
        options(noreturn)
    );
}

/// Initialize FPU for current CPU
pub fn init_fpu() {
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

/// Check if CPU supports SVE
#[allow(dead_code)]
pub fn has_sve() -> bool {
    unsafe {
        let mut id_aa64pfr0: u64;
        asm!("mrs {}, ID_AA64PFR0_EL1", out(reg) id_aa64pfr0);
        ((id_aa64pfr0 >> 32) & 0xF) != 0
    }
}

/// Enable SVE if supported
#[allow(dead_code)]
pub fn enable_sve() {
    if has_sve() {
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
#[allow(dead_code)]
pub fn current_el() -> u8 {
    unsafe {
        let mut current_el: u64;
        asm!("mrs {}, CurrentEL", out(reg) current_el);
        ((current_el >> 2) & 0x3) as u8
    }
}

/// Load context for first time (no previous context to save)
///
/// # Safety
/// This function manipulates CPU state directly and must be called
/// with interrupts disabled.
#[naked]
#[no_mangle]
pub unsafe extern "C" fn load_context(_context: *const AArch64Context) {
    asm!(
        // x0 = context pointer

        // Load translation table base
        "ldr x1, [x0, #296]",
        "msr TTBR0_EL1, x1",
        "isb",
        // Load thread pointers
        "ldp x1, x2, [x0, #280]",
        "msr TPIDR_EL0, x1",
        "msr TPIDR_EL1, x2",
        // Load SPSR and ELR
        "ldp x1, x2, [x0, #264]",
        "msr SPSR_EL1, x1",
        "msr ELR_EL1, x2",
        // Load general purpose registers
        "ldp x2, x3, [x0, #16]",
        "ldp x4, x5, [x0, #32]",
        "ldp x6, x7, [x0, #48]",
        "ldp x8, x9, [x0, #64]",
        "ldp x10, x11, [x0, #80]",
        "ldp x12, x13, [x0, #96]",
        "ldp x14, x15, [x0, #112]",
        "ldp x16, x17, [x0, #128]",
        "ldp x18, x19, [x0, #144]",
        "ldp x20, x21, [x0, #160]",
        "ldp x22, x23, [x0, #176]",
        "ldp x24, x25, [x0, #192]",
        "ldp x26, x27, [x0, #208]",
        "ldp x28, x29, [x0, #224]",
        "ldr x30, [x0, #240]",
        // Load stack pointer
        "ldr x1, [x0, #248]",
        "mov sp, x1",
        // Load x0 and x1
        "ldp x0, x1, [x0, #0]",
        // Return to loaded context via exception return
        "eret",
        options(noreturn)
    );
}

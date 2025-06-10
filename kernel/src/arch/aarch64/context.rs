//! AArch64 context switching implementation

use core::arch::asm;

/// AArch64 CPU context
#[repr(C)]
#[derive(Debug, Clone)]
pub struct Context {
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
pub struct FpuState {
    /// SIMD&FP registers (v0-v31)
    pub v: [[u64; 2]; 32],
    /// Floating-point control register
    pub fpcr: u32,
    /// Floating-point status register
    pub fpsr: u32,
}

impl Context {
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

/// Switch from current context to new context
///
/// # Safety
/// This function manipulates CPU state directly and must be called
/// with interrupts disabled.
#[no_mangle]
pub unsafe extern "C" fn context_switch(current: *mut Context, next: *const Context) {
    // Note: This is a simplified implementation
    // Real implementation would need proper register saving/restoring

    // Save current context
    let current_ref = &mut *current;
    let next_ref = &*next;

    // In a real implementation, we would:
    // 1. Save all general purpose registers to current context
    // 2. Save FPU/NEON state
    // 3. Load new page table if different
    // 4. Load all registers from new context
    // 5. Return to new context

    // For now, this is a placeholder
    core::ptr::copy_nonoverlapping(next_ref, current_ref, 1);
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
pub fn has_sve() -> bool {
    unsafe {
        let mut id_aa64pfr0: u64;
        asm!("mrs {}, ID_AA64PFR0_EL1", out(reg) id_aa64pfr0);
        ((id_aa64pfr0 >> 32) & 0xF) != 0
    }
}

/// Enable SVE if supported
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
pub fn current_el() -> u8 {
    unsafe {
        let mut current_el: u64;
        asm!("mrs {}, CurrentEL", out(reg) current_el);
        ((current_el >> 2) & 0x3) as u8
    }
}

//! RISC-V context switching implementation

use core::arch::asm;

use crate::sched::task::TaskContext;

/// RISC-V CPU context
#[repr(C)]
#[derive(Debug, Clone)]
pub struct RiscVContext {
    /// Return address
    pub ra: usize,
    /// Stack pointer
    pub sp: usize,
    /// Global pointer
    pub gp: usize,
    /// Thread pointer
    pub tp: usize,
    /// Temporary registers
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    /// Saved registers
    pub s0: usize, // Frame pointer
    pub s1: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    /// Function arguments
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    /// Temporary registers (continued)
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,

    /// Program counter
    pub pc: usize,

    /// Machine status register
    pub sstatus: usize,

    /// Supervisor exception program counter
    pub sepc: usize,

    /// Supervisor trap vector
    pub stvec: usize,

    /// Supervisor address translation and protection
    pub satp: usize,

    /// Floating point registers (if F extension enabled)
    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    pub fp_regs: FpuState,
}

/// RISC-V FPU state (F/D extensions)
#[repr(C, align(16))]
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#[derive(Debug, Clone)]
pub struct FpuState {
    /// Floating-point registers (f0-f31)
    pub f: [f64; 32],
    /// Floating-point control and status register
    pub fcsr: u32,
}

impl RiscVContext {
    /// Create new context for a task
    pub fn new(entry_point: usize, stack_pointer: usize) -> Self {
        Self {
            // Clear all registers
            ra: 0,
            sp: stack_pointer,
            gp: 0,
            tp: 0,
            t0: 0,
            t1: 0,
            t2: 0,
            s0: 0,
            s1: 0,
            s2: 0,
            s3: 0,
            s4: 0,
            s5: 0,
            s6: 0,
            s7: 0,
            s8: 0,
            s9: 0,
            s10: 0,
            s11: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            a3: 0,
            a4: 0,
            a5: 0,
            a6: 0,
            a7: 0,
            t3: 0,
            t4: 0,
            t5: 0,
            t6: 0,

            // Set program counter to entry point
            pc: entry_point,

            // Default sstatus (supervisor mode, interrupts enabled)
            sstatus: 0x120, // SPP=1, SPIE=1

            // Set supervisor exception PC
            sepc: entry_point,

            // Will be set to actual trap vector
            stvec: 0,

            // Will be set to actual page table
            satp: 0,

            // Clear FPU state
            #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
            fp_regs: FpuState {
                f: [0.0; 32],
                fcsr: 0,
            },
        }
    }
}

impl crate::arch::context::ThreadContext for RiscVContext {
    fn new() -> Self {
        Self {
            ra: 0,
            sp: 0,
            gp: 0,
            tp: 0,
            t0: 0,
            t1: 0,
            t2: 0,
            s0: 0,
            s1: 0,
            s2: 0,
            s3: 0,
            s4: 0,
            s5: 0,
            s6: 0,
            s7: 0,
            s8: 0,
            s9: 0,
            s10: 0,
            s11: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            a3: 0,
            a4: 0,
            a5: 0,
            a6: 0,
            a7: 0,
            t3: 0,
            t4: 0,
            t5: 0,
            t6: 0,
            pc: 0,
            sstatus: 0x120,
            sepc: 0,
            stvec: 0,
            satp: 0,
            #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
            fp_regs: FpuState {
                f: [0.0; 32],
                fcsr: 0,
            },
        }
    }

    fn init(&mut self, entry_point: usize, stack_pointer: usize, _kernel_stack: usize) {
        self.pc = entry_point;
        self.sepc = entry_point;
        self.sp = stack_pointer;
    }

    fn get_instruction_pointer(&self) -> usize {
        self.pc
    }

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.pc = ip;
        self.sepc = ip;
    }

    fn get_stack_pointer(&self) -> usize {
        self.sp
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.sp = sp;
    }

    fn get_kernel_stack(&self) -> usize {
        // TODO: Return from thread pointer
        0
    }

    fn set_kernel_stack(&mut self, _sp: usize) {
        // TODO: Set in thread pointer
    }

    fn set_return_value(&mut self, value: usize) {
        self.a0 = value; // a0 is return register
    }

    fn clone_from(&mut self, other: &Self) {
        *self = other.clone();
    }

    fn to_task_context(&self) -> TaskContext {
        TaskContext::RiscV(self.clone())
    }
}

/// Switch context using the ThreadContext interface
pub fn switch_context(from: &mut RiscVContext, to: &RiscVContext) {
    unsafe {
        context_switch(from as *mut _, to as *const _);
    }
}

/// Switch from current context to new context
///
/// # Safety
/// This function manipulates CPU state directly and must be called
/// with interrupts disabled.
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#[no_mangle]
pub unsafe extern "C" fn context_switch(current: *mut RiscVContext, next: *const RiscVContext) {
    // Note: This is a simplified implementation
    // Real implementation would need proper register saving/restoring

    // Save current context
    let current_ref = &mut *current;
    let next_ref = &*next;

    // In a real implementation, we would:
    // 1. Save all general purpose registers to current context
    // 2. Save FPU state if enabled
    // 3. Load new page table if different
    // 4. Load all registers from new context
    // 5. Return to new context

    // For now, this is a placeholder
    core::ptr::copy_nonoverlapping(next_ref, current_ref, 1);
}

/// Initialize FPU for current CPU
pub fn init_fpu() {
    unsafe {
        // Enable FPU in mstatus
        asm!(
            "li t0, 0x6000",     // FS = 11 (dirty)
            "csrs mstatus, t0",
            out("t0") _,
        );
    }
}

/// Check if CPU supports F extension
pub fn has_f_extension() -> bool {
    // Check misa register
    unsafe {
        let misa: usize;
        asm!("csrr {}, misa", out(reg) misa);
        (misa & (1 << 5)) != 0 // Bit 5 = F extension
    }
}

/// Check if CPU supports D extension
pub fn has_d_extension() -> bool {
    // Check misa register
    unsafe {
        let misa: usize;
        asm!("csrr {}, misa", out(reg) misa);
        (misa & (1 << 3)) != 0 // Bit 3 = D extension
    }
}

/// Get current hart (hardware thread) ID
pub fn hart_id() -> usize {
    unsafe {
        let id: usize;
        asm!("csrr {}, mhartid", out(reg) id);
        id
    }
}

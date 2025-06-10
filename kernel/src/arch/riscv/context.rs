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
        // Kernel stack pointer is stored in thread pointer (tp)
        self.tp
    }

    fn set_kernel_stack(&mut self, sp: usize) {
        // Store kernel stack in thread pointer for quick access
        self.tp = sp;
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
    // Cannot use naked functions with asm! macro in current Rust version
    // Using inline assembly in regular function
    asm!(
        // a0 = current context pointer
        // a1 = next context pointer

        // Save current context
        // Save return address
        "sd ra, 0(a0)",
        // Save stack pointer
        "sd sp, 8(a0)",
        // Save global pointer
        "sd gp, 16(a0)",
        // Save thread pointer
        "sd tp, 24(a0)",
        // Save temporary registers
        "sd t0, 32(a0)",
        "sd t1, 40(a0)",
        "sd t2, 48(a0)",
        // Save saved registers
        "sd s0, 56(a0)",
        "sd s1, 64(a0)",
        "sd s2, 72(a0)",
        "sd s3, 80(a0)",
        "sd s4, 88(a0)",
        "sd s5, 96(a0)",
        "sd s6, 104(a0)",
        "sd s7, 112(a0)",
        "sd s8, 120(a0)",
        "sd s9, 128(a0)",
        "sd s10, 136(a0)",
        "sd s11, 144(a0)",
        // Save argument registers
        "sd a0, 152(a0)", // Save current a0
        "sd a1, 160(a0)", // Save current a1
        "sd a2, 168(a0)",
        "sd a3, 176(a0)",
        "sd a4, 184(a0)",
        "sd a5, 192(a0)",
        "sd a6, 200(a0)",
        "sd a7, 208(a0)",
        // Save more temporary registers
        "sd t3, 216(a0)",
        "sd t4, 224(a0)",
        "sd t5, 232(a0)",
        "sd t6, 240(a0)",
        // Save CSRs
        "csrr t0, sstatus",
        "sd t0, 256(a0)",
        "csrr t0, sepc",
        "sd t0, 264(a0)",
        "csrr t0, stvec",
        "sd t0, 272(a0)",
        "csrr t0, satp",
        "sd t0, 280(a0)",
        // Load new context
        // Load satp first (if different)
        "ld t1, 280(a1)", // New satp
        "beq t0, t1, 1f", // Skip if same
        "csrw satp, t1",  // Set new page table
        "sfence.vma",     // Flush TLB
        "1:",
        // Load CSRs
        "ld t0, 256(a1)",
        "csrw sstatus, t0",
        "ld t0, 264(a1)",
        "csrw sepc, t0",
        "ld t0, 272(a1)",
        "csrw stvec, t0",
        // Load general purpose registers
        "ld ra, 0(a1)",
        "ld sp, 8(a1)",
        "ld gp, 16(a1)",
        "ld tp, 24(a1)",
        // Load temporary registers
        "ld t0, 32(a1)",
        "ld t1, 40(a1)",
        "ld t2, 48(a1)",
        // Load saved registers
        "ld s0, 56(a1)",
        "ld s1, 64(a1)",
        "ld s2, 72(a1)",
        "ld s3, 80(a1)",
        "ld s4, 88(a1)",
        "ld s5, 96(a1)",
        "ld s6, 104(a1)",
        "ld s7, 112(a1)",
        "ld s8, 120(a1)",
        "ld s9, 128(a1)",
        "ld s10, 136(a1)",
        "ld s11, 144(a1)",
        // Load argument registers (except a0, a1)
        "ld a2, 168(a1)",
        "ld a3, 176(a1)",
        "ld a4, 184(a1)",
        "ld a5, 192(a1)",
        "ld a6, 200(a1)",
        "ld a7, 208(a1)",
        // Load more temporary registers
        "ld t3, 216(a1)",
        "ld t4, 224(a1)",
        "ld t5, 232(a1)",
        "ld t6, 240(a1)",
        // Load a0 and a1 last
        "ld a0, 152(a1)",
        "ld a1, 160(a1)",
        // Return to new context
        "ret",
        in("a0") current,
        in("a1") next,
        options(noreturn)
    );
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
#[allow(dead_code)]
pub fn has_f_extension() -> bool {
    // Check misa register
    unsafe {
        let misa: usize;
        asm!("csrr {}, misa", out(reg) misa);
        (misa & (1 << 5)) != 0 // Bit 5 = F extension
    }
}

/// Check if CPU supports D extension
#[allow(dead_code)]
pub fn has_d_extension() -> bool {
    // Check misa register
    unsafe {
        let misa: usize;
        asm!("csrr {}, misa", out(reg) misa);
        (misa & (1 << 3)) != 0 // Bit 3 = D extension
    }
}

/// Get current hart (hardware thread) ID
#[allow(dead_code)]
pub fn hart_id() -> usize {
    unsafe {
        let id: usize;
        asm!("csrr {}, mhartid", out(reg) id);
        id
    }
}

/// Load context for first time (no previous context to save)
///
/// # Safety
/// This function manipulates CPU state directly and must be called
/// with interrupts disabled.
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#[no_mangle]
pub unsafe extern "C" fn load_context(context: *const RiscVContext) {
    // Cannot use naked functions with asm! macro in current Rust version
    // Using inline assembly in regular function
    asm!(
        // a0 = context pointer

        // Load satp (page table)
        "ld t0, 280(a0)",
        "csrw satp, t0",
        "sfence.vma", // Flush TLB
        // Load CSRs
        "ld t0, 256(a0)",
        "csrw sstatus, t0",
        "ld t0, 264(a0)",
        "csrw sepc, t0",
        "ld t0, 272(a0)",
        "csrw stvec, t0",
        // Load general purpose registers
        "ld ra, 0(a0)",
        "ld sp, 8(a0)",
        "ld gp, 16(a0)",
        "ld tp, 24(a0)",
        // Load temporary registers
        "ld t0, 32(a0)",
        "ld t1, 40(a0)",
        "ld t2, 48(a0)",
        // Load saved registers
        "ld s0, 56(a0)",
        "ld s1, 64(a0)",
        "ld s2, 72(a0)",
        "ld s3, 80(a0)",
        "ld s4, 88(a0)",
        "ld s5, 96(a0)",
        "ld s6, 104(a0)",
        "ld s7, 112(a0)",
        "ld s8, 120(a0)",
        "ld s9, 128(a0)",
        "ld s10, 136(a0)",
        "ld s11, 144(a0)",
        // Load argument registers (except a0)
        "ld a1, 160(a0)",
        "ld a2, 168(a0)",
        "ld a3, 176(a0)",
        "ld a4, 184(a0)",
        "ld a5, 192(a0)",
        "ld a6, 200(a0)",
        "ld a7, 208(a0)",
        // Load more temporary registers
        "ld t3, 216(a0)",
        "ld t4, 224(a0)",
        "ld t5, 232(a0)",
        "ld t6, 240(a0)",
        // Load a0 last
        "ld a0, 152(a0)",
        // Return to loaded context via supervisor return
        "sret",
        in("a0") context,
        options(noreturn)
    );
}

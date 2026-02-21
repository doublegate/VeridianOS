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

impl Default for RiscVContext {
    fn default() -> Self {
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
}

impl crate::arch::context::ThreadContext for RiscVContext {
    fn new() -> Self {
        Self::default()
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

    fn set_tls_base(&mut self, base: u64) {
        // For user-mode TLS, use tp register
        self.tp = base as usize;
    }

    fn tls_base(&self) -> u64 {
        self.tp as u64
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
    // SAFETY: Both `from` and `to` are valid references to RiscVContext structs
    // with repr(C) layout. context_switch saves all registers from the current
    // CPU state into `from` and restores them from `to`. Interrupts must be
    // disabled by the caller to prevent concurrent access to the contexts.
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
    // SAFETY: Sets the FS field in mstatus to "Dirty" (0x6000), enabling the FPU.
    // This is a privileged CSR write that must be done in machine/supervisor mode.
    // The t0 register is used as a temporary and marked as clobbered.
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
///
/// Currently unused but retained for future FPU context switching
/// and feature detection during SMP bring-up.
#[allow(dead_code)]
pub fn has_f_extension() -> bool {
    // Check misa register
    // SAFETY: Reading the misa CSR is a read-only operation that reports
    // which ISA extensions are supported. Always accessible in M-mode.
    unsafe {
        let misa: usize;
        asm!("csrr {}, misa", out(reg) misa);
        (misa & (1 << 5)) != 0 // Bit 5 = F extension
    }
}

/// Check if CPU supports D extension
///
/// Called by save_fpu_state() and restore_fpu_state() to check
/// for double-precision FP register availability.
pub fn has_d_extension() -> bool {
    // Check misa register
    // SAFETY: Reading the misa CSR is a read-only operation that reports
    // which ISA extensions are supported. Always accessible in M-mode.
    unsafe {
        let misa: usize;
        asm!("csrr {}, misa", out(reg) misa);
        (misa & (1 << 3)) != 0 // Bit 3 = D extension
    }
}

/// Save FPU state (F/D extension registers)
///
/// Saves f0-f31 and fcsr if the D extension is available.
pub fn save_fpu_state(state: &mut FpuState) {
    if !has_d_extension() {
        return;
    }
    // SAFETY: Saves all 32 double-precision FP registers via FSD instructions.
    // The FpuState struct has repr(C, align(16)) layout with f: [f64; 32].
    // The D extension is confirmed available by the has_d_extension() check above.
    unsafe {
        let base = state.f.as_mut_ptr() as *mut u8;
        asm!(
            "fsd f0,  0({base})",
            "fsd f1,  8({base})",
            "fsd f2,  16({base})",
            "fsd f3,  24({base})",
            "fsd f4,  32({base})",
            "fsd f5,  40({base})",
            "fsd f6,  48({base})",
            "fsd f7,  56({base})",
            "fsd f8,  64({base})",
            "fsd f9,  72({base})",
            "fsd f10, 80({base})",
            "fsd f11, 88({base})",
            "fsd f12, 96({base})",
            "fsd f13, 104({base})",
            "fsd f14, 112({base})",
            "fsd f15, 120({base})",
            "fsd f16, 128({base})",
            "fsd f17, 136({base})",
            "fsd f18, 144({base})",
            "fsd f19, 152({base})",
            "fsd f20, 160({base})",
            "fsd f21, 168({base})",
            "fsd f22, 176({base})",
            "fsd f23, 184({base})",
            "fsd f24, 192({base})",
            "fsd f25, 200({base})",
            "fsd f26, 208({base})",
            "fsd f27, 216({base})",
            "fsd f28, 224({base})",
            "fsd f29, 232({base})",
            "fsd f30, 240({base})",
            "fsd f31, 248({base})",
            base = in(reg) base,
        );
        let fcsr: u32;
        asm!("frcsr {fcsr}", fcsr = out(reg) fcsr);
        state.fcsr = fcsr;
    }
}

/// Restore FPU state (F/D extension registers)
///
/// Restores f0-f31 and fcsr if the D extension is available.
pub fn restore_fpu_state(state: &FpuState) {
    if !has_d_extension() {
        return;
    }
    // SAFETY: Restores all 32 double-precision FP registers via FLD instructions.
    // The FpuState struct has repr(C, align(16)) layout with f: [f64; 32].
    // The D extension is confirmed available by the has_d_extension() check above.
    unsafe {
        let fcsr = state.fcsr;
        asm!("fscsr {fcsr}", fcsr = in(reg) fcsr);
        let base = state.f.as_ptr() as *const u8;
        asm!(
            "fld f0,  0({base})",
            "fld f1,  8({base})",
            "fld f2,  16({base})",
            "fld f3,  24({base})",
            "fld f4,  32({base})",
            "fld f5,  40({base})",
            "fld f6,  48({base})",
            "fld f7,  56({base})",
            "fld f8,  64({base})",
            "fld f9,  72({base})",
            "fld f10, 80({base})",
            "fld f11, 88({base})",
            "fld f12, 96({base})",
            "fld f13, 104({base})",
            "fld f14, 112({base})",
            "fld f15, 120({base})",
            "fld f16, 128({base})",
            "fld f17, 136({base})",
            "fld f18, 144({base})",
            "fld f19, 152({base})",
            "fld f20, 160({base})",
            "fld f21, 168({base})",
            "fld f22, 176({base})",
            "fld f23, 184({base})",
            "fld f24, 192({base})",
            "fld f25, 200({base})",
            "fld f26, 208({base})",
            "fld f27, 216({base})",
            "fld f28, 224({base})",
            "fld f29, 232({base})",
            "fld f30, 240({base})",
            "fld f31, 248({base})",
            base = in(reg) base,
        );
    }
}

/// Get current hart (hardware thread) ID
///
/// Currently unused but retained for future SMP support where
/// hart identification is needed for per-CPU data structures.
#[allow(dead_code)]
pub fn hart_id() -> usize {
    // SAFETY: Reading the mhartid CSR is a read-only operation that returns the
    // current hardware thread ID. Always accessible in M-mode with no side effects.
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

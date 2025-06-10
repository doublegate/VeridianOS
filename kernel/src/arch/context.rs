//! Architecture-independent context management interface
//!
//! This module defines the common interface for thread context management
//! that must be implemented for each architecture.

use crate::sched::task::TaskContext;

/// Thread context trait
#[allow(dead_code)]
pub trait ThreadContext: Sized {
    /// Create a new empty context
    fn new() -> Self;

    /// Initialize context for a new thread
    fn init(&mut self, entry_point: usize, stack_pointer: usize, kernel_stack: usize);

    /// Get instruction pointer
    fn get_instruction_pointer(&self) -> usize;

    /// Set instruction pointer
    fn set_instruction_pointer(&mut self, ip: usize);

    /// Get stack pointer
    fn get_stack_pointer(&self) -> usize;

    /// Set stack pointer
    fn set_stack_pointer(&mut self, sp: usize);

    /// Get kernel stack pointer
    fn get_kernel_stack(&self) -> usize;

    /// Set kernel stack pointer
    fn set_kernel_stack(&mut self, sp: usize);

    /// Set return value (for syscalls, fork, etc.)
    fn set_return_value(&mut self, value: usize);

    /// Clone the context
    fn clone_from(&mut self, other: &Self);

    /// Convert to scheduler's TaskContext
    fn to_task_context(&self) -> TaskContext;
}

/// Architecture-specific thread context type alias
#[allow(dead_code)]
#[cfg(target_arch = "x86_64")]
pub type ArchThreadContext = crate::arch::x86_64::context::X86_64Context;

#[allow(dead_code)]
#[cfg(target_arch = "aarch64")]
pub type ArchThreadContext = crate::arch::aarch64::context::AArch64Context;

#[allow(dead_code)]
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub type ArchThreadContext = crate::arch::riscv::context::RiscVContext;

/// Perform a context switch between two threads
///
/// # Safety
/// This function must be called with interrupts disabled and
/// both contexts must be valid.
#[allow(dead_code)]
pub unsafe fn switch_context(from: &mut ArchThreadContext, to: &ArchThreadContext) {
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86_64::context::switch_context(from, to);

    #[cfg(target_arch = "aarch64")]
    crate::arch::aarch64::context::switch_context(from, to);

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    crate::arch::riscv::context::switch_context(from, to);
}

/// Initialize FPU/SIMD for the current CPU
#[allow(dead_code)]
pub fn init_fpu() {
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86_64::context::init_fpu();

    #[cfg(target_arch = "aarch64")]
    crate::arch::aarch64::context::init_fpu();

    #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
    crate::arch::riscv::context::init_fpu();
}

/// Save FPU/SIMD state
#[allow(dead_code)]
pub fn save_fpu_state(state: &mut [u8]) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::context::save_fpu_state(
            &mut *(state.as_mut_ptr() as *mut crate::arch::x86_64::context::FpuState),
        );
    }

    #[cfg(not(target_arch = "x86_64"))]
    let _ = state;

    // TODO: Implement for other architectures
}

/// Restore FPU/SIMD state
#[allow(dead_code)]
pub fn restore_fpu_state(state: &[u8]) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::context::restore_fpu_state(
            &*(state.as_ptr() as *const crate::arch::x86_64::context::FpuState),
        );
    }

    #[cfg(not(target_arch = "x86_64"))]
    let _ = state;

    // TODO: Implement for other architectures
}

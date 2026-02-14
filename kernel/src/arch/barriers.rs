//! Architecture-independent memory barrier abstractions.
//!
//! Centralizes memory barrier/fence operations so that non-arch code does not
//! need scattered `#[cfg(target_arch)]` blocks with inline assembly.
//!
//! # Barrier types
//!
//! * [`memory_fence`] -- full read/write fence (strongest).
//! * [`data_sync_barrier`] -- data synchronization barrier with instruction
//!   synchronization on AArch64; equivalent to a full fence on other
//!   architectures.
//! * [`instruction_sync_barrier`] -- instruction stream synchronization
//!   (AArch64 ISB, RISC-V FENCE.I, x86_64 no-op because of strong ordering).

/// Full memory fence -- all reads and writes issued before this barrier are
/// globally visible before any reads or writes issued after it.
///
/// * **x86_64**: `core::sync::atomic::fence(SeqCst)` -- MFENCE semantics.
/// * **AArch64**: `dsb sy` -- Data Synchronization Barrier (full system).
/// * **RISC-V**: `fence rw, rw` -- read/write ordering fence.
#[inline(always)]
pub fn memory_fence() {
    #[cfg(target_arch = "x86_64")]
    {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: `dsb sy` is a data synchronization barrier that ensures all
        // preceding memory accesses are complete before subsequent ones begin.
        // No side effects beyond ordering.
        unsafe {
            core::arch::asm!("dsb sy", options(nostack, nomem, preserves_flags));
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: `fence rw, rw` ensures all prior reads and writes are ordered
        // before subsequent reads and writes. Standard RISC-V fence instruction.
        unsafe {
            core::arch::asm!("fence rw, rw", options(nostack, nomem, preserves_flags));
        }
    }
}

/// Data synchronization barrier with instruction synchronization.
///
/// On AArch64 this issues `dsb sy` followed by `isb`, which is the standard
/// pattern used when a data store must be visible before instruction fetch
/// proceeds (e.g., writing to a pointer that will be dereferenced
/// immediately after).
///
/// On other architectures this is equivalent to [`memory_fence`] because
/// their memory models already guarantee the necessary ordering.
///
/// * **x86_64**: `core::sync::atomic::fence(SeqCst)`.
/// * **AArch64**: `dsb sy` + `isb`.
/// * **RISC-V**: `fence rw, rw`.
#[inline(always)]
pub fn data_sync_barrier() {
    #[cfg(target_arch = "x86_64")]
    {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: `dsb sy` ensures all data accesses are complete; `isb`
        // flushes the instruction pipeline so subsequent instructions see
        // the updated data. Standard AArch64 barrier pair.
        unsafe {
            core::arch::asm!("dsb sy", "isb", options(nostack, nomem, preserves_flags));
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: `fence rw, rw` is the RISC-V full read/write ordering fence.
        unsafe {
            core::arch::asm!("fence rw, rw", options(nostack, nomem, preserves_flags));
        }
    }
}

/// Instruction synchronization barrier.
///
/// Ensures that all preceding instructions have completed and the instruction
/// pipeline is flushed before subsequent instructions execute.  This is
/// primarily needed on AArch64 and RISC-V after modifying code pages or
/// after a data barrier that affects instruction fetch.
///
/// * **x86_64**: no-op -- x86_64's strong ordering model and unified cache make
///   an explicit instruction barrier unnecessary in most scenarios.
/// * **AArch64**: `isb` -- Instruction Synchronization Barrier.
/// * **RISC-V**: `fence.i` -- Instruction fence.
#[inline(always)]
pub fn instruction_sync_barrier() {
    #[cfg(target_arch = "x86_64")]
    {
        // x86_64 has a strongly ordered memory model; no explicit ISB needed.
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: `isb` flushes the instruction pipeline. No side effects
        // beyond pipeline synchronization.
        unsafe {
            core::arch::asm!("isb", options(nostack, nomem, preserves_flags));
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: `fence.i` synchronizes the instruction and data streams.
        // Required after modifying code in memory. No memory side effects.
        unsafe {
            core::arch::asm!("fence.i", options(nostack, nomem));
        }
    }
}

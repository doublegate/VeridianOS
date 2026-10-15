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
//! * [`dma_wmb`] / [`dma_rmb`] -- order memory a device reads or writes by DMA
//!   (descriptor rings) against other such memory, as Linux's.
//! * [`io_wmb`] -- make prior memory writes visible to a device before a
//!   following MMIO write (a doorbell), as Linux's `__io_bw()`/`__iowmb()`.
//!
//! None of the asm blocks says `nomem`: each is also a compiler barrier, so
//! the compiler cannot move memory accesses across it (they used to, which
//! made them CPU barriers only).

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
            core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: `fence rw, rw` ensures all prior reads and writes are ordered
        // before subsequent reads and writes. Standard RISC-V fence instruction.
        unsafe {
            core::arch::asm!("fence rw, rw", options(nostack, preserves_flags));
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
            core::arch::asm!("dsb sy", "isb", options(nostack, preserves_flags));
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: `fence rw, rw` is the RISC-V full read/write ordering fence.
        unsafe {
            core::arch::asm!("fence rw, rw", options(nostack, preserves_flags));
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
            core::arch::asm!("isb", options(nostack, preserves_flags));
        }
    }

    #[cfg(target_arch = "riscv64")]
    {
        // SAFETY: `fence.i` synchronizes the instruction and data streams.
        // Required after modifying code in memory. No memory side effects.
        unsafe {
            core::arch::asm!("fence.i", options(nostack));
        }
    }
}

/// Order this CPU's earlier writes to DMA memory before its later writes
/// to DMA memory, as a device sees them (Linux `dma_wmb`): ring entries
/// before the index that publishes them.
///
/// * **x86_64**: compiler fence (stores are not reordered with stores).
/// * **AArch64**: `dmb oshst` (outer shareable, stores).
/// * **RISC-V**: `fence w, w`.
#[inline(always)]
pub fn dma_wmb() {
    #[cfg(target_arch = "x86_64")]
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

    #[cfg(target_arch = "aarch64")]
    // SAFETY: `dmb oshst` only orders stores; no other effect.
    unsafe {
        core::arch::asm!("dmb oshst", options(nostack, preserves_flags));
    }

    #[cfg(target_arch = "riscv64")]
    // SAFETY: `fence w, w` only orders stores; no other effect.
    unsafe {
        core::arch::asm!("fence w, w", options(nostack, preserves_flags));
    }
}

/// Order this CPU's earlier reads of DMA memory before its later reads of
/// it (Linux `dma_rmb`): a device-written index before the entries it
/// covers.
///
/// * **x86_64**: compiler fence (loads are not reordered with loads).
/// * **AArch64**: `dmb oshld`.
/// * **RISC-V**: `fence r, r`.
#[inline(always)]
pub fn dma_rmb() {
    #[cfg(target_arch = "x86_64")]
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

    #[cfg(target_arch = "aarch64")]
    // SAFETY: `dmb oshld` only orders loads; no other effect.
    unsafe {
        core::arch::asm!("dmb oshld", options(nostack, preserves_flags));
    }

    #[cfg(target_arch = "riscv64")]
    // SAFETY: `fence r, r` only orders loads; no other effect.
    unsafe {
        core::arch::asm!("fence r, r", options(nostack, preserves_flags));
    }
}

/// Make this CPU's earlier writes to memory visible to devices before its
/// next MMIO write, such as a queue doorbell (CRIT-SEC-01). Without it a
/// weakly ordered CPU may deliver the doorbell before the ring update, and
/// the device finds nothing to do.
///
/// * **x86_64**: compiler fence (stores to write-back memory are ordered before
///   stores to uncached MMIO).
/// * **AArch64**: `dmb oshst`, as Linux's `__iowmb()`.
/// * **RISC-V**: `fence w, o`, as Linux's `__io_bw()`.
#[inline(always)]
pub fn io_wmb() {
    #[cfg(target_arch = "x86_64")]
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

    #[cfg(target_arch = "aarch64")]
    // SAFETY: `dmb oshst` only orders stores; no other effect.
    unsafe {
        core::arch::asm!("dmb oshst", options(nostack, preserves_flags));
    }

    #[cfg(target_arch = "riscv64")]
    // SAFETY: `fence w, o` orders memory stores before device output; no
    // other effect.
    unsafe {
        core::arch::asm!("fence w, o", options(nostack, preserves_flags));
    }
}

//! TLB invalidation on every CPU (MEM-SEC-02, MEM-ARCH-03).
//!
//! A mapping that is removed or changed must be invalidated in every CPU's
//! TLB before the frame behind it is reused, or another CPU can keep using
//! a stale translation to memory that now belongs to someone else. These
//! functions flush locally and then reach every other online CPU, and
//! return only once all of them have flushed:
//!
//! - AArch64: the architectural flushes use the Inner Shareable broadcast forms
//!   (`tlbi ...is` + `dsb ish`), so the local flush already covers every CPU.
//! - RISC-V: SBI RFENCE `remote_sfence_vma` on all harts (OpenSBI completes it
//!   synchronously).
//! - x86_64: an IPI (vector 49) to every other CPU, each of which flushes and
//!   acknowledges; the requester waits for all acknowledgements. A CPU waiting
//!   to make its own request services requests aimed at it, so two CPUs
//!   requesting at once with interrupts disabled cannot deadlock.
//!
//! With one CPU online the remote part is a single atomic load.

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
use core::sync::atomic::AtomicU32;
use core::sync::atomic::{AtomicBool, Ordering};

/// Set when a remote invalidation could not be confirmed (an x86 CPU that
/// did not acknowledge, an SBI error). Checked by the boot tests.
static REMOTE_FAILED: AtomicBool = AtomicBool::new(false);

/// Whether every remote invalidation so far was confirmed.
pub fn remote_flushes_confirmed() -> bool {
    !REMOTE_FAILED.load(Ordering::Acquire)
}

/// Invalidate the translation of `vaddr` on every CPU.
pub fn flush_page(vaddr: u64) {
    crate::arch::tlb_flush_address(vaddr);
    remote(Some(vaddr));
}

/// Invalidate the translations of `pages` on every CPU, with one remote
/// request for the whole set.
pub fn flush_pages(pages: &[u64]) {
    match pages {
        [] => {}
        [one] => flush_page(*one),
        _ => {
            for &p in pages {
                crate::arch::tlb_flush_address(p);
            }
            remote(None);
        }
    }
}

/// Invalidate every (non-global) translation on every CPU.
pub fn flush_all() {
    crate::arch::tlb_flush_all();
    remote(None);
}

fn remote(addr: Option<u64>) {
    if crate::arch::smp_boot::online_cpus() <= 1 {
        return;
    }
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        let _ = addr;
        x86::shootdown();
    }
    #[cfg(all(target_arch = "riscv64", target_os = "none"))]
    {
        let (start, size) = match addr {
            Some(a) => (a as usize & !0xFFF, 4096),
            None => (0, usize::MAX),
        };
        if !crate::arch::riscv::sbi::remote_sfence_vma_all(start, size).is_ok() {
            REMOTE_FAILED.store(true, Ordering::Release);
        }
    }
    #[cfg(not(any(
        all(target_arch = "x86_64", target_os = "none"),
        all(target_arch = "riscv64", target_os = "none")
    )))]
    let _ = addr;
}

/// Service a TLB shootdown aimed at this CPU (from the IPI handler, or from
/// a CPU spinning while it waits to make its own request).
pub fn service_pending() {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    x86::service_pending();
}

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod x86 {
    use super::*;
    use crate::arch::percpu::MAX_CPUS;

    /// Held by the CPU whose request is in flight.
    static REQUEST_LOCK: AtomicBool = AtomicBool::new(false);
    /// CPUs that have yet to acknowledge the request in flight.
    static ACKS_OUTSTANDING: AtomicU32 = AtomicU32::new(0);
    /// Per CPU: a flush is requested of it.
    static PENDING: [AtomicBool; MAX_CPUS] = [const { AtomicBool::new(false) }; MAX_CPUS];

    /// Longest wait for acknowledgements before giving up with a warning
    /// (a CPU that never answers must not hang the kernel).
    const ACK_TIMEOUT_MS: u64 = 100;

    pub(super) fn service_pending() {
        let me = crate::arch::percpu::this_cpu_id() as usize;
        if me < MAX_CPUS && PENDING[me].swap(false, Ordering::AcqRel) {
            crate::arch::tlb_flush_all();
            ACKS_OUTSTANDING.fetch_sub(1, Ordering::AcqRel);
        }
    }

    pub(super) fn shootdown() {
        let me = crate::arch::percpu::this_cpu_id() as usize;
        while REQUEST_LOCK
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            service_pending();
            core::hint::spin_loop();
        }
        let online = (crate::arch::smp_boot::online_cpus() as usize).min(MAX_CPUS);
        ACKS_OUTSTANDING.store(online as u32 - 1, Ordering::Release);
        for (cpu, pending) in PENDING.iter().enumerate().take(online) {
            if cpu != me {
                pending.store(true, Ordering::Release);
            }
        }
        let _ = crate::arch::x86_64::apic::send_ipi_all_excluding_self(
            crate::arch::x86_64::apic::TLB_SHOOTDOWN_VECTOR,
        );
        let start = crate::arch::timer::monotonic_ns();
        while ACKS_OUTSTANDING.load(Ordering::Acquire) != 0 {
            service_pending();
            if crate::arch::timer::monotonic_ns().saturating_sub(start) > ACK_TIMEOUT_MS * 1_000_000
            {
                // SAFETY: raw COM1 output, no locks taken.
                unsafe {
                    crate::arch::x86_64::idt::raw_serial_str(
                        b"[TLB] shootdown not acknowledged by every CPU\n",
                    )
                };
                REMOTE_FAILED.store(true, Ordering::Release);
                for p in PENDING.iter() {
                    p.store(false, Ordering::Release);
                }
                ACKS_OUTSTANDING.store(0, Ordering::Release);
                break;
            }
            core::hint::spin_loop();
        }
        REQUEST_LOCK.store(false, Ordering::Release);
    }
}

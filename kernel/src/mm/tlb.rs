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
//! This fails closed: a requester never returns -- and so never lets a frame
//! be reused -- until every CPU has flushed. A slow CPU is reported once and
//! still waited for, as Linux waits in `smp_call_function`; an SBI RFENCE
//! error is fatal.
//!
//! With one CPU online the remote part is a single atomic load.
//!
//! **Targeted flushes (N-262).** A change to a user address space only has to
//! reach the CPUs running it: user mappings are not global and PCIDs are not
//! used, so a CPU that switched to another page-table root dropped every
//! entry of this one with the CR3 load. Each CPU records the root it is about
//! to load ([`note_root`], before the load, sequentially consistent); a
//! requester changes the page tables, then reads the records (also
//! sequentially consistent). Either it sees a CPU's new root and sends it the
//! request, or that CPU's load comes after the change and walks the new
//! tables. A CPU with no record yet is always sent the request.

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
use core::sync::atomic::AtomicU32;
use core::sync::atomic::{AtomicBool, Ordering};

/// Mask of a CR3 value's page-table address.
const ROOT_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// No root recorded for the CPU yet: every request goes to it.
const UNKNOWN_ROOT: u64 = u64::MAX;

/// The page-table root each CPU runs on (see the module docs).
static CPU_ROOT: [core::sync::atomic::AtomicU64; crate::sched::smp::MAX_CPUS] =
    [const { core::sync::atomic::AtomicU64::new(UNKNOWN_ROOT) }; crate::sched::smp::MAX_CPUS];

/// Record that this CPU is about to load page-table root `root` (a CR3
/// value). Every load of a different root must call this first, or a
/// targeted flush could miss this CPU.
pub fn note_root(root: u64) {
    let me = crate::arch::percpu::this_cpu_id() as usize;
    if let Some(slot) = CPU_ROOT.get(me) {
        slot.store(root & ROOT_MASK, Ordering::SeqCst);
    }
}

/// The other CPUs that may hold translations of user address space `root`
/// (all of them for `root` 0, meaning unknown).
fn cpus_on(root: u64) -> u64 {
    let me = crate::arch::percpu::this_cpu_id() as usize;
    let online = (crate::arch::smp_boot::online_cpus() as usize).min(CPU_ROOT.len());
    // Order the caller's page-table changes before reading the records.
    core::sync::atomic::fence(Ordering::SeqCst);
    let mut records = [UNKNOWN_ROOT; crate::sched::smp::MAX_CPUS];
    for (rec, slot) in records.iter_mut().zip(CPU_ROOT.iter()).take(online) {
        *rec = slot.load(Ordering::SeqCst);
    }
    targets(root, me, &records[..online])
}

/// The CPUs other than `me` whose recorded root (`records`, by CPU) may
/// hold translations of address space `root`: those on it, those with no
/// record, and every one for `root` 0 (unknown).
fn targets(root: u64, me: usize, records: &[u64]) -> u64 {
    let root = root & ROOT_MASK;
    records
        .iter()
        .enumerate()
        .filter(|&(cpu, &r)| cpu != me && cpu < 64 && (root == 0 || r == root || r == UNKNOWN_ROOT))
        .fold(0, |mask, (cpu, _)| mask | 1 << cpu)
}

/// Invalidate user address `vaddr` of address space `root` (0: unknown) on
/// every CPU that may hold it.
pub fn flush_page_in(root: u64, vaddr: u64) {
    if is_kernel_half(vaddr) {
        return flush_page(vaddr);
    }
    crate::arch::tlb_flush_address(vaddr);
    remote_to(cpus_on(root), Some(vaddr));
}

/// Invalidate user `pages` of address space `root` (0: unknown), with one
/// remote request.
pub fn flush_pages_in(root: u64, pages: &[u64]) {
    if pages.iter().any(|&p| is_kernel_half(p)) {
        return flush_pages(pages);
    }
    for &p in pages {
        crate::arch::tlb_flush_address(p);
    }
    if !pages.is_empty() {
        remote_to(cpus_on(root), None);
    }
}

/// Invalidate every user translation of address space `root` (0: unknown).
pub fn flush_all_in(root: u64) {
    crate::arch::tlb_flush_all();
    remote_to(cpus_on(root), None);
}

/// Whether `vaddr` is in the kernel half (shared by every address space).
fn is_kernel_half(vaddr: u64) -> bool {
    vaddr >= 0xFFFF_8000_0000_0000
}

/// Set when a remote invalidation was slow to be confirmed (it was still
/// waited for). Checked by the boot tests.
static REMOTE_SLOW: AtomicBool = AtomicBool::new(false);

/// Whether every remote invalidation so far was confirmed promptly.
pub fn remote_flushes_confirmed() -> bool {
    !REMOTE_SLOW.load(Ordering::Acquire)
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
    remote_to(u64::MAX, addr);
}

/// Reach the CPUs in `mask` (all of them on RISC-V, which has no user mode
/// yet and flushes every hart through the SBI).
fn remote_to(mask: u64, addr: Option<u64>) {
    if crate::arch::smp_boot::online_cpus() <= 1 || mask == 0 {
        return;
    }
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        let _ = addr;
        x86::shootdown(mask);
    }
    #[cfg(all(target_arch = "riscv64", target_os = "none"))]
    {
        let (start, size) = match addr {
            Some(a) => (a as usize & !0xFFF, 4096),
            None => (0, usize::MAX),
        };
        // Secondary harts are started only when the firmware has RFENCE
        // (smp bring-up checks), so an error means another hart may keep a
        // stale translation; continuing could let the frame be reused while
        // it is still mapped there.
        let ret = crate::arch::riscv::sbi::remote_sfence_vma_all(start, size);
        assert!(ret.is_ok(), "SBI remote_sfence_vma failed: {}", ret.error);
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

    /// Wait after which a missing acknowledgement is reported (and still
    /// waited for).
    const ACK_WARN_MS: u64 = 100;

    pub(super) fn service_pending() {
        let me = crate::arch::percpu::this_cpu_id() as usize;
        if me < MAX_CPUS && PENDING[me].swap(false, Ordering::AcqRel) {
            crate::arch::tlb_flush_all();
            ACKS_OUTSTANDING.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// Flush on the CPUs in `mask` (other than this one) and wait for each.
    pub(super) fn shootdown(mask: u64) {
        let me = crate::arch::percpu::this_cpu_id() as usize;
        while REQUEST_LOCK
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            service_pending();
            core::hint::spin_loop();
        }
        let online = (crate::arch::smp_boot::online_cpus() as usize).min(MAX_CPUS);
        let everyone = if online >= 64 {
            u64::MAX
        } else {
            (1u64 << online) - 1
        } & !(1 << me);
        let targets = mask & everyone;
        if targets == 0 {
            REQUEST_LOCK.store(false, Ordering::Release);
            return;
        }
        ACKS_OUTSTANDING.store(targets.count_ones(), Ordering::Release);
        for (cpu, pending) in PENDING.iter().enumerate().take(online) {
            if targets & (1 << cpu) != 0 {
                pending.store(true, Ordering::Release);
            }
        }
        if targets == everyone {
            let _ = crate::arch::x86_64::apic::send_ipi_all_excluding_self(
                crate::arch::x86_64::apic::TLB_SHOOTDOWN_VECTOR,
            );
        } else {
            for cpu in 0..online {
                if targets & (1 << cpu) != 0 {
                    let _ = crate::arch::x86_64::apic::send_ipi_to_cpu(
                        cpu,
                        crate::arch::x86_64::apic::TLB_SHOOTDOWN_VECTOR,
                    );
                }
            }
        }
        let start = crate::arch::timer::monotonic_ns();
        let mut warned = false;
        // Fail closed: wait until every CPU has flushed, however long that
        // takes. The caller is about to reuse the frames, and giving up
        // here (as a timeout once did) let a slow CPU keep a translation to
        // a frame that already belonged to someone else; it also let a late
        // acknowledgement wrap the counter.
        while ACKS_OUTSTANDING.load(Ordering::Acquire) != 0 {
            service_pending();
            if !warned
                && crate::arch::timer::monotonic_ns().saturating_sub(start)
                    > ACK_WARN_MS * 1_000_000
            {
                warned = true;
                REMOTE_SLOW.store(true, Ordering::Release);
                // SAFETY: raw COM1 output, no locks taken.
                unsafe {
                    crate::arch::x86_64::idt::raw_serial_str(
                        b"[TLB] shootdown still waiting for a CPU after 100 ms\n",
                    )
                };
            }
            core::hint::spin_loop();
        }
        REQUEST_LOCK.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_targets_only_cpus_on_the_address_space() {
        let a = 0x1234_5000;
        let b = 0x9999_0000;
        let records = [a, b, a, UNKNOWN_ROOT];
        // CPU 0 asks: CPU 2 runs `a`, CPU 3 has no record yet.
        assert_eq!(targets(a, 0, &records), 0b1100);
        // CR3 flag bits do not matter.
        assert_eq!(targets(a | 0x18, 0, &records), 0b1100);
        // An address space no other CPU runs: only the unrecorded one.
        assert_eq!(targets(0x5555_0000, 1, &records), 0b1000);
        // Unknown space: everyone but the requester.
        assert_eq!(targets(0, 1, &records), 0b1101);
    }
}

//! Local descriptor tables and modify_ldt(2) (ADR 0027 decision 29), as
//! Linux 7.3's arch/x86/kernel/ldt.c built with MODIFY_LDT_SYSCALL=y,
//! X86_16BIT=n and without IA32 emulation.
//!
//! Each address space (each `Process`; a CLONE_VM child shares its parent's)
//! has at most one table, made on its first write. Linux replaces its table
//! on every write and switches every CPU over with an IPI; here the table
//! is a fixed 8,192 entries (64 KiB, LDT_ENTRIES), never moved, and a write
//! stores one 8-byte descriptor in place, so no CPU's LDTR names a stale
//! table. A segment register caches its descriptor when loaded, though:
//! after a write, ds and es holding LDT selectors are reloaded on every CPU
//! running the address space, the writer's first and the others through the
//! shootdown IPI, before modify_ldt returns, as Linux's flush_ldt and
//! refresh_ldt_segments do (fs and gs keep their 64-bit bases, as there).
//!
//! The table lives in the address space's slot ([`LdtSlot`]), which is set
//! once, on the first write, and never changed: exec gives the process a
//! fresh slot instead. Every task holds its address space's slot, read at
//! each switch-in without a lock, so a table made after a thread started is
//! that thread's at its next switch with no pass over the tasks (PR #50
//! review: such a pass could miss a task being started, or reach one after
//! its process had exec'd). A slot, and its table, is freed only when the
//! last task holding it is dropped, after it has left its CPU, and a switch
//! always loads the next task's table (or none), so no CPU's LDTR names a
//! freed table.
//!
//! A thread on another CPU whose LDTR predates the table (made by a
//! sibling after it was switched in) loads it on its next switch, or at
//! once when it uses an LDT selector: its #GP or #NP from ring 3 reloads
//! the table and runs the instruction again ([`refresh_for_fault`]).
//!
//! The segment registers a thread loaded (ds, es, fs, gs) are kept per
//! thread across switches, as Linux's `thread.ds` and friends. A selector
//! is checked against the table it names before it is loaded, under the
//! table's lock, which modify_ldt also takes to write: the load can then
//! never fault in ring 0; a selector whose entry was cleared meanwhile is
//! loaded as 0, as Linux's loadsegment fixup does.

#[cfg(feature = "alloc")]
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Entries in a table (LDT_ENTRIES) and their size (LDT_ENTRY_SIZE).
pub const LDT_ENTRIES: usize = 8192;
pub const LDT_ENTRY_SIZE: usize = 8;

/// `struct user_desc` (asm/ldt.h): three words, then the flag bits.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserDesc {
    pub entry_number: u32,
    pub base_addr: u32,
    pub limit: u32,
    pub flags: u32,
}

impl UserDesc {
    fn seg_32bit(&self) -> u32 {
        self.flags & 1
    }
    fn contents(&self) -> u32 {
        (self.flags >> 1) & 3
    }
    fn read_exec_only(&self) -> u32 {
        (self.flags >> 3) & 1
    }
    fn limit_in_pages(&self) -> u32 {
        (self.flags >> 4) & 1
    }
    fn seg_not_present(&self) -> u32 {
        (self.flags >> 5) & 1
    }
    fn useable(&self) -> u32 {
        (self.flags >> 6) & 1
    }

    /// LDT_empty: the description that clears an entry.
    fn is_empty(&self) -> bool {
        self.base_addr == 0
            && self.limit == 0
            && self.contents() == 0
            && self.read_exec_only() == 1
            && self.seg_32bit() == 0
            && self.limit_in_pages() == 0
            && self.seg_not_present() == 1
            && self.useable() == 0
    }
}

/// fill_ldt: the descriptor a `user_desc` describes. Always a DPL 3 code or
/// data segment with its accessed bit set; never a 64-bit (L) one.
pub fn fill_ldt(info: &UserDesc) -> u64 {
    let base = info.base_addr as u64;
    let limit = info.limit as u64;
    let ty = (((info.read_exec_only() ^ 1) << 1) | (info.contents() << 2) | 1) as u64;
    (limit & 0xffff)
        | ((base & 0xffff) << 16)
        | (((base >> 16) & 0xff) << 32)
        | (ty << 40)
        | (1 << 44) // S: a code or data segment
        | (3 << 45) // DPL 3
        | (((info.seg_not_present() ^ 1) as u64) << 47)
        | (((limit >> 16) & 0xf) << 48)
        | ((info.useable() as u64) << 52)
        | ((info.seg_32bit() as u64) << 54)
        | ((info.limit_in_pages() as u64) << 55)
        | (((base >> 24) & 0xff) << 56)
}

/// write_ldt's checks on `info` (`oldmode`: func 1, the old interface):
/// the descriptor to store at `info.entry_number`, or `None` for EINVAL.
pub fn descriptor_for(info: &UserDesc, oldmode: bool) -> Option<u64> {
    if info.entry_number as usize >= LDT_ENTRIES {
        return None;
    }
    if info.contents() == 3 && (oldmode || info.seg_not_present() == 0) {
        return None;
    }
    if (oldmode && info.base_addr == 0 && info.limit == 0) || info.is_empty() {
        return Some(0);
    }
    // 16-bit segments need X86_16BIT, which this kernel is built without.
    if info.seg_32bit() == 0 {
        return None;
    }
    let mut d = fill_ldt(info);
    if oldmode {
        d &= !(1 << 52); // avl
    }
    Some(d)
}

/// Whether descriptor `d` may be loaded into a data segment register (ds,
/// es, fs, gs) from ring 3: present, a code or data segment of DPL 3, and
/// readable if code.
pub fn loadable_data(d: u64) -> bool {
    let present = d & (1 << 47) != 0;
    let s = d & (1 << 44) != 0;
    let dpl = (d >> 45) & 3;
    let code = d & (1 << 43) != 0;
    let readable = d & (1 << 41) != 0;
    present && s && dpl == 3 && (!code || readable)
}

/// The two-slot system descriptor of a table at `base` with
/// LDT_ENTRIES entries (type 2, present).
pub fn ldt_system_descriptor(base: u64) -> (u64, u64) {
    let limit = (LDT_ENTRIES * LDT_ENTRY_SIZE - 1) as u64;
    let low = (limit & 0xffff)
        | ((base & 0xff_ffff) << 16)
        | (0x82 << 40)
        | (((limit >> 16) & 0xf) << 48)
        | (((base >> 24) & 0xff) << 56);
    (low, base >> 32)
}

/// One address space's table.
#[cfg(feature = "alloc")]
pub struct Ldt {
    entries: Box<[AtomicU64]>,
    /// One past the highest entry ever written (nr_entries, for read_ldt).
    nr_entries: AtomicU32,
    /// Writers, and the switch's check-and-load of selectors.
    lock: spin::Mutex<()>,
}

#[cfg(feature = "alloc")]
impl Ldt {
    fn new() -> Option<Arc<Self>> {
        let mut v = Vec::new();
        v.try_reserve_exact(LDT_ENTRIES).ok()?;
        v.resize_with(LDT_ENTRIES, || AtomicU64::new(0));
        Some(Arc::new(Self {
            entries: v.into_boxed_slice(),
            nr_entries: AtomicU32::new(0),
            lock: spin::Mutex::new(()),
        }))
    }

    /// A copy for a fork child (Linux's ldt_dup_context).
    pub fn duplicate(&self) -> Option<Arc<Self>> {
        let new = Self::new()?;
        let _irq = irq_off();
        let _g = self.lock.lock();
        let n = self.nr_entries.load(Ordering::Acquire);
        for i in 0..n as usize {
            new.entries[i].store(self.entries[i].load(Ordering::Relaxed), Ordering::Relaxed);
        }
        new.nr_entries.store(n, Ordering::Release);
        Some(new)
    }

    /// The table's linear address (the LDT descriptor's base).
    pub fn base(&self) -> u64 {
        self.entries.as_ptr() as u64
    }

    /// The descriptor of LDT selector `sel` (TI set), if it names an entry.
    pub fn entry(&self, sel: u16) -> Option<u64> {
        self.entries
            .get((sel >> 3) as usize)
            .map(|e| e.load(Ordering::Acquire))
    }

    fn write(&self, index: usize, desc: u64) {
        let _irq = irq_off();
        let _g = self.lock.lock();
        self.entries[index].store(desc, Ordering::Release);
        self.nr_entries
            .fetch_max(index as u32 + 1, Ordering::AcqRel);
    }

    /// Hold the table still while selectors are checked and loaded. The
    /// caller has interrupts off, as every holder does: the shootdown
    /// handler takes it too, so a holder interrupted on its own CPU would
    /// wait for itself.
    pub fn hold(&self) -> spin::MutexGuard<'_, ()> {
        self.lock.lock()
    }
}

/// Interrupts off for a holder of a table's lock (see [`Ldt::hold`]); a
/// no-op in host tests, which have no interrupts to turn off.
#[cfg(feature = "alloc")]
fn irq_off() -> impl Drop {
    #[cfg(target_os = "none")]
    {
        crate::arch::disable_interrupts()
    }
    #[cfg(not(target_os = "none"))]
    {
        struct NoIrq;
        impl Drop for NoIrq {
            fn drop(&mut self) {}
        }
        NoIrq
    }
}

/// An address space's LDT slot: the table, once its first write made one,
/// and never another (see the module docs). CLONE_VM processes share the
/// slot itself, so a table made after the clone is both processes' (the
/// address space has one LDT; PR #50 review); their tasks hold it too.
/// Read without a lock (`get`), so a switch never waits for a writer.
#[cfg(feature = "alloc")]
pub type LdtSlot = spin::Once<Arc<Ldt>>;

/// A fresh slot, or one holding `table` (a fork child's copy).
#[cfg(feature = "alloc")]
pub fn new_slot(table: Option<Arc<Ldt>>) -> Arc<LdtSlot> {
    Arc::new(match table {
        Some(t) => spin::Once::initialized(t),
        None => spin::Once::new(),
    })
}

/// Whether `sel` may be loaded into a data segment register now, `ldt`
/// being the table the CPU has loaded. In the GDT only null and the user
/// descriptors (`__USER_DS`, `__USER_CS`, `__USER32_CS`) qualify: anything
/// else there is a null, kernel or system descriptor, and loading it in
/// ring 0 at a switch would fault (a stale firmware selector a boot task
/// recorded, say; PR #50 review). Any RPL: a data-register load needs
/// DPL >= max(CPL, RPL), which a DPL 3 descriptor meets at every RPL, in
/// ring 3 as in ring 0, so a program may hold 0x28 in ds (PR #50 review;
/// the RPL 3 rule is ss's, `trap::valid_user_ss`). An LDT selector must
/// name a loadable entry.
#[cfg(feature = "alloc")]
pub fn selector_ok(sel: u16, ldt: Option<&Ldt>) -> bool {
    if sel & 4 == 0 {
        use super::gdt::{USER32_CS, USER_CS, USER_DS};
        let index = sel & !3;
        return index == 0
            || [USER_DS, USER_CS, USER32_CS]
                .iter()
                .any(|u| u & !3 == index);
    }
    match ldt.and_then(|l| l.entry(sel)) {
        Some(d) => loadable_data(d),
        None => false,
    }
}

/// The segment registers a thread runs with, kept across switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Segments {
    pub ds: u16,
    pub es: u16,
    pub fs: u16,
    pub gs: u16,
}

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod hw {
    use super::Segments;

    /// The segment registers as the CPU holds them.
    pub fn read() -> Segments {
        let (ds, es, fs, gs): (u16, u16, u16, u16);
        // SAFETY: reading segment registers has no side effects.
        unsafe {
            core::arch::asm!(
                "mov {0:x}, ds", "mov {1:x}, es", "mov {2:x}, fs", "mov {3:x}, gs",
                out(reg) ds, out(reg) es, out(reg) fs, out(reg) gs,
                options(nomem, nostack, preserves_flags)
            );
        }
        Segments { ds, es, fs, gs }
    }

    /// Load ds.
    ///
    /// # Safety
    /// `sel` must be loadable now (null, a user GDT selector, or a checked
    /// LDT one under the table's lock).
    pub unsafe fn load_ds(sel: u16) {
        // SAFETY: per the contract.
        unsafe { core::arch::asm!("mov ds, {0:x}", in(reg) sel, options(nostack, preserves_flags)) }
    }

    /// Load es. # Safety: as [`load_ds`].
    pub unsafe fn load_es(sel: u16) {
        // SAFETY: per the contract.
        unsafe { core::arch::asm!("mov es, {0:x}", in(reg) sel, options(nostack, preserves_flags)) }
    }

    /// Load fs; the FS base becomes the descriptor's (0 for null).
    /// # Safety: as [`load_ds`].
    pub unsafe fn load_fs(sel: u16) {
        // SAFETY: per the contract.
        unsafe { core::arch::asm!("mov fs, {0:x}", in(reg) sel, options(nostack, preserves_flags)) }
    }

    /// Load gs without touching the per-CPU base: the load is made between
    /// two `swapgs`, so it lands in the user half (Linux's load_gs_index).
    /// The user GS base (IA32_KERNEL_GS_BASE) becomes the descriptor's.
    ///
    /// # Safety
    /// As [`load_ds`], with interrupts off: nothing may run between the
    /// `swapgs` pair but NMI and #MC, which never use the GS base.
    pub unsafe fn load_gs(sel: u16) {
        // SAFETY: per the contract; the load cannot fault (checked), so the
        // second swapgs always runs.
        unsafe {
            core::arch::asm!("swapgs", "mov gs, {0:x}", "swapgs", in(reg) sel, options(nostack, preserves_flags))
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use hw::read as read_segments;

/// Null fs before its base is set from a 64-bit value (arch_prctl
/// ARCH_SET_FS), as Linux's do_arch_prctl_64 loads 0 first: a selector left
/// in fs would make the next switch-in reload the descriptor's base over
/// the one just set (PR #50 review).
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn clear_fs_selector() {
    // SAFETY: the null selector is always loadable.
    unsafe { hw::load_fs(0) }
}

/// Null gs before its user base is set (ARCH_SET_GS), between two
/// `swapgs` (Linux's load_gs_index(0)), with interrupts off.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub fn clear_gs_selector() {
    let irq = crate::arch::disable_interrupts();
    // SAFETY: interrupts off; the null selector is always loadable.
    unsafe { hw::load_gs(0) }
    drop(irq);
}

/// Load this CPU's LDT register with `ldt` (or none) if it holds another,
/// writing the table's descriptor into this CPU's GDT slot first.
///
/// # Safety
/// Interrupts off; `ldt` must outlive its use by this CPU (a task holding
/// it is running here, or about to).
#[cfg(all(target_arch = "x86_64", target_os = "none", feature = "alloc"))]
pub unsafe fn install(ldt: Option<&Ldt>) {
    let cpu = crate::arch::percpu::this_arch_cpu();
    let base = ldt.map_or(0, |l| l.base());
    // SAFETY: this CPU's block, interrupts off.
    unsafe {
        if (*cpu).ldt_loaded == base {
            return;
        }
        if base == 0 {
            core::arch::asm!("lldt {0:x}", in(reg) 0u16, options(nostack, preserves_flags));
        } else {
            let (lo, hi) = ldt_system_descriptor(base);
            super::gdt::set_ldt_descriptor(super::gdt::this_cpu_gdt(), lo, hi);
            core::arch::asm!("lldt {0:x}", in(reg) super::gdt::LDT_SELECTOR, options(nostack, preserves_flags));
        }
        (*cpu).ldt_loaded = base;
    }
}

/// Switch-in of a user task: its table, then its segment registers, each
/// checked against that table (`prev` the outgoing task's, to skip loads
/// of null over null, as Linux does). Returns whether fs and gs were
/// loaded with a selector (their bases then come from the descriptor and
/// must not be overwritten from the saved MSR values).
///
/// # Safety
/// As [`install`].
#[cfg(all(target_arch = "x86_64", target_os = "none", feature = "alloc"))]
pub unsafe fn switch_in(ldt: Option<&Ldt>, prev: Segments, next: Segments) -> (bool, bool) {
    // SAFETY: per the contract.
    unsafe { install(ldt) };
    let _hold = ldt.map(|l| l.hold());
    let ok = |s: u16| if selector_ok(s, ldt) { s } else { 0 };
    let n = Segments {
        ds: ok(next.ds),
        es: ok(next.es),
        fs: ok(next.fs),
        gs: ok(next.gs),
    };
    // SAFETY: every selector was checked under the table's lock.
    unsafe {
        if n.ds | prev.ds != 0 {
            hw::load_ds(n.ds);
        }
        if n.es | prev.es != 0 {
            hw::load_es(n.es);
        }
        if n.fs > 3 || prev.fs != n.fs {
            hw::load_fs(n.fs);
        }
        if n.gs > 3 || prev.gs != n.gs {
            hw::load_gs(n.gs);
        }
    }
    // A selector whose entry went while the task was out is loaded as 0
    // with base 0, as Linux's loadsegment fixup leaves it, whatever the
    // previous task's register held; and reported as handled, so the base
    // saved with the dead selector (the descriptor's) is not written back
    // (PR #50 review). The base is written, not left to the null load,
    // which keeps the old base on AMD.
    let fs_dead = next.fs > 3 && n.fs == 0;
    let gs_dead = next.gs > 3 && n.gs == 0;
    // SAFETY: null selectors are always loadable; interrupts are off per
    // the contract.
    unsafe {
        if fs_dead {
            hw::load_fs(0);
            super::switch::write_fs_base(0);
        }
        if gs_dead {
            hw::load_gs(0);
            super::switch::write_user_gs_base(0);
        }
    }
    (n.fs > 3 || fs_dead, n.gs > 3 || gs_dead)
}

/// A #GP or #NP from ring 3 with the LDT bit in its error code, taken on a
/// CPU whose LDTR is not the running task's current table (made by
/// another thread after this one was switched in): load the table and
/// report that the instruction should simply run again.
#[cfg(all(target_arch = "x86_64", target_os = "none", feature = "alloc"))]
pub fn refresh_for_fault(error_code: u64) -> bool {
    // Bit 2 (TI) of a selector error code: the selector was an LDT one.
    if error_code & 4 == 0 {
        return false;
    }
    let Some(table) = crate::sched::dispatch::current_ldt() else {
        return false;
    };
    let cpu = crate::arch::percpu::this_arch_cpu();
    let irq = crate::arch::disable_interrupts();
    // SAFETY: this CPU's block, interrupts off; the running task holds the
    // table.
    let stale = unsafe { (*cpu).ldt_loaded != table.base() };
    if stale {
        // SAFETY: as above.
        unsafe { install(Some(&table)) };
    }
    drop(irq);
    stale
}

/// exec: the new program starts with no LDT (the new mm) and null fs and
/// gs; `enter_usermode` then gives it `__USER_DS` in ds and es, as Linux's
/// start_thread does. This CPU stops naming the table before the process's
/// references to it go; another process sharing it (a CLONE_VM parent)
/// keeps it, with the slot: the exec'd process takes a fresh one.
#[cfg(all(target_arch = "x86_64", target_os = "none", feature = "alloc"))]
pub fn exec_reset() {
    // The task's slot is swapped for a fresh, empty one before this CPU
    // lets go of the old table, and the old one is dropped only after: with
    // the clear first, a REFRESH_LDT interrupt taken between the two would
    // install the old table again from the task's slot, and dropping it then
    // would free the table this CPU's LDTR names (`install` keeps only its
    // address; PR #50 review).
    let old = crate::process::current_process().map(|p| {
        let fresh = new_slot(None);
        let old = core::mem::replace(&mut *p.ldt.lock(), fresh.clone());
        // exec has left one thread, this one, so no task of the process is
        // being started meanwhile; a CLONE_VM sharer keeps the old slot,
        // which no task of this process names any more.
        crate::sched::dispatch::set_process_ldt_slot(p.pid.0, &fresh);
        old
    });
    let irq = crate::arch::disable_interrupts();
    // SAFETY: interrupts off; null selectors are always loadable.
    unsafe {
        install(None);
        hw::load_ds(0);
        hw::load_es(0);
        hw::load_fs(0);
        hw::load_gs(0);
    }
    super::switch::write_fs_base(0);
    super::switch::write_user_gs_base(0);
    drop(irq);
    drop(old);
}

/// After a write to the table this CPU's task uses: reload ds and es if
/// they hold LDT selectors, so their cached descriptors match the table
/// (Linux's refresh_ldt_segments), each checked under the table's lock
/// first and loaded as 0 if its entry is no longer loadable (the
/// loadsegment fixup). Run by the writer and by each CPU the shootdown
/// reaches, with interrupts off.
///
/// # Safety
/// Interrupts off.
#[cfg(all(target_arch = "x86_64", target_os = "none", feature = "alloc"))]
pub unsafe fn refresh_segments() {
    let Some(table) = crate::sched::dispatch::current_ldt() else {
        return;
    };
    // SAFETY: interrupts off; the running task holds the table.
    unsafe { install(Some(&table)) };
    let _hold = table.hold();
    let now = read_segments();
    let fresh = |s: u16| if selector_ok(s, Some(&table)) { s } else { 0 };
    // SAFETY: each selector was checked under the table's lock.
    unsafe {
        if now.ds & 4 != 0 {
            hw::load_ds(fresh(now.ds));
        }
        if now.es & 4 != 0 {
            hw::load_es(fresh(now.es));
        }
    }
}

/// The Linux return convention of modify_ldt: the int result as an
/// unsigned 32-bit value, so an error is not sign-extended into %rax
/// (LTP's lapi/ldt.h undoes this, as glibc does not).
fn int_result(r: Result<usize, crate::syscall::SyscallError>) -> crate::syscall::SyscallResult {
    match r {
        Ok(v) => Ok(v as u32 as usize),
        Err(e) => Ok(crate::syscall::linux_compat::to_linux_errno(e) as i32 as u32 as usize),
    }
}

/// modify_ldt(func, ptr, bytecount): 0 reads the table, 1 writes an entry
/// with the old rules, 2 reads the default table (128 zero bytes), 0x11
/// writes with the new rules; any other func is ENOSYS.
#[cfg(feature = "alloc")]
pub fn sys_modify_ldt(func: usize, ptr: usize, bytecount: usize) -> crate::syscall::SyscallResult {
    use crate::syscall::SyscallError;
    let r = match func as i32 {
        0 => read_ldt(ptr, bytecount),
        1 => write_ldt(ptr, bytecount, true),
        2 => {
            let n = bytecount.min(128);
            crate::syscall::userspace::clear_user(ptr, n).map(|_| n)
        }
        0x11 => write_ldt(ptr, bytecount, false),
        _ => Err(SyscallError::NotImplemented),
    };
    int_result(r)
}

#[cfg(feature = "alloc")]
fn read_ldt(ptr: usize, bytecount: usize) -> Result<usize, crate::syscall::SyscallError> {
    use crate::syscall::{userspace, SyscallError};
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let slot = process.ldt.lock().clone();
    let Some(table) = slot.get().cloned() else {
        return Ok(0);
    };
    let count = bytecount.min(LDT_ENTRIES * LDT_ENTRY_SIZE);
    let size = ((table.nr_entries.load(Ordering::Acquire) as usize) * LDT_ENTRY_SIZE).min(count);
    // Byte for byte, as Linux's copy_to_user of the entries: a count that
    // ends inside a descriptor gets that descriptor's first bytes, so every
    // byte reported is written (PR #50 review).
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size.div_ceil(LDT_ENTRY_SIZE) * LDT_ENTRY_SIZE)
        .map_err(|_| SyscallError::OutOfMemory)?;
    for i in 0..size.div_ceil(LDT_ENTRY_SIZE) {
        bytes.extend_from_slice(&table.entries[i].load(Ordering::Acquire).to_le_bytes());
    }
    bytes.truncate(size);
    userspace::write_user_bytes(ptr, &bytes)?;
    if size != count {
        let rest = ptr.checked_add(size).ok_or(SyscallError::InvalidPointer)?;
        userspace::clear_user(rest, count - size)?;
    }
    Ok(count)
}

#[cfg(feature = "alloc")]
fn write_ldt(
    ptr: usize,
    bytecount: usize,
    oldmode: bool,
) -> Result<usize, crate::syscall::SyscallError> {
    use crate::syscall::{userspace, SyscallError};
    if bytecount != core::mem::size_of::<UserDesc>() {
        return Err(SyscallError::InvalidArgument);
    }
    let w: [u32; 4] = userspace::read_user(ptr)?;
    let info = UserDesc {
        entry_number: w[0],
        base_addr: w[1],
        limit: w[2],
        flags: w[3],
    };
    let desc = descriptor_for(&info, oldmode).ok_or(SyscallError::InvalidArgument)?;
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let slot = process.ldt.lock().clone();
    // Made once, in the slot every task of the address space holds, in
    // every process sharing it (CLONE_VM): each loads it at its next
    // switch, or at its first fault on an LDT selector.
    let table = slot
        .try_call_once(|| Ldt::new().ok_or(SyscallError::OutOfMemory))?
        .clone();
    table.write(info.entry_number as usize, desc);
    // Every CPU running the address space reloads its LDT-backed ds and es
    // before this returns (Linux's install_ldt: on_each_cpu_mm(flush_ldt)).
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        {
            let _irq = crate::arch::disable_interrupts();
            // SAFETY: interrupts off; the running task holds the table.
            unsafe { refresh_segments() };
        }
        crate::mm::tlb::refresh_ldt_everywhere();
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(entry: u32, base: u32, limit: u32, flags: u32) -> UserDesc {
        UserDesc {
            entry_number: entry,
            base_addr: base,
            limit,
            flags,
        }
    }

    /// A flat 32-bit data segment, 4 KiB pages: Linux's fill_ldt result.
    #[test]
    fn fill_ldt_matches_linux() {
        // seg_32bit, contents 0 (data), writable, limit_in_pages, present.
        let d = fill_ldt(&desc(0, 0x1234_5678, 0xfffff, 1 | (1 << 4)));
        assert_eq!(d, 0x12cf_f334_5678_ffff);
        // contents 2 (code), read_exec_only: execute-only code.
        let c = fill_ldt(&desc(0, 0, 0xfffff, 1 | (2 << 1) | (1 << 3) | (1 << 4)));
        assert_eq!((c >> 40) & 0xf, 0b1001);
        assert!(loadable_data(d));
        assert!(
            !loadable_data(c),
            "execute-only code cannot be a data segment"
        );
        assert!(loadable_data(fill_ldt(&desc(
            0,
            0,
            0xfffff,
            1 | (2 << 1) | (1 << 4)
        ))));
        // useable is avl; seg_not_present clears P.
        let u = fill_ldt(&desc(0, 0, 1, 1 | (1 << 6) | (1 << 5)));
        assert!(u & (1 << 52) != 0 && u & (1 << 47) == 0);
        assert!(!loadable_data(u));
    }

    /// write_ldt's EINVAL rules and the clearing descriptions.
    #[test]
    fn write_rules() {
        assert!(descriptor_for(&desc(8192, 0, 0, 0), false).is_none());
        // contents 3 needs the new interface and seg_not_present.
        assert!(descriptor_for(&desc(0, 0, 1, 1 | (3 << 1)), false).is_none());
        assert!(descriptor_for(&desc(0, 0, 1, 1 | (3 << 1) | (1 << 5)), true).is_none());
        assert!(descriptor_for(&desc(0, 0, 1, 1 | (3 << 1) | (1 << 5)), false).is_some());
        // LDT_empty clears; so does a zero base and limit in old mode.
        let empty = desc(5, 0, 0, (1 << 3) | (1 << 5));
        assert_eq!(descriptor_for(&empty, false), Some(0));
        assert_eq!(descriptor_for(&desc(5, 0, 0, 1), true), Some(0));
        // Without X86_16BIT a 16-bit segment is EINVAL.
        assert!(descriptor_for(&desc(0, 0, 1, 0), false).is_none());
        // Old mode drops avl.
        let a = descriptor_for(&desc(0, 0, 1, 1 | (1 << 6)), true).unwrap();
        assert_eq!(a & (1 << 52), 0);
        let b = descriptor_for(&desc(0, 0, 1, 1 | (1 << 6)), false).unwrap();
        assert_ne!(b & (1 << 52), 0);
        // Never a 64-bit segment.
        assert_eq!(b & (1 << 53), 0);
    }

    #[test]
    fn system_descriptor() {
        let (lo, hi) = ldt_system_descriptor(0xffff_8000_1234_5000);
        assert_eq!(lo & 0xffff, 0xffff);
        assert_eq!((lo >> 40) & 0xff, 0x82);
        assert_eq!((lo >> 48) & 0xf, 0);
        assert_eq!(
            ((lo >> 16) & 0xff_ffff) | (((lo >> 56) & 0xff) << 24),
            0x1234_5000
        );
        assert_eq!(hi, 0xffff_8000);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn selectors_against_a_table() {
        let t = Ldt::new().unwrap();
        let data = fill_ldt(&desc(0, 0, 0xfffff, 1 | (1 << 4)));
        t.write(3, data);
        assert_eq!(t.nr_entries.load(Ordering::Relaxed), 4);
        assert!(selector_ok((3 << 3) | 7, Some(&t)));
        assert!(!selector_ok((2 << 3) | 7, Some(&t)), "an empty entry");
        assert!(!selector_ok(7, None), "no table");
        assert!(selector_ok(0x2b, None), "GDT user selectors stay");
        assert!(selector_ok(0x23, None) && selector_ok(0x33, None));
        assert!(
            selector_ok(0, None) && selector_ok(3, None),
            "null at any RPL"
        );
        // A user descriptor loads at any RPL (DPL 3 >= max(CPL, RPL)).
        for sel in [0x20, 0x28, 0x29, 0x2a, 0x30] {
            assert!(selector_ok(sel, None), "{sel:#x}");
        }
        // A null, kernel or system GDT descriptor would fault if loaded at
        // a switch (PR #50 review).
        for sel in [0x08, 0x10, 0x18, 0x38, 0x40, 0x48, 0x50, 0x58, 0x0b] {
            assert!(!selector_ok(sel, None), "{sel:#x}");
        }
        let copy = t.duplicate().unwrap();
        assert_eq!(copy.entry((3 << 3) | 7), Some(data));
        t.write(3, 0);
        assert_eq!(copy.entry((3 << 3) | 7), Some(data), "the copy is apart");
    }

    /// A slot is set once: a second first-write finds the table the first
    /// made, and a slot cloned for a sharer (CLONE_VM) sees a table made
    /// after the clone (PR #50 review).
    #[test]
    fn a_slot_is_made_once_and_shared() {
        let slot = new_slot(None);
        let sharer = slot.clone();
        assert!(sharer.get().is_none());
        let a = slot.try_call_once(|| Ldt::new().ok_or(())).unwrap().clone();
        let b = sharer
            .try_call_once(|| Ldt::new().ok_or(()))
            .unwrap()
            .clone();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(Arc::ptr_eq(sharer.get().unwrap(), &a));
        // A fork child's slot holds its copy from the start.
        let child = new_slot(Some(a.duplicate().unwrap()));
        assert!(!Arc::ptr_eq(child.get().unwrap(), &a));
    }
}

//! Page Fault Handler Framework
//!
//! Provides infrastructure for handling page faults including demand paging,
//! copy-on-write, and stack growth. Architecture-specific trap handlers
//! construct a [`PageFaultInfo`] and delegate to [`handle_page_fault`].

// Page fault handler -- demand paging, CoW, stack growth
#![allow(dead_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

use crate::{
    error::KernelError,
    mm::{PageFlags, VirtualAddress, PAGE_SIZE},
};

/// Reason a page fault occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFaultReason {
    /// Page is not present in the page table.
    NotPresent,
    /// A protection violation was detected (e.g., access rights mismatch).
    ProtectionViolation,
    /// Write to a read-only page.
    WriteToReadOnly,
    /// Attempt to execute a page marked as no-execute.
    ExecuteNoExecute,
    /// User-mode code tried to access a kernel-only page.
    UserModeKernelAccess,
}

/// Information about a page fault collected by the architecture trap handler.
#[derive(Debug, Clone, Copy)]
pub struct PageFaultInfo {
    /// The virtual address that caused the fault.
    pub faulting_address: u64,
    /// Why the fault occurred.
    pub reason: PageFaultReason,
    /// Whether the access was a write (true) or read (false).
    pub was_write: bool,
    /// Whether the fault occurred while executing in user mode.
    pub was_user_mode: bool,
    /// Whether the access was an instruction fetch (a retry must then find
    /// the page executable).
    pub was_fetch: bool,
    /// Instruction pointer at the time of the fault.
    pub instruction_pointer: u64,
}

/// Default stack guard region size (one page below the mapped stack).
const _STACK_GUARD_SIZE: usize = PAGE_SIZE;

/// Gap kept free below a growing stack (Linux `stack_guard_gap`).
const STACK_GUARD_GAP: u64 = 1024 * 1024;

/// Main page fault handler.
///
/// Dispatches the fault to the appropriate sub-handler:
/// 1. **Demand paging** -- the address is within a valid VAS mapping but the
///    physical page has not been allocated yet.
/// 2. **Copy-on-Write** -- the page is marked read-only for CoW; allocate a
///    private copy and remap as writable.
/// 3. **Stack growth** -- the faulting address is just below the current stack
///    mapping; extend the stack downward.
/// 4. If none of the above apply, return an error. The caller (the
///    architecture's trap handler) sends the signal: it knows the cause and
///    builds the siginfo, so a fault is signalled exactly once.
pub fn handle_page_fault(info: PageFaultInfo) -> Result<(), KernelError> {
    // Attempt demand paging first.
    if let Ok(()) = try_demand_page(&info) {
        return Ok(());
    }

    // Attempt copy-on-write handling.
    if info.was_write {
        if let Ok(()) = try_copy_on_write(&info) {
            return Ok(());
        }
    }

    // Attempt stack growth.
    if let Ok(()) = try_stack_growth(&info) {
        return Ok(());
    }

    // None of the handlers could resolve the fault.
    Err(KernelError::InvalidAddress {
        addr: info.faulting_address as usize,
    })
}

// ---------------------------------------------------------------------------
// Sub-handlers
// ---------------------------------------------------------------------------

/// Try to resolve the fault via demand paging.
///
/// If the faulting address falls within a valid VAS mapping that simply has
/// not been backed by a physical frame yet, allocate a frame and install the
/// mapping.
fn try_demand_page(info: &PageFaultInfo) -> Result<(), KernelError> {
    let process = crate::process::current_process().ok_or(KernelError::NotInitialized {
        subsystem: "process",
    })?;
    try_demand_page_in(&process, info, info.was_user_mode)
}

/// Resolve a fault the kernel took while copying to or from user memory
/// (x86_64 `usercopy`). Returns whether the page is now present.
///
/// Deliberately narrower than [`handle_page_fault`]:
/// - not-present faults, and writes to copy-on-write pages of mappings the
///   process may write: any other protection fault on a present page is never
///   "fixed" on the kernel's behalf, so a syscall can never write a page the
///   process itself may not write (a `read()` into a buffer shared after a fork
///   would otherwise fail with EFAULT);
/// - demand paging, checked against the mapping as a *user* access (USER
///   required, WRITABLE required for writes), and CoW -- no stack growth;
/// - no blocking on the scheduler lock, and never a signal: on any failure the
///   copy returns EFAULT instead.
pub fn resolve_user_copy_fault(info: &PageFaultInfo) -> bool {
    let Some(process) = crate::process::try_current_process() else {
        return false;
    };
    if info.reason != PageFaultReason::NotPresent {
        return info.was_write && resolve_cow_in(&process, info, false).is_ok();
    }
    let as_user = PageFaultInfo {
        was_user_mode: true,
        ..*info
    };
    // Never sleeps: the system call may hold a spinlock (security review
    // of d1e2b47); a held address-space lock gives EFAULT, as before.
    // RISC-V's scause does not tell a missing page from a protection fault
    // (`from_riscv` reports NotPresent for both), so a write to a present
    // page still shared after a fork arrives here: it is resolved as CoW,
    // which acts only on pages fork marked copy-on-write.
    try_demand_page_in(&process, &as_user, false).is_ok()
        || (info.was_write && resolve_cow_in(&process, info, false).is_ok())
}

/// The SIGSEGV code for a user fault at `addr` that could not be resolved,
/// as Linux chooses it: SEGV_ACCERR when a mapping covers the address (the
/// access was not allowed), SEGV_MAPERR when none does. For RISC-V, whose
/// scause does not tell the two apart (`from_riscv`). Called from a user
/// fault, where the thread holds no kernel lock.
pub fn user_segv_code(addr: u64) -> i32 {
    #[cfg(feature = "alloc")]
    let mapped = crate::process::try_current_process().is_some_and(|p| {
        p.memory_space
            .lock_if_may_sleep(true)
            .is_some_and(|ms| ms.find_mapping(VirtualAddress::new(addr)).is_some())
    });
    #[cfg(not(feature = "alloc"))]
    let mapped = {
        let _ = addr;
        false
    };
    if mapped {
        crate::process::signals::SEGV_ACCERR
    } else {
        crate::process::signals::SEGV_MAPERR
    }
}

/// `may_sleep`: whether the address-space lock may be waited for. Only a
/// fault taken in user mode may: the thread holds no kernel lock then. A
/// fault in kernel mode -- a system call copying user memory, perhaps under
/// a spinlock -- only tries it.
fn try_demand_page_in(
    process: &crate::process::Process,
    info: &PageFaultInfo,
    may_sleep: bool,
) -> Result<(), KernelError> {
    let vaddr = VirtualAddress::new(info.faulting_address);

    // Check whether the faulting address is within any existing mapping.
    // The lock sleeps if another thread holds it (N-138); it is refused
    // only when this thread holds it itself (a system call copying user
    // memory under it), where waiting would deadlock.
    let memory_space =
        process
            .memory_space
            .lock_if_may_sleep(may_sleep)
            .ok_or(KernelError::NotInitialized {
                subsystem: "memory_space (held by this thread)",
            })?;

    #[cfg(feature = "alloc")]
    {
        // Another thread of this process mapped the page since this one
        // faulted, or was replacing it (a copy-on-write copy unmaps the
        // shared page first), or this CPU still caches the old entry (a
        // write to a page another CPU has just made writable): retry the
        // access, as Linux does with a spurious fault. Whatever the fault's
        // reason, but only if the page now allows exactly this access --
        // including execution for an instruction fetch -- so a real fault
        // is never retried forever. The local flush drops the stale
        // translation (PR #19, #20 reviews).
        if memory_space.page_allows(
            info.faulting_address,
            info.was_write,
            info.was_user_mode,
            info.was_fetch,
        ) {
            crate::arch::tlb_flush_address(info.faulting_address & !(PAGE_SIZE as u64 - 1));
            return Ok(());
        }

        let mapping = memory_space.find_mapping(vaddr);
        match mapping {
            Some(m) => {
                // The address is in a valid mapping. Check whether the page
                // has already been backed (has a physical frame).
                let offset = info.faulting_address - m.start.as_u64();
                let page_index = (offset / PAGE_SIZE as u64) as usize;

                if page_index < m.physical_frames.len() {
                    // Frame already exists -- this is not a demand-page fault.
                    return Err(KernelError::InvalidAddress {
                        addr: info.faulting_address as usize,
                    });
                }

                // Permission check: the access type must be allowed by the mapping.
                if info.was_write && !m.flags.contains(PageFlags::WRITABLE) {
                    return Err(KernelError::PermissionDenied {
                        operation: "write to read-only mapping",
                    });
                }

                if info.was_user_mode && !m.flags.contains(PageFlags::USER) {
                    return Err(KernelError::PermissionDenied {
                        operation: "user access to kernel mapping",
                    });
                }

                // Allocate a frame and map the page.
                // We need to drop the lock before mutating the VAS.
                drop(memory_space);

                let page_addr = (info.faulting_address & !(PAGE_SIZE as u64 - 1)) as usize;
                let mut memory_space_mut = process
                    .memory_space
                    .lock_if_may_sleep(may_sleep)
                    .ok_or(KernelError::NotInitialized {
                        subsystem: "memory_space (lock held, map)",
                    })?;
                memory_space_mut.map_page(page_addr, m.flags)?;

                Ok(())
            }
            None => Err(KernelError::UnmappedMemory {
                addr: info.faulting_address as usize,
            }),
        }
    }

    #[cfg(not(feature = "alloc"))]
    {
        let _ = vaddr;
        Err(KernelError::NotImplemented {
            feature: "demand paging (requires alloc)",
        })
    }
}

/// Try to resolve the fault via copy-on-write.
///
/// If the page is mapped read-only and is marked for CoW, create a private
/// copy of the page, map it as writable, and return success.
fn try_copy_on_write(info: &PageFaultInfo) -> Result<(), KernelError> {
    let process = crate::process::current_process().ok_or(KernelError::NotInitialized {
        subsystem: "process",
    })?;
    resolve_cow_in(&process, info, info.was_user_mode)
}

/// Resolve a write to a copy-on-write page of `process` (see
/// `VirtualAddressSpace::resolve_cow_fault`). `may_sleep` as for
/// `try_demand_page_in`.
fn resolve_cow_in(
    process: &crate::process::Process,
    info: &PageFaultInfo,
    may_sleep: bool,
) -> Result<(), KernelError> {
    #[cfg(feature = "alloc")]
    {
        let memory_space = process.memory_space.lock_if_may_sleep(may_sleep).ok_or(
            KernelError::NotInitialized {
                subsystem: "memory_space (lock held)",
            },
        )?;
        if memory_space.resolve_cow_fault(info.faulting_address)? {
            return Ok(());
        }
    }
    #[cfg(not(feature = "alloc"))]
    let _ = (process, info);
    Err(KernelError::InvalidAddress {
        addr: info.faulting_address as usize,
    })
}

/// Try to resolve the fault by growing the user stack.
///
/// The stack grows downward. If the faulting address is below the current
/// stack mapping, the whole stack would stay within the process's
/// RLIMIT_STACK (8 MiB by default; Linux's acct_stack_growth) and the new
/// pages within RLIMIT_AS, and no other mapping is within the guard gap,
/// we extend the stack by mapping new pages.
/// Whether a stack whose top is `stack_top` may grow down to the page at
/// `fault_page` under an RLIMIT_STACK of `limit` bytes: the whole stack,
/// in pages, within the limit (Linux's acct_stack_growth). The fault's
/// byte address was compared, so a limit one byte above the stack let it
/// grow a page past the limit (PR #22 review).
fn stack_growth_allowed(stack_top: u64, fault_page: u64, limit: u64) -> bool {
    stack_top
        .checked_sub(fault_page)
        .is_some_and(|size| size <= limit)
}

fn try_stack_growth(info: &PageFaultInfo) -> Result<(), KernelError> {
    // Stack growth only applies to user-mode faults.
    if !info.was_user_mode {
        return Err(KernelError::PermissionDenied {
            operation: "kernel stack growth not supported",
        });
    }

    let process = crate::process::current_process().ok_or(KernelError::NotInitialized {
        subsystem: "process",
    })?;
    // Only user-mode faults get here (checked above): no kernel lock held.
    let may_sleep = true;

    // One lock from the checks to the mapping: released in between,
    // another thread could map into the checked gap (which map_page would
    // then make writable) or use up RLIMIT_AS, and two growth faults could
    // record different sizes (PR #22 review).
    let mut memory_space =
        process
            .memory_space
            .lock_if_may_sleep(may_sleep)
            .ok_or(KernelError::NotInitialized {
                subsystem: "memory_space (lock held, stack)",
            })?;
    let stack_top = memory_space.stack_top() as u64;
    let stack_size = memory_space.user_stack_size() as u64;
    let stack_bottom = stack_top - stack_size;

    // Check if the fault is just below the current stack bottom.
    let fault = info.faulting_address;
    if fault >= stack_bottom {
        // Already within the stack region -- not a growth fault.
        return Err(KernelError::InvalidAddress {
            addr: fault as usize,
        });
    }

    // The stack may span at most RLIMIT_STACK below its top in total.
    // (This used to be measured from the already-grown size, so the limit
    // moved down with every growth and the stack had none; N-136.)
    let limits = process.limits();
    let fault_page = fault & !(PAGE_SIZE as u64 - 1);
    if !stack_growth_allowed(
        stack_top,
        fault_page,
        limits.cur(crate::process::rlimit::RLIMIT_STACK),
    ) {
        // Too far below the stack -- real SIGSEGV.
        return Err(KernelError::InvalidAddress {
            addr: fault as usize,
        });
    }

    // Calculate how many pages to grow (from current bottom down to fault).
    let pages_needed = ((stack_bottom - fault_page) / PAGE_SIZE as u64) as usize;

    if pages_needed == 0 {
        return Err(KernelError::InvalidAddress {
            addr: fault as usize,
        });
    }

    // Never grow into, or within a guard gap of, another mapping: growing
    // over one rewrote its protection to read-write (N-136; Linux keeps
    // `stack_guard_gap`, 1 MiB).
    #[cfg(feature = "alloc")]
    {
        let gap_bottom = fault_page.saturating_sub(STACK_GUARD_GAP);
        let usage = memory_space.vm_usage(0, 0);
        let bytes = (pages_needed * PAGE_SIZE) as u64;
        if !memory_space.range_is_free(gap_bottom, stack_bottom)
            || !crate::process::rlimit::may_expand_vm(&limits, usage, bytes, false)
        {
            return Err(KernelError::InvalidAddress {
                addr: fault as usize,
            });
        }
    }

    let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE;
    for i in 0..pages_needed {
        let page_addr = (stack_bottom - ((i + 1) as u64 * PAGE_SIZE as u64)) as usize;
        memory_space.map_page(page_addr, flags)?;
    }

    // Update stack size to reflect the growth so future faults work correctly.
    let new_size = stack_size as usize + (pages_needed * PAGE_SIZE);
    memory_space.set_stack_size(new_size);

    Ok(())
}

// ---------------------------------------------------------------------------
// Architecture-specific entry points
// ---------------------------------------------------------------------------

/// Build a [`PageFaultInfo`] from an x86_64 page fault error code and CR2.
///
/// Error code bits (from Intel SDM):
/// - Bit 0 (P):    0 = not-present, 1 = protection violation
/// - Bit 1 (W/R):  0 = read, 1 = write
/// - Bit 2 (U/S):  0 = supervisor, 1 = user
/// - Bit 4 (I/D):  1 = instruction fetch
#[cfg(target_arch = "x86_64")]
pub fn from_x86_64(error_code: u64, cr2: u64, rip: u64) -> PageFaultInfo {
    let not_present = (error_code & 1) == 0;
    let was_write = (error_code & 2) != 0;
    let was_user = (error_code & 4) != 0;
    let was_fetch = (error_code & 16) != 0;

    let reason = if not_present {
        PageFaultReason::NotPresent
    } else if was_fetch {
        PageFaultReason::ExecuteNoExecute
    } else if was_write {
        PageFaultReason::WriteToReadOnly
    } else if was_user {
        PageFaultReason::UserModeKernelAccess
    } else {
        PageFaultReason::ProtectionViolation
    };

    PageFaultInfo {
        faulting_address: cr2,
        reason,
        was_write,
        was_user_mode: was_user,
        was_fetch,
        instruction_pointer: rip,
    }
}

/// Build a [`PageFaultInfo`] from an AArch64 data/instruction abort.
///
/// `esr_el1` contains the ESR value and `far_el1` the faulting address.
/// ISS encoding for Data Abort (EC=0b100100/0b100101):
/// - Bit 6 (WnR): 0 = read, 1 = write
/// - Bits [5:0] (DFSC): fault status code
// Pure decoding: built everywhere so host tests check it.
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub fn from_aarch64(esr_el1: u64, far_el1: u64, elr_el1: u64) -> PageFaultInfo {
    let dfsc = (esr_el1 & 0x3F) as u8;
    // EC field is bits [31:26]
    let ec = ((esr_el1 >> 26) & 0x3F) as u8;
    // A data abort (0x24/0x25) says in WnR whether it was a write; an
    // instruction abort (0x20/0x21) is a fetch.
    let was_write = matches!(ec, 0x24 | 0x25) && (esr_el1 & (1 << 6)) != 0;
    // From a lower EL (user mode): instruction abort 0x20, data abort 0x24.
    let was_user = matches!(ec, 0x20 | 0x24);
    let was_fetch = matches!(ec, 0x20 | 0x21);

    // The full fault status code: a synchronous external abort on a table
    // walk (0x14-0x17) must not read as a translation fault.
    let reason = match dfsc {
        // Translation faults (levels 0-3)
        0x04..=0x07 => PageFaultReason::NotPresent,
        // Permission faults (levels 0-3): on an instruction abort, a fetch
        // from a no-execute page, as x86_64 and RISC-V report it.
        0x0C..=0x0F => {
            if was_fetch {
                PageFaultReason::ExecuteNoExecute
            } else if was_write {
                PageFaultReason::WriteToReadOnly
            } else {
                PageFaultReason::ProtectionViolation
            }
        }
        _ => PageFaultReason::ProtectionViolation,
    };

    PageFaultInfo {
        faulting_address: far_el1,
        reason,
        was_write,
        was_user_mode: was_user,
        was_fetch,
        instruction_pointer: elr_el1,
    }
}

/// Build a [`PageFaultInfo`] from a RISC-V page fault trap.
///
/// RISC-V uses different exception codes for load, store, and instruction
/// page faults (causes 12, 13, 15 respectively).
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub fn from_riscv(cause: u64, stval: u64, sepc: u64) -> PageFaultInfo {
    let was_write = cause == 15; // Store/AMO page fault
    let was_fetch = cause == 12; // Instruction page fault
                                 // cause == 13 is load page fault

    // RISC-V does not encode present vs. permission in the cause alone;
    // the PTE must be inspected. Default to NotPresent and let the handler
    // check VAS mappings.
    let reason = if was_fetch {
        PageFaultReason::ExecuteNoExecute
    } else {
        PageFaultReason::NotPresent
    };

    // User-mode faults come from U-mode; the SPP bit of sstatus indicates
    // whether the previous privilege was S-mode. We conservatively mark all
    // page faults as user-mode here; the caller can refine using sstatus.
    PageFaultInfo {
        faulting_address: stval,
        reason,
        was_write,
        was_user_mode: true,
        was_fetch,
        instruction_pointer: sepc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_growth_counts_whole_pages_against_the_limit() {
        let top = 0x7FFF_0000_0000u64;
        let size = 8 * 4096;
        let bottom = top - size;
        // A limit one byte above the stack: the next page would pass it.
        assert!(!stack_growth_allowed(top, bottom - 4096, size + 1));
        assert!(stack_growth_allowed(top, bottom - 4096, size + 4096));
        assert!(stack_growth_allowed(top, 0, u64::MAX)); // RLIM_INFINITY
    }

    #[test]
    fn test_page_fault_reason_equality() {
        assert_eq!(PageFaultReason::NotPresent, PageFaultReason::NotPresent);
        assert_ne!(
            PageFaultReason::NotPresent,
            PageFaultReason::WriteToReadOnly
        );
    }

    /// An instruction fetch is reported as one on every architecture, so a
    /// spurious-fault retry checks execute permission and a fetch from a
    /// no-execute page faults instead of retrying forever (PR #20 review).
    #[test]
    fn fetch_faults_say_so() {
        // x86_64: P=1, U=1, I/D=1 (a fetch from a present NX page).
        let x = from_x86_64(0x15, 0x4000, 0x4000);
        assert!(x.was_fetch && x.reason == PageFaultReason::ExecuteNoExecute);
        assert!(!from_x86_64(0x7, 0x4000, 0x1000).was_fetch);
        // AArch64: instruction abort from EL0 (EC 0x20), permission fault
        // level 3 (IFSC 0x0F).
        let a = from_aarch64((0x20 << 26) | 0x0F, 0x4000, 0x4000);
        assert!(a.was_fetch && a.reason == PageFaultReason::ExecuteNoExecute);
        // A translation fault on a fetch is still "not present".
        let a = from_aarch64((0x20 << 26) | 0x07, 0x4000, 0x4000);
        assert!(a.was_fetch && a.reason == PageFaultReason::NotPresent);
        // A data abort read permission fault is no fetch.
        let d = from_aarch64((0x24 << 26) | 0x0F, 0x4000, 0x1000);
        assert!(!d.was_fetch && d.reason == PageFaultReason::ProtectionViolation);
    }

    #[test]
    fn test_page_fault_info_construction() {
        let info = PageFaultInfo {
            faulting_address: 0xDEAD_BEEF,
            reason: PageFaultReason::NotPresent,
            was_write: false,
            was_user_mode: true,
            was_fetch: false,
            instruction_pointer: 0x4010_0000,
        };
        assert_eq!(info.faulting_address, 0xDEAD_BEEF);
        assert!(!info.was_write);
        assert!(info.was_user_mode);
    }

    #[test]
    fn test_page_fault_info_write_fault() {
        let info = PageFaultInfo {
            faulting_address: 0x1000,
            reason: PageFaultReason::WriteToReadOnly,
            was_write: true,
            was_user_mode: true,
            was_fetch: false,
            instruction_pointer: 0x2000,
        };
        assert!(info.was_write);
        assert_eq!(info.reason, PageFaultReason::WriteToReadOnly);
    }

    #[test]
    fn test_page_fault_info_kernel_fault() {
        let info = PageFaultInfo {
            faulting_address: 0xFFFF_8000_0000_1000,
            reason: PageFaultReason::ProtectionViolation,
            was_write: false,
            was_user_mode: false,
            was_fetch: false,
            instruction_pointer: 0xFFFF_8000_0010_0000,
        };
        assert!(!info.was_user_mode);
    }
}

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
    /// Instruction pointer at the time of the fault.
    pub instruction_pointer: u64,
}

/// Default stack guard region size (one page below the mapped stack).
const _STACK_GUARD_SIZE: usize = PAGE_SIZE;

/// Maximum stack growth beyond initial allocation (8 MiB).
/// cc1 (GCC compiler proper) uses deep recursion for complex expressions.
const MAX_STACK_GROWTH: usize = 8 * 1024 * 1024;

/// Largest total user stack: the default initial stack plus the growth
/// allowance, fixed (not measured from the current size; N-136).
const STACK_LIMIT: usize = crate::process::creation::DEFAULT_USER_STACK_SIZE + MAX_STACK_GROWTH;

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
/// 4. If none of the above apply, deliver SIGSEGV / return an error.
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
    signal_segv(&info)
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
    try_demand_page_in(&process, info)
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
        return info.was_write && resolve_cow_in(&process, info).is_ok();
    }
    let as_user = PageFaultInfo {
        was_user_mode: true,
        ..*info
    };
    try_demand_page_in(&process, &as_user).is_ok()
}

fn try_demand_page_in(
    process: &crate::process::Process,
    info: &PageFaultInfo,
) -> Result<(), KernelError> {
    let vaddr = VirtualAddress::new(info.faulting_address);

    // Check whether the faulting address is within any existing mapping.
    // Use try_lock() to avoid deadlock: the page fault handler runs from IST
    // interrupt context, so if the syscall path already holds memory_space.lock(),
    // a blocking .lock() would spin forever and GP fault on the IST stack.
    let memory_space = process
        .memory_space
        .try_lock()
        .ok_or(KernelError::NotInitialized {
            subsystem: "memory_space (lock held)",
        })?;

    #[cfg(feature = "alloc")]
    {
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
                let mut memory_space_mut =
                    process
                        .memory_space
                        .try_lock()
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
    resolve_cow_in(&process, info)
}

/// Resolve a write to a copy-on-write page of `process` (see
/// `VirtualAddressSpace::resolve_cow_fault`). Uses `try_lock`: the fault may
/// have interrupted code that holds the memory-space lock.
fn resolve_cow_in(
    process: &crate::process::Process,
    info: &PageFaultInfo,
) -> Result<(), KernelError> {
    #[cfg(feature = "alloc")]
    {
        let memory_space = process
            .memory_space
            .try_lock()
            .ok_or(KernelError::NotInitialized {
                subsystem: "memory_space (lock held)",
            })?;
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
/// The stack grows downward. If the faulting address is within one
/// [`MAX_STACK_GROWTH`] below the current stack mapping and above the stack
/// guard page, we extend the stack by mapping new pages.
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

    let memory_space = process
        .memory_space
        .try_lock()
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

    // The stack may span at most STACK_LIMIT below its top in total.
    // (This used to be measured from the already-grown size, so the limit
    // moved down with every growth and the stack had none; N-136.)
    let absolute_bottom = stack_top.saturating_sub(STACK_LIMIT as u64);
    if fault < absolute_bottom {
        // Too far below the stack -- real SIGSEGV.
        return Err(KernelError::InvalidAddress {
            addr: fault as usize,
        });
    }

    // Calculate how many pages to grow (from current bottom down to fault).
    let fault_page = fault & !(PAGE_SIZE as u64 - 1);
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
        if !memory_space.range_is_free(gap_bottom, stack_bottom) {
            return Err(KernelError::InvalidAddress {
                addr: fault as usize,
            });
        }
    }

    // Drop the lock and re-acquire as mutable to map pages.
    drop(memory_space);

    let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE;

    let mut memory_space_mut =
        process
            .memory_space
            .try_lock()
            .ok_or(KernelError::NotInitialized {
                subsystem: "memory_space (lock held, stack map)",
            })?;
    for i in 0..pages_needed {
        let page_addr = (stack_bottom - ((i + 1) as u64 * PAGE_SIZE as u64)) as usize;
        memory_space_mut.map_page(page_addr, flags)?;
    }

    // Update stack size to reflect the growth so future faults work correctly.
    let new_size = stack_size as usize + (pages_needed * PAGE_SIZE);
    memory_space_mut.set_stack_size(new_size);

    Ok(())
}

/// Deliver SIGSEGV to the faulting process or return an error for kernel
/// faults.
fn signal_segv(info: &PageFaultInfo) -> Result<(), KernelError> {
    if info.was_user_mode {
        // Attempt to deliver SIGSEGV to the process.
        if let Some(process) = crate::process::current_process() {
            use crate::process::exit::signals::SIGSEGV;
            if let Err(_e) = crate::process::exit::kill_process(process.pid, SIGSEGV) {
                kprintln!(
                    "[MM] Warning: Failed to deliver SIGSEGV to pid {}: {:?}",
                    process.pid,
                    _e
                );
            }
        }
    }

    Err(KernelError::InvalidAddress {
        addr: info.faulting_address as usize,
    })
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
        instruction_pointer: rip,
    }
}

/// Build a [`PageFaultInfo`] from an AArch64 data/instruction abort.
///
/// `esr_el1` contains the ESR value and `far_el1` the faulting address.
/// ISS encoding for Data Abort (EC=0b100100/0b100101):
/// - Bit 6 (WnR): 0 = read, 1 = write
/// - Bits [5:0] (DFSC): fault status code
#[cfg(target_arch = "aarch64")]
pub fn from_aarch64(esr_el1: u64, far_el1: u64, elr_el1: u64) -> PageFaultInfo {
    let dfsc = (esr_el1 & 0x3F) as u8;
    let was_write = (esr_el1 & (1 << 6)) != 0;
    // EC field is bits [31:26]
    let ec = ((esr_el1 >> 26) & 0x3F) as u8;
    // If EC == 0b100100 the abort came from a lower EL (user mode)
    let was_user = ec == 0b100100;

    let reason = match dfsc & 0x0F {
        // Translation faults (levels 0-3)
        0x04..=0x07 => PageFaultReason::NotPresent,
        // Permission faults (levels 0-3)
        0x0C..=0x0F => {
            if was_write {
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
        instruction_pointer: sepc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_fault_reason_equality() {
        assert_eq!(PageFaultReason::NotPresent, PageFaultReason::NotPresent);
        assert_ne!(
            PageFaultReason::NotPresent,
            PageFaultReason::WriteToReadOnly
        );
    }

    #[test]
    fn test_page_fault_info_construction() {
        let info = PageFaultInfo {
            faulting_address: 0xDEAD_BEEF,
            reason: PageFaultReason::NotPresent,
            was_write: false,
            was_user_mode: true,
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
            instruction_pointer: 0xFFFF_8000_0010_0000,
        };
        assert!(!info.was_user_mode);
    }
}

//! Page Fault Handler Framework
//!
//! Provides infrastructure for handling page faults including demand paging,
//! copy-on-write, and stack growth. Architecture-specific trap handlers
//! construct a [`PageFaultInfo`] and delegate to [`handle_page_fault`].

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
const STACK_GUARD_SIZE: usize = PAGE_SIZE;

/// Maximum stack growth per fault (128 KiB).
const MAX_STACK_GROWTH: usize = 128 * 1024;

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

    let vaddr = VirtualAddress::new(info.faulting_address);

    // Check whether the faulting address is within any existing mapping.
    let memory_space = process.memory_space.lock();

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
                let mut memory_space_mut = process.memory_space.lock();
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
    // Copy-on-write requires detecting CoW-marked pages. The current VAS
    // implementation does not track CoW state, so we cannot resolve it yet.
    // When CoW tracking is added (e.g., a `cow: bool` field on
    // VirtualMapping), this handler will:
    //   1. Allocate a new physical frame.
    //   2. Copy the old frame's contents to the new frame.
    //   3. Remap the page as writable with the new frame.
    //   4. Flush the TLB entry.
    let _ = info;
    Err(KernelError::NotImplemented {
        feature: "copy-on-write page handling",
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

    let memory_space = process.memory_space.lock();
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

    // Guard page check: do not grow into the guard region.
    let guard_bottom = stack_bottom.saturating_sub(STACK_GUARD_SIZE as u64);
    if fault < guard_bottom.saturating_sub(MAX_STACK_GROWTH as u64) {
        // Too far below the stack -- likely a real SIGSEGV.
        return Err(KernelError::InvalidAddress {
            addr: fault as usize,
        });
    }

    if fault < guard_bottom {
        // Inside the guard page region.
        return Err(KernelError::PermissionDenied {
            operation: "stack guard page hit",
        });
    }

    // Calculate how many pages to grow.
    let pages_needed = (stack_bottom - fault).div_ceil(PAGE_SIZE as u64) as usize;

    // Cap at MAX_STACK_GROWTH worth of pages.
    let max_pages = MAX_STACK_GROWTH / PAGE_SIZE;
    if pages_needed > max_pages {
        return Err(KernelError::ResourceExhausted {
            resource: "stack growth limit",
        });
    }

    // Drop the read lock and re-acquire as mutable to map pages.
    drop(memory_space);

    let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER | PageFlags::NO_EXECUTE;

    let mut memory_space_mut = process.memory_space.lock();
    for i in 0..pages_needed {
        let page_addr = (stack_bottom - ((i + 1) as u64 * PAGE_SIZE as u64)) as usize;
        memory_space_mut.map_page(page_addr, flags)?;
    }

    Ok(())
}

/// Deliver SIGSEGV to the faulting process or return an error for kernel
/// faults.
fn signal_segv(info: &PageFaultInfo) -> Result<(), KernelError> {
    if info.was_user_mode {
        // Attempt to deliver SIGSEGV to the process.
        if let Some(process) = crate::process::current_process() {
            use crate::process::exit::signals::SIGSEGV;
            let _ = crate::process::exit::kill_process(process.pid, SIGSEGV);
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

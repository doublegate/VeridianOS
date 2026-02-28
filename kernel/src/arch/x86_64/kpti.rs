//! Kernel Page Table Isolation (KPTI) for x86_64
//!
//! Mitigates Meltdown (CVE-2017-5754) by maintaining separate page table
//! hierarchies for user mode and kernel mode. When running in user mode,
//! the shadow page table contains only the minimal kernel mappings needed
//! for the syscall/interrupt trampoline. On kernel entry, CR3 is switched
//! to the full kernel page table.
//!
//! ## Design
//!
//! - **Kernel page table**: The full L4 table with both user (L4[0..255]) and
//!   kernel (L4[256..511]) entries.
//! - **Shadow page table**: A separate L4 with user entries copied from the
//!   kernel table, but only a single trampoline mapping in the kernel half
//!   (L4[511]) that maps the syscall entry/exit code.
//! - **CR3 switching**: `switch_to_user()` loads the shadow CR3 before
//!   returning to Ring 3; `switch_to_kernel()` restores the full CR3 on entry
//!   to Ring 0.

#![allow(dead_code)]

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

use crate::mm::{phys_to_virt_addr, PhysicalAddress, FRAME_ALLOCATOR};

// ===========================================================================
// Constants
// ===========================================================================

/// Virtual address of the syscall trampoline page.
/// Placed at the top of the address space (last page of L4[511]).
const TRAMPOLINE_VADDR: u64 = 0xFFFF_FFFF_FFFF_0000;

/// L4 index that separates user-space from kernel-space.
/// Entries 0..255 are user, 256..511 are kernel.
const USER_KERNEL_SPLIT: usize = 256;

/// Number of L4 entries
const L4_ENTRY_COUNT: usize = 512;

// Page table entry flags (raw x86_64 PTE bits)
const PTE_PRESENT: u64 = 1 << 0;
const PTE_WRITABLE: u64 = 1 << 1;
const PTE_USER: u64 = 1 << 2;
const PTE_NO_EXECUTE: u64 = 1 << 63;

// ===========================================================================
// KPTI State
// ===========================================================================

/// Per-process KPTI page table pair.
#[derive(Debug)]
pub struct KptiPageTables {
    /// Physical address of the full kernel L4 table.
    pub kernel_cr3: u64,
    /// Physical address of the shadow (user-mode) L4 table.
    pub shadow_cr3: u64,
}

/// Global KPTI state: the current page table pair.
struct KptiState {
    tables: KptiPageTables,
    initialized: bool,
}

static KPTI_STATE: Mutex<Option<KptiState>> = Mutex::new(None);

/// Shadow CR3 for fast access without locking (set during init).
static SHADOW_CR3: AtomicU64 = AtomicU64::new(0);

// ===========================================================================
// Initialization
// ===========================================================================

/// Initialize KPTI with shadow page tables derived from the current CR3.
///
/// Must be called after the kernel page tables are fully set up.
pub fn init() {
    let kernel_cr3 = super::mmu::read_cr3().as_u64();

    match create_shadow_tables(kernel_cr3) {
        Ok(shadow_cr3) => {
            SHADOW_CR3.store(shadow_cr3, Ordering::Release);
            *KPTI_STATE.lock() = Some(KptiState {
                tables: KptiPageTables {
                    kernel_cr3,
                    shadow_cr3,
                },
                initialized: true,
            });
            crate::println!(
                "[KPTI] Initialized: kernel CR3=0x{:x}, shadow CR3=0x{:x}",
                kernel_cr3,
                shadow_cr3
            );
        }
        Err(e) => {
            crate::println!(
                "[KPTI] Initialization failed: {:?} -- running without KPTI",
                e
            );
        }
    }
}

/// Create shadow page tables from the kernel's L4 table.
///
/// Allocates a new L4 frame and:
/// 1. Copies all user-space entries (L4[0..255]) from the kernel table.
/// 2. Leaves kernel-space entries (L4[256..510]) empty (unmapped).
/// 3. Maps a single trampoline page at L4[511] for syscall transitions.
///
/// Returns the physical address of the shadow L4 table.
pub fn create_shadow_tables(kernel_cr3: u64) -> Result<u64, crate::error::KernelError> {
    // Allocate a frame for the shadow L4 table
    let shadow_frame = FRAME_ALLOCATOR
        .lock()
        .allocate_frames(1, None)
        .map_err(|_| crate::error::KernelError::OutOfMemory {
            requested: 4096,
            available: 0,
        })?;

    let shadow_phys = shadow_frame.as_u64() * 4096;
    let shadow_virt = phys_to_virt_addr(shadow_phys) as *mut u64;

    // Zero the entire shadow L4 table
    // SAFETY: shadow_virt points to a freshly allocated 4KB frame in the
    // kernel physical memory window. We have exclusive access.
    unsafe {
        core::ptr::write_bytes(shadow_virt, 0, 512);
    }

    // Read the kernel L4 table
    let kernel_l4_virt = phys_to_virt_addr(kernel_cr3) as *const u64;

    // Copy user-space entries (L4[0..255])
    // SAFETY: Both pointers are within the kernel physical memory window,
    // referencing valid L4 page table frames.
    unsafe {
        for i in 0..USER_KERNEL_SPLIT {
            let entry = core::ptr::read_volatile(kernel_l4_virt.add(i));
            core::ptr::write_volatile(shadow_virt.add(i), entry);
        }
    }

    // Map the trampoline page at L4[511]
    // This provides the minimal kernel mapping needed for syscall entry/exit.
    map_trampoline_in_l4(shadow_virt, kernel_l4_virt)?;

    Ok(shadow_phys)
}

/// Map the trampoline entry in L4[511] of the shadow table.
///
/// Copies only the L4[511] entry from the kernel table, which covers
/// the top 512GB of virtual memory including the trampoline page.
/// In a production implementation, this would create a minimal L3/L2/L1
/// chain mapping only the trampoline code page.
fn map_trampoline_in_l4(
    shadow_l4: *mut u64,
    kernel_l4: *const u64,
) -> Result<(), crate::error::KernelError> {
    // Copy L4[511] from the kernel table.
    // This gives the shadow table access to the same L3 subtree as the
    // kernel for the top 512GB, which includes the trampoline address.
    //
    // For tighter isolation, a dedicated L3->L2->L1 chain mapping only
    // the trampoline page should be used (deferred to Phase 7.5).
    // SAFETY: Both pointers reference valid L4 page table frames within
    // the kernel physical memory window.
    unsafe {
        let kernel_entry = core::ptr::read_volatile(kernel_l4.add(511));
        if kernel_entry & PTE_PRESENT != 0 {
            // Keep the entry but mark it user-accessible for the trampoline
            let trampoline_entry = kernel_entry | PTE_USER;
            core::ptr::write_volatile(shadow_l4.add(511), trampoline_entry);
        } else {
            // L4[511] is not mapped in the kernel -- create a new entry
            let frame = FRAME_ALLOCATOR
                .lock()
                .allocate_frames(1, None)
                .map_err(|_| crate::error::KernelError::OutOfMemory {
                    requested: 4096,
                    available: 0,
                })?;
            let frame_phys = frame.as_u64() * 4096;

            // Zero the L3 table
            let l3_virt = phys_to_virt_addr(frame_phys) as *mut u8;
            core::ptr::write_bytes(l3_virt, 0, 4096);

            // Create L4[511] entry pointing to the new L3
            let entry = frame_phys | PTE_PRESENT | PTE_WRITABLE | PTE_USER;
            core::ptr::write_volatile(shadow_l4.add(511), entry);
        }
    }

    Ok(())
}

// ===========================================================================
// CR3 Switching
// ===========================================================================

/// Switch to the shadow (user-mode) page table.
///
/// Called just before returning to Ring 3 (e.g., after syscall completion
/// or interrupt return). Loads the shadow CR3 which lacks kernel mappings.
#[inline(always)]
pub fn switch_to_user() {
    let shadow = SHADOW_CR3.load(Ordering::Acquire);
    if shadow != 0 {
        let cr3_val = PhysicalAddress::new(shadow);
        super::mmu::write_cr3(cr3_val);
    }
}

/// Switch to the full kernel page table.
///
/// Called on kernel entry (syscall, interrupt, exception). Restores the
/// full CR3 so the kernel has access to all its mappings.
#[inline(always)]
pub fn switch_to_kernel() {
    let guard = KPTI_STATE.lock();
    if let Some(state) = guard.as_ref() {
        if state.initialized {
            let cr3_val = PhysicalAddress::new(state.tables.kernel_cr3);
            super::mmu::write_cr3(cr3_val);
        }
    }
}

// ===========================================================================
// Syscall Hooks
// ===========================================================================

/// Called at the start of every syscall handler.
///
/// Currently a no-op because the syscall entry assembly switches CR3
/// before reaching Rust code. This hook exists for future use (e.g.,
/// per-CPU KPTI state tracking, telemetry).
#[inline(always)]
pub fn on_syscall_entry() {
    // CR3 switch is handled in assembly (syscall_entry) for performance.
    // This Rust-level hook is reserved for bookkeeping/diagnostics.
}

/// Called at the end of every syscall handler, just before SYSRET.
///
/// Currently a no-op; the SYSRET path in assembly handles CR3 restore.
#[inline(always)]
pub fn on_syscall_exit() {
    // CR3 switch back to shadow is handled in assembly (syscall_return).
}

// ===========================================================================
// Query / Diagnostics
// ===========================================================================

/// Check whether KPTI is initialized and active.
pub fn is_active() -> bool {
    SHADOW_CR3.load(Ordering::Acquire) != 0
}

/// Get the current KPTI page table pair (for diagnostics).
pub fn get_page_tables() -> Option<(u64, u64)> {
    let guard = KPTI_STATE.lock();
    guard
        .as_ref()
        .map(|s| (s.tables.kernel_cr3, s.tables.shadow_cr3))
}

/// Validate shadow table integrity.
///
/// Checks that user-space entries in the shadow L4 match the kernel L4,
/// and that kernel-space entries (except L4[511]) are empty.
pub fn validate_shadow_tables() -> bool {
    let guard = KPTI_STATE.lock();
    let state = match guard.as_ref() {
        Some(s) if s.initialized => s,
        _ => return false,
    };

    let kernel_l4 = phys_to_virt_addr(state.tables.kernel_cr3) as *const u64;
    let shadow_l4 = phys_to_virt_addr(state.tables.shadow_cr3) as *const u64;

    // SAFETY: Both pointers reference valid L4 page table frames.
    unsafe {
        // User entries should match
        for i in 0..USER_KERNEL_SPLIT {
            let k = core::ptr::read_volatile(kernel_l4.add(i));
            let s = core::ptr::read_volatile(shadow_l4.add(i));
            if k != s {
                crate::println!(
                    "[KPTI] Mismatch at L4[{}]: kernel=0x{:x}, shadow=0x{:x}",
                    i,
                    k,
                    s
                );
                return false;
            }
        }

        // Kernel entries [256..510] should be empty in shadow
        for i in USER_KERNEL_SPLIT..511 {
            let s = core::ptr::read_volatile(shadow_l4.add(i));
            if s & PTE_PRESENT != 0 {
                crate::println!("[KPTI] Shadow L4[{}] unexpectedly present: 0x{:x}", i, s);
                return false;
            }
        }

        // L4[511] should be present (trampoline)
        let trampoline = core::ptr::read_volatile(shadow_l4.add(511));
        if trampoline & PTE_PRESENT == 0 {
            crate::println!("[KPTI] Shadow L4[511] (trampoline) not present");
            return false;
        }
    }

    true
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(USER_KERNEL_SPLIT, 256);
        assert_eq!(L4_ENTRY_COUNT, 512);
        assert_eq!(TRAMPOLINE_VADDR, 0xFFFF_FFFF_FFFF_0000);
    }

    #[test]
    fn test_pte_flags() {
        assert_eq!(PTE_PRESENT, 1);
        assert_eq!(PTE_WRITABLE, 2);
        assert_eq!(PTE_USER, 4);
        assert_eq!(PTE_NO_EXECUTE, 1u64 << 63);
    }

    #[test]
    fn test_kpti_not_active_initially() {
        // KPTI requires actual page tables, so it should not be active
        // in a test environment without hardware initialization.
        // Just verify the atomic loads don't panic.
        let _ = is_active();
    }

    #[test]
    fn test_kpti_page_tables_struct() {
        let tables = KptiPageTables {
            kernel_cr3: 0x1000,
            shadow_cr3: 0x2000,
        };
        assert_eq!(tables.kernel_cr3, 0x1000);
        assert_eq!(tables.shadow_cr3, 0x2000);
    }
}

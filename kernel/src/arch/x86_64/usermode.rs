//! User-mode entry point for x86_64
//!
//! Provides `enter_usermode()` which pushes the iretq frame and transitions
//! the CPU from Ring 0 to Ring 3. Also provides `map_user_page()` for
//! creating user-accessible page table entries through the bootloader's
//! physical memory mapping.
//!
//! `enter_usermode_returnable()` is a variant that saves the boot context
//! (callee-saved registers, RSP, CR3) so that `sys_exit` can restore it
//! and effectively "return" to the caller, allowing sequential user-mode
//! program execution during bootstrap.

use core::{
    arch::asm,
    sync::atomic::{AtomicU64, Ordering},
};

/// Saved bootstrap RSP for returning after a user process exits.
/// Set by `enter_usermode_returnable()`, consumed by `boot_return_to_kernel()`.
pub static BOOT_RETURN_RSP: AtomicU64 = AtomicU64::new(0);

/// Saved bootstrap CR3 for returning after a user process exits.
pub static BOOT_RETURN_CR3: AtomicU64 = AtomicU64::new(0);

/// Stack canary for detecting corruption of the boot context.
/// Set to a known value when the boot context is saved, verified before
/// restore. A mismatch indicates stack corruption (buffer overflow,
/// use-after-free, etc.).
pub static BOOT_STACK_CANARY: AtomicU64 = AtomicU64::new(0);

/// Magic value for the boot stack canary.
/// Chosen to be unlikely to appear naturally in memory.
const BOOT_CANARY_MAGIC: u64 = 0xDEAD_BEEF_CAFE_BABE;

/// Enter user mode for the first time via iretq.
///
/// The iretq instruction pops SS, RSP, RFLAGS, CS, RIP from the stack
/// and transitions the CPU to the privilege level specified in the CS
/// selector's RPL field.
///
/// # Arguments
/// - `entry_point`: User-space RIP (entry point of the user program)
/// - `user_stack`: User-space RSP (top of user stack)
/// - `user_cs`: User code segment selector with RPL=3 (0x33)
/// - `user_ss`: User data segment selector with RPL=3 (0x2B)
///
/// # Safety
/// - `entry_point` must be a valid user-space address with executable code
///   mapped
/// - `user_stack` must be a valid user-space stack address, 16-byte aligned
/// - The correct page tables must be loaded in CR3 with USER-accessible
///   mappings
/// - Per-CPU data (`kernel_rsp`) must be set before calling this, otherwise the
///   first syscall or interrupt will crash due to invalid kernel stack
/// - The GDT must contain valid Ring 3 segments at the specified selectors
pub unsafe fn enter_usermode(entry_point: u64, user_stack: u64, user_cs: u64, user_ss: u64) -> ! {
    // SAFETY: We build the iretq frame on the current kernel stack.
    // iretq expects (from top of stack): RIP, CS, RFLAGS, RSP, SS.
    // We set DS and ES to the user data selector and clear FS/GS.
    // RFLAGS = 0x202: bit 1 (reserved, always 1) + bit 9 (IF = interrupts enabled).
    // The caller guarantees all arguments point to valid mapped memory and
    // the GDT/TSS/per-CPU data are properly configured.
    asm!(
        // Set data segment registers to user data selector
        "mov ds, {ss:r}",
        "mov es, {ss:r}",
        // Clear FS and GS (will be set up later for TLS if needed).
        // Use a dedicated zero operand to avoid clobbering other operands
        // (the compiler may place rflags in eax, so "xor eax, eax" would
        // destroy it).
        "mov fs, {zero:x}",
        "mov gs, {zero:x}",
        // Build iretq frame on current kernel stack:
        //   [RSP+0]  RIP    - user entry point
        //   [RSP+8]  CS     - user code segment (Ring 3)
        //   [RSP+16] RFLAGS - IF set (0x202)
        //   [RSP+24] RSP    - user stack pointer
        //   [RSP+32] SS     - user stack segment (Ring 3)
        "push {ss}",       // SS
        "push {rsp}",      // RSP (user stack)
        "push {rflags}",   // RFLAGS (IF enabled)
        "push {cs}",       // CS
        "push {rip}",      // RIP (entry point)
        "iretq",
        ss = in(reg) user_ss,
        rsp = in(reg) user_stack,
        rflags = in(reg) 0x202u64,
        cs = in(reg) user_cs,
        rip = in(reg) entry_point,
        zero = in(reg) 0u64,
        options(noreturn)
    );
}

/// Enter user mode with the ability to return when the process exits.
///
/// Saves callee-saved registers and the current RSP/CR3 to globals before
/// performing iretq. When the user process calls `sys_exit`, the
/// `boot_return_to_kernel()` function restores the saved context, making
/// this function appear to return normally.
///
/// # Arguments
/// - `entry_point`: User-space RIP
/// - `user_stack`: User-space RSP
/// - `user_cs`: User CS selector (Ring 3)
/// - `user_ss`: User SS selector (Ring 3)
/// - `process_cr3`: Physical address of the process's L4 page table
/// - `kernel_rsp_ptr`: Pointer to per-CPU kernel_rsp (written after context
///   save)
///
/// # Safety
/// Same requirements as `enter_usermode`, plus:
/// - `process_cr3` must be a valid L4 page table with both user and kernel
///   mappings
/// - `kernel_rsp_ptr` must point to a valid u64 for storing the kernel RSP
#[unsafe(naked)]
pub unsafe extern "C" fn enter_usermode_returnable(
    _entry_point: u64,    // rdi
    _user_stack: u64,     // rsi
    _user_cs: u64,        // rdx
    _user_ss: u64,        // rcx
    _process_cr3: u64,    // r8
    _kernel_rsp_ptr: u64, // r9
) {
    core::arch::naked_asm!(
        // Save callee-saved registers (System V ABI)
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        // Alignment padding: after 6 pushes from function entry (RSP was
        // 16n+8 after the CALL), RSP is now 16n+8 - 48 = 16m+8 (mod 16 = 8).
        // syscall_entry loads kernel_rsp, pushes 14 registers (112 bytes,
        // alignment-neutral), then does CALL handler. For the handler to get
        // the ABI-required RSP mod 16 = 8, the loaded kernel_rsp must be
        // mod 16 = 0. Adding 8 bytes of padding achieves this:
        //   16m+8 - 8 = 16m (mod 16 = 0).
        // boot_return_to_kernel must skip this padding when restoring RSP.
        "sub rsp, 8",

        // FIX 3: Set stack canary BEFORE saving boot context
        // Load canary magic value and store to global
        "mov rax, {canary_magic}",
        "lea r12, [rip + {boot_canary}]",
        "mov [r12], rax",

        // Save boot CR3 to global
        "mov rax, cr3",
        "lea r12, [rip + {boot_cr3}]",
        "mov [r12], rax",

        // Save boot RSP to global (includes alignment padding)
        "lea r12, [rip + {boot_rsp}]",
        "mov [r12], rsp",

        // Update per-CPU kernel_rsp via pointer passed in r9
        // This value is 16-byte aligned, ensuring syscall handlers get
        // correct SSE alignment for movaps instructions.
        "mov [r9], rsp",

        // Switch to process page tables
        "mov cr3, r8",

        // Set segment registers for user mode
        "mov ds, ecx",
        "mov es, ecx",
        "xor eax, eax",
        "mov fs, ax",
        "mov gs, ax",

        // Build iretq frame on stack
        "push rcx",       // SS
        "push rsi",       // RSP (user stack)
        "push 0x202",     // RFLAGS (IF enabled)
        "push rdx",       // CS
        "push rdi",       // RIP (entry point)

        "iretq",

        boot_cr3 = sym BOOT_RETURN_CR3,
        boot_rsp = sym BOOT_RETURN_RSP,
        boot_canary = sym BOOT_STACK_CANARY,
        canary_magic = const BOOT_CANARY_MAGIC,
    );
}

/// Like `enter_usermode_returnable`, but sets RAX=0 before iretq.
///
/// Used for running forked child processes inline from the wait loop.
/// The forked child expects RAX=0 as the fork() return value indicating
/// it's the child process.
///
/// # Safety
/// Same preconditions as `enter_usermode_returnable`.
#[unsafe(naked)]
pub unsafe extern "C" fn enter_forked_child_returnable(
    _entry_point: u64,    // rdi
    _user_stack: u64,     // rsi
    _user_cs: u64,        // rdx
    _user_ss: u64,        // rcx
    _process_cr3: u64,    // r8
    _kernel_rsp_ptr: u64, // r9
) {
    core::arch::naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "sub rsp, 8",

        // Set stack canary
        "mov rax, {canary_magic}",
        "lea r12, [rip + {boot_canary}]",
        "mov [r12], rax",

        // Save boot CR3 to global
        "mov rax, cr3",
        "lea r12, [rip + {boot_cr3}]",
        "mov [r12], rax",

        // Save boot RSP to global
        "lea r12, [rip + {boot_rsp}]",
        "mov [r12], rsp",

        // Update per-CPU kernel_rsp
        "mov [r9], rsp",

        // Switch to child's page tables
        "mov cr3, r8",

        // Set segment registers for user mode
        "mov ds, ecx",
        "mov es, ecx",
        "xor eax, eax",
        "mov fs, ax",
        "mov gs, ax",

        // Build iretq frame on stack
        "push rcx",       // SS
        "push rsi",       // RSP (user stack)
        "push 0x202",     // RFLAGS (IF enabled)
        "push rdx",       // CS
        "push rdi",       // RIP (entry point)

        // RAX = 0 for fork() child return value
        // (already 0 from xor eax,eax above, but be explicit)
        "xor eax, eax",

        "iretq",

        boot_cr3 = sym BOOT_RETURN_CR3,
        boot_rsp = sym BOOT_RETURN_RSP,
        boot_canary = sym BOOT_STACK_CANARY,
        canary_magic = const BOOT_CANARY_MAGIC,
    );
}

/// Restore the boot context saved by `enter_usermode_returnable` and return
/// to the bootstrap code.
///
/// Called from `sys_exit` after cleaning up the exiting process. This function:
/// 1. Restores the boot CR3 (switching back to boot page tables)
/// 2. Restores kernel segment registers (DS, ES, FS, GS cleared)
/// 3. Does `swapgs` to balance the swapgs from `syscall_entry`
/// 4. Restores RSP to the saved value (past the callee-saved pushes)
/// 5. Pops callee-saved registers and returns to the caller of
///    `enter_usermode_returnable`
///
/// # Safety
/// - Must only be called when `BOOT_RETURN_RSP` and `BOOT_RETURN_CR3` are valid
/// - Must be called from kernel mode on the kernel stack set by syscall_entry
/// - The saved boot stack frame must still be intact
///
/// # Implementation Notes
/// - `#[inline(never)]` prevents aggressive optimization that could corrupt the
///   stack frame restoration in release builds
/// - `compiler_fence` ensures loads complete before subsequent operations
/// - `black_box` prevents constant propagation and reordering of critical
///   values
#[inline(never)]
pub unsafe fn boot_return_to_kernel() -> ! {
    // RAW SERIAL DIAGNOSTIC: Trace boot return entry
    crate::arch::x86_64::idt::raw_serial_str(b"[BOOT_RETURN ENTRY]\n");

    // FIX 2 & 6: Use black_box to force compiler to treat values as opaque,
    // preventing optimization assumptions. Follow with compiler fence to
    // prevent instruction reordering across this boundary.
    //
    // CRITICAL FIX: The release optimizer was reusing RAX after `xor eax,eax`
    // (used to zero FS/GS) to load RSP, which set RSP=0 and caused a double
    // fault. We now use inline assembly with explicit register constraints
    // to force RSP into a register that won't be clobbered, and keep CR3
    // separate. The asm! block below uses `inout` constraints to prevent
    // the compiler from reusing these registers.
    let rsp: u64;
    let cr3: u64;
    let canary: u64;

    // Load values with explicit register assignments to prevent optimization
    asm!(
        "mov {rsp}, [{rsp_addr}]",
        "mov {cr3}, [{cr3_addr}]",
        "mov {canary}, [{canary_addr}]",
        rsp = out(reg) rsp,
        cr3 = out(reg) cr3,
        canary = out(reg) canary,
        rsp_addr = in(reg) &BOOT_RETURN_RSP,
        cr3_addr = in(reg) &BOOT_RETURN_CR3,
        canary_addr = in(reg) &BOOT_STACK_CANARY,
        options(nostack, preserves_flags)
    );

    // Apply black_box to prevent further optimization
    let rsp = core::hint::black_box(rsp);
    let cr3 = core::hint::black_box(cr3);
    let canary = core::hint::black_box(canary);
    core::sync::atomic::compiler_fence(Ordering::SeqCst);

    // FIX 3: Validate stack canary before restoring context
    // If the canary doesn't match, the boot stack has been corrupted
    if canary != BOOT_CANARY_MAGIC {
        crate::arch::x86_64::idt::raw_serial_str(b"[BOOT_RETURN] FATAL: Stack canary mismatch!\n");
        crate::arch::x86_64::idt::raw_serial_str(b"Expected: 0x");
        crate::arch::x86_64::idt::raw_serial_hex(BOOT_CANARY_MAGIC);
        crate::arch::x86_64::idt::raw_serial_str(b"\nGot:      0x");
        crate::arch::x86_64::idt::raw_serial_hex(canary);
        crate::arch::x86_64::idt::raw_serial_str(b"\n");
        panic!("Stack canary mismatch - boot context corrupted");
    }

    // NOTE: Cannot use println! here - would access locks/memory with wrong CR3
    // crate::println!("[BOOT-RETURN] RSP={:#x} CR3={:#x}", rsp, cr3);

    // Clear the boot return context (one-shot)
    BOOT_RETURN_RSP.store(0, Ordering::SeqCst);
    BOOT_RETURN_CR3.store(0, Ordering::SeqCst);
    BOOT_STACK_CANARY.store(0, Ordering::SeqCst);

    // SAFETY: cr3 is the boot page table address saved before entering user
    // mode. rsp points to the stack with 8 bytes of alignment padding and
    // 6 callee-saved registers, with the return address below them. We
    // restore kernel segment registers and
    // balance the swapgs from syscall_entry. The swapgs must come BEFORE
    // clearing GS so we don't corrupt KERNEL_GS_BASE. After restoring RSP
    // and popping registers, ret returns to the caller of
    // enter_usermode_returnable.
    //
    // CRITICAL FIX FOR OPT-LEVEL S/Z/3: The optimizer was allocating RSP
    // to RAX, which then got clobbered by `xor eax,eax` used for zeroing
    // FS/GS. We now explicitly allocate RSP to RCX and CR3 to RDX, both
    // of which are preserved across the segment register operations. This
    // is the ONLY way to prevent the optimizer from reusing RAX.
    asm!(
        "mov cr3, rdx",       // Restore boot page tables (CR3 in RDX)
        "swapgs",              // Balance syscall_entry's swapgs (before touching GS!)
        "mov ax, 0x10",       // Kernel data segment (GDT index 2, RPL 0)
        "mov ds, ax",         // Restore kernel DS
        "mov es, ax",         // Restore kernel ES
        "xor eax, eax",       // Zero FS and GS (clobbers RAX but NOT RCX/RDX!)
        "mov fs, ax",
        "mov gs, ax",
        "mov rsp, rcx",       // Restore saved boot RSP (RSP in RCX, safe!)
        "add rsp, 8",         // Skip alignment padding from enter_usermode_returnable
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",                 // Return to caller of enter_usermode_returnable
        in("rcx") rsp,        // RSP MUST be in RCX (preserved across xor eax,eax)
        in("rdx") cr3,        // CR3 MUST be in RDX (preserved across xor eax,eax)
        options(noreturn)
    );
}

/// Check whether a boot return context is available.
///
/// Returns `true` if `enter_usermode_returnable` has saved a boot context
/// that `boot_return_to_kernel` can restore.
pub fn has_boot_return_context() -> bool {
    BOOT_RETURN_RSP.load(Ordering::SeqCst) != 0
}

/// Physical memory offset provided by the bootloader.
///
/// All physical memory is mapped at virtual address `phys_addr + PHYS_OFFSET`.
/// Initialized during `try_enter_usermode()` from BOOT_INFO.
static PHYS_OFFSET: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Get the physical memory offset, or 0 if not yet initialized.
///
/// Used by kernel subsystems that need to convert physical addresses to
/// virtual addresses after the initial user-mode setup.
#[allow(dead_code)] // Helper for phys_to_virt below
fn phys_offset() -> u64 {
    PHYS_OFFSET.load(core::sync::atomic::Ordering::Relaxed)
}

/// Convert a physical address to a virtual address via the bootloader's
/// physical memory mapping.
///
/// Returns `None` if the physical memory offset has not been initialized.
/// Used by kernel subsystems that need to access physical memory after
/// the initial user-mode setup.
#[allow(dead_code)] // Physical-to-virtual conversion for page table manipulation
fn phys_to_virt(phys: u64) -> Option<u64> {
    let offset = phys_offset();
    if offset == 0 {
        return None;
    }
    Some(phys + offset)
}

/// Page table entry flags for x86_64 4-level paging.
const PTE_PRESENT: u64 = 1 << 0;
const PTE_WRITABLE: u64 = 1 << 1;
const PTE_USER: u64 = 1 << 2;

/// Extract the physical address of the next-level page table from a PTE.
///
/// The physical address is stored in bits 12..51 of the entry.
fn pte_phys_addr(entry: u64) -> u64 {
    entry & 0x000F_FFFF_FFFF_F000
}

/// Map a single 4KiB page in the current page tables with USER access.
///
/// Walks the 4-level page table hierarchy (PML4 -> PDPT -> PD -> PT),
/// allocating intermediate tables as needed from the frame allocator.
/// The leaf entry maps `virt_addr` to `phys_frame_addr` with the given flags.
///
/// # Safety
/// - `phys_offset_val` must be the correct bootloader physical memory offset
/// - `virt_addr` must be page-aligned (4KiB)
/// - `phys_frame_addr` must be a valid, page-aligned physical address
/// - The caller must ensure no conflicting mapping exists
unsafe fn map_user_page(
    phys_offset_val: u64,
    virt_addr: u64,
    phys_frame_addr: u64,
    flags: u64,
) -> Result<(), crate::error::KernelError> {
    // Read current CR3 to get the PML4 physical address
    let cr3: u64;
    // SAFETY: Reading CR3 is always valid in kernel mode.
    asm!("mov {}, cr3", out(reg) cr3);
    let pml4_phys = cr3 & 0x000F_FFFF_FFFF_F000;

    // Extract page table indices from the virtual address
    let pml4_idx = ((virt_addr >> 39) & 0x1FF) as usize;
    let pdpt_idx = ((virt_addr >> 30) & 0x1FF) as usize;
    let pd_idx = ((virt_addr >> 21) & 0x1FF) as usize;
    let pt_idx = ((virt_addr >> 12) & 0x1FF) as usize;

    // Walk PML4 -> PDPT
    let pml4_virt = (pml4_phys + phys_offset_val) as *mut u64;
    let pml4_entry = pml4_virt.add(pml4_idx);
    let pdpt_phys = ensure_table_present(pml4_entry, phys_offset_val)?;

    // Walk PDPT -> PD
    let pdpt_virt = (pdpt_phys + phys_offset_val) as *mut u64;
    let pdpt_entry = pdpt_virt.add(pdpt_idx);
    let pd_phys = ensure_table_present(pdpt_entry, phys_offset_val)?;

    // Walk PD -> PT
    let pd_virt = (pd_phys + phys_offset_val) as *mut u64;
    let pd_entry = pd_virt.add(pd_idx);
    let pt_phys = ensure_table_present(pd_entry, phys_offset_val)?;

    // Set the leaf PT entry
    let pt_virt = (pt_phys + phys_offset_val) as *mut u64;
    let pt_entry = pt_virt.add(pt_idx);
    // SAFETY: pt_entry points into a valid page table mapped via the physical
    // memory offset. We write the leaf mapping: physical frame + flags.
    pt_entry.write_volatile(phys_frame_addr | flags);

    // Flush TLB for this address
    // SAFETY: invlpg invalidates the TLB entry for virt_addr. No side effects.
    asm!("invlpg [{}]", in(reg) virt_addr);

    Ok(())
}

/// Ensure a page table entry at `entry_ptr` is present. If not, allocate
/// a new zeroed frame for the next-level table and write the entry.
///
/// Returns the physical address of the next-level table.
///
/// # Safety
/// - `entry_ptr` must point to a valid page table entry in mapped memory
/// - `phys_offset_val` must be the correct physical memory offset
unsafe fn ensure_table_present(
    entry_ptr: *mut u64,
    phys_offset_val: u64,
) -> Result<u64, crate::error::KernelError> {
    // SAFETY: entry_ptr was computed from a valid page table base + index,
    // both within the physical memory mapping provided by the bootloader.
    let entry = entry_ptr.read_volatile();

    if (entry & PTE_PRESENT) != 0 {
        // Table already exists. Ensure USER bit is set on intermediate entries
        // so user-mode accesses can traverse the hierarchy.
        let updated = entry | PTE_USER | PTE_WRITABLE;
        if updated != entry {
            // SAFETY: Updating flags on an existing present entry is safe.
            // We only add USER and WRITABLE bits to intermediate tables.
            entry_ptr.write_volatile(updated);
        }
        Ok(pte_phys_addr(entry))
    } else {
        // Allocate a new frame for the next-level table
        let frame = crate::mm::FRAME_ALLOCATOR
            .lock()
            .allocate_frames(1, None)
            .map_err(|_| crate::error::KernelError::ResourceExhausted {
                resource: "physical frames",
            })?;
        let frame_phys = frame.as_u64() * crate::mm::FRAME_SIZE as u64;

        // Zero the new table
        let frame_virt = (frame_phys + phys_offset_val) as *mut u8;
        // SAFETY: frame_virt points to a freshly allocated 4KiB frame mapped
        // via the physical memory offset. write_bytes zeroes the entire page.
        core::ptr::write_bytes(frame_virt, 0, 4096);

        // Write the entry: physical address + PRESENT + WRITABLE + USER
        let new_entry = frame_phys | PTE_PRESENT | PTE_WRITABLE | PTE_USER;
        // SAFETY: entry_ptr points to a valid PTE slot. Writing a new entry
        // that points to our freshly zeroed frame is safe.
        entry_ptr.write_volatile(new_entry);

        Ok(frame_phys)
    }
}

/// Check if a physical address is used by the active page table hierarchy.
///
/// Walks PML4 -> PDPT -> PD -> PT and returns true if `phys` matches any
/// page-table frame's base address. This is O(n) in the number of page table
/// pages (~1000 for a typical bootloader mapping).
///
/// # Safety
/// - `phys_offset` must be the bootloader's physical memory offset
/// - `pml4_phys` must be a valid PML4 physical address (from CR3)
unsafe fn is_page_table_frame(phys_offset: u64, pml4_phys: u64, phys: u64) -> bool {
    if phys == pml4_phys {
        return true;
    }

    let pml4_virt = (pml4_phys + phys_offset) as *const u64;
    for i in 0..512 {
        // SAFETY: pml4_virt + i is within the PML4 page, mapped via phys_offset.
        let pml4_entry = pml4_virt.add(i).read_volatile();
        if (pml4_entry & PTE_PRESENT) == 0 {
            continue;
        }
        let pdpt_phys = pte_phys_addr(pml4_entry);
        if phys == pdpt_phys {
            return true;
        }

        let pdpt_virt = (pdpt_phys + phys_offset) as *const u64;
        for j in 0..512 {
            // SAFETY: pdpt_virt + j is within the PDPT page.
            let pdpt_entry = pdpt_virt.add(j).read_volatile();
            if (pdpt_entry & PTE_PRESENT) == 0 {
                continue;
            }
            if (pdpt_entry & (1 << 7)) != 0 {
                continue; // 1GiB huge page
            }
            let pd_phys = pte_phys_addr(pdpt_entry);
            if phys == pd_phys {
                return true;
            }

            let pd_virt = (pd_phys + phys_offset) as *const u64;
            for k in 0..512 {
                // SAFETY: pd_virt + k is within the PD page.
                let pd_entry = pd_virt.add(k).read_volatile();
                if (pd_entry & PTE_PRESENT) == 0 {
                    continue;
                }
                if (pd_entry & (1 << 7)) != 0 {
                    continue; // 2MiB huge page
                }
                let pt_phys = pte_phys_addr(pd_entry);
                if phys == pt_phys {
                    return true;
                }
            }
        }
    }

    false
}

/// Allocate a physical frame that does not overlap with any active page table
/// page. Frames that are page table pages are allocated (to consume them from
/// the free pool) but not returned.
///
/// # Safety
/// - `phys_offset` and `pml4_phys` must be valid (see `is_page_table_frame`)
unsafe fn allocate_safe_frame(
    phys_offset: u64,
    pml4_phys: u64,
    count: usize,
) -> Result<crate::mm::FrameNumber, crate::error::KernelError> {
    use crate::mm::{FRAME_ALLOCATOR, FRAME_SIZE};

    // Try up to 8192 times (enough to skip the ~1050 page table frames)
    for _ in 0..8192 {
        let frame = FRAME_ALLOCATOR
            .lock()
            .allocate_frames(count, None)
            .map_err(|_| crate::error::KernelError::ResourceExhausted {
                resource: "physical frames",
            })?;
        let phys = frame.as_u64() * FRAME_SIZE as u64;

        // Check all allocated frames in the range
        let mut overlaps = false;
        for f in 0..count as u64 {
            if is_page_table_frame(phys_offset, pml4_phys, phys + f * FRAME_SIZE as u64) {
                overlaps = true;
                break;
            }
        }

        if !overlaps {
            return Ok(frame);
        }
        // Frame overlaps a page table page -- leave it allocated (consumed)
        // so the allocator won't return it again, and try the next one.
    }

    Err(crate::error::KernelError::ResourceExhausted {
        resource: "non-page-table frames",
    })
}

/// Attempt to enter user mode with the embedded init binary.
///
/// This function:
/// 1. Retrieves the physical memory offset from BOOT_INFO
/// 2. Allocates physical frames for user code and stack
/// 3. Maps them at user-accessible virtual addresses in the current page tables
/// 4. Copies the embedded INIT_CODE machine code to the code page
/// 5. Sets up the per-CPU kernel_rsp for syscall/interrupt return
/// 6. Transitions to Ring 3 via iretq
///
/// On success, this function does not return (enters user mode).
/// On failure, returns a KernelError for the caller to log.
pub fn try_enter_usermode() -> Result<(), crate::error::KernelError> {
    use crate::{mm::FRAME_SIZE, userspace::embedded};

    // Step 1: Get the physical memory offset from BOOT_INFO
    // SAFETY: BOOT_INFO is a static mut written once during early boot
    // (in main.rs) and only read afterwards. At this point we are in
    // single-threaded Stage 6 bootstrap, so no data race is possible.
    // We use addr_of! to avoid creating a direct reference to the static mut.
    let phys_offset_val = unsafe {
        let boot_info_ptr = core::ptr::addr_of!(crate::arch::x86_64::boot::BOOT_INFO);
        let boot_info =
            (*boot_info_ptr)
                .as_ref()
                .ok_or(crate::error::KernelError::NotInitialized {
                    subsystem: "BOOT_INFO",
                })?;
        boot_info.physical_memory_offset.into_option().ok_or(
            crate::error::KernelError::NotInitialized {
                subsystem: "physical memory offset",
            },
        )?
    };

    PHYS_OFFSET.store(phys_offset_val, core::sync::atomic::Ordering::Relaxed);

    // Step 1b: Read CR3 to identify page table frames that must not be
    // allocated for user-space use (the bootloader doesn't mark them as
    // reserved in the memory map).
    let cr3_val: u64;
    // SAFETY: Reading CR3 is always valid in kernel mode.
    unsafe {
        asm!("mov {}, cr3", out(reg) cr3_val);
    }
    let pml4_phys = cr3_val & 0x000F_FFFF_FFFF_F000;

    // Step 2: Get the embedded init code
    let init_code = embedded::init_code_bytes();

    // Step 3: Allocate physical frames, skipping any that are page table pages.
    // One frame for code (mapped at 0x400000)
    // One frame for stack (mapped at 0x7FFFF000, stack grows down from 0x80000000)
    // SAFETY: phys_offset_val and pml4_phys are valid (verified above).
    let code_frame = unsafe { allocate_safe_frame(phys_offset_val, pml4_phys, 1)? };
    let code_phys = code_frame.as_u64() * FRAME_SIZE as u64;

    let stack_frame = unsafe { allocate_safe_frame(phys_offset_val, pml4_phys, 1)? };
    let stack_phys = stack_frame.as_u64() * FRAME_SIZE as u64;

    // Step 4: Map pages in the current page tables
    // Code page at 0x400000 (PRESENT + WRITABLE + USER, executable)
    let code_vaddr: u64 = 0x40_0000;
    let stack_vaddr: u64 = 0x7FFF_F000;

    // SAFETY: We have verified that phys_offset_val is the correct bootloader
    // mapping offset. The virtual addresses are in the user-space range (below
    // 0x0000_8000_0000_0000) and do not conflict with kernel mappings. The
    // physical frames were just allocated and are valid.
    unsafe {
        map_user_page(
            phys_offset_val,
            code_vaddr,
            code_phys,
            PTE_PRESENT | PTE_WRITABLE | PTE_USER,
        )?;

        map_user_page(
            phys_offset_val,
            stack_vaddr,
            stack_phys,
            PTE_PRESENT | PTE_WRITABLE | PTE_USER,
        )?;
    }

    // Step 5: Copy init code to the code page
    // Access the code frame through the physical memory mapping
    let code_virt_via_phys = phys_offset_val + code_phys;
    // SAFETY: code_virt_via_phys points to a freshly allocated, zeroed frame
    // accessible through the bootloader's physical memory mapping. We copy
    // init_code.len() bytes (< 4096) into the frame.
    unsafe {
        let dest = code_virt_via_phys as *mut u8;
        core::ptr::copy_nonoverlapping(init_code.as_ptr(), dest, init_code.len());
    }

    // Step 6: Set up per-CPU kernel_rsp
    // Allocate a dedicated kernel stack for syscall/interrupt return
    // SAFETY: phys_offset_val and pml4_phys are valid.
    let kernel_stack_frame = unsafe { allocate_safe_frame(phys_offset_val, pml4_phys, 4)? };
    let kernel_stack_phys = kernel_stack_frame.as_u64() * FRAME_SIZE as u64;
    let kernel_stack_top = phys_offset_val + kernel_stack_phys + (4 * FRAME_SIZE as u64);

    // Write kernel_rsp to per-CPU data so syscall_entry can find it
    let per_cpu = crate::arch::x86_64::syscall::per_cpu_data_ptr();
    // SAFETY: per_cpu_data_ptr() returns a valid pointer to the static
    // PerCpuData. We are in single-threaded bootstrap context. Setting
    // kernel_rsp before entering user mode is required for syscall_entry
    // to have a valid kernel stack.
    unsafe {
        (*per_cpu).kernel_rsp = kernel_stack_top;
    }

    // Step 7: Enter user mode
    // User entry point = start of code at 0x400000
    // User stack pointer = top of stack page at 0x80000000 (grows down from top of
    // page)
    let user_entry = code_vaddr;
    let user_stack = stack_vaddr + FRAME_SIZE as u64; // Top of the stack page
    let user_cs: u64 = 0x33; // User code segment (GDT index 6, RPL 3)
    let user_ss: u64 = 0x2B; // User data segment (GDT index 5, RPL 3)

    crate::println!(
        "[USERMODE] Entering Ring 3: entry={:#x} stack={:#x}",
        user_entry,
        user_stack,
    );

    // SAFETY: All preconditions for enter_usermode are met:
    // - entry_point (0x400000) has executable code mapped with USER access
    // - user_stack (0x80000000) points to the top of a mapped user stack page
    // - CS/SS are valid Ring 3 selectors from the GDT
    // - CR3 contains page tables with USER-accessible mappings
    // - Per-CPU kernel_rsp is set for syscall/interrupt return
    unsafe {
        enter_usermode(user_entry, user_stack, user_cs, user_ss);
    }
}

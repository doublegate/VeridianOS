//! User-mode entry point for x86_64
//!
//! Provides `enter_usermode()` which pushes the iretq frame and transitions
//! the CPU from Ring 0 to Ring 3 (used by exec), and the cached
//! physical-memory offset.
//!
//! Programs run as dispatcher tasks (ADR 0006); the boot "returnable"
//! entry that ran them nested inside the boot flow was removed in sprint D4.

use core::arch::asm;

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
    // If the process has TLS (fs_base != 0), set FS_BASE via wrmsr AFTER
    // clearing the FS selector. Writing 0 to FS zeros FS_BASE, so the
    // wrmsr must come after to re-establish TLS.
    let fs_base = crate::process::current_process()
        .map(|p| p.tls_fs_base.load(core::sync::atomic::Ordering::Acquire))
        .unwrap_or(0);

    let fs_lo = fs_base as u32;
    let fs_hi = (fs_base >> 32) as u32;

    // SAFETY: We build the iretq frame on the current kernel stack.
    // iretq expects (from top of stack): RIP, CS, RFLAGS, RSP, SS.
    // We set DS and ES to the user data selector and clear FS/GS.
    // RFLAGS = 0x202: bit 1 (reserved, always 1) + bit 9 (IF = interrupts enabled).
    // Forwarded from this function's contract: the caller guarantees the
    // entry point and stack are mapped USER-accessible in the loaded CR3,
    // the selectors are valid Ring 3 GDT segments, and per-CPU kernel_rsp is
    // set.
    unsafe {
        asm!(
            // Set data segment registers to user data selector
            "mov ds, {ss:r}",
            "mov es, {ss:r}",
            // No interrupt from here to the iretq: after the swapgs below it
            // would see a ring-0 CS with the user GS base.
            "cli",
            // Clear FS. Writing 0 to FS zeros FS_BASE (MSR 0xC0000100). GS
            // is left alone: loading a selector would clear the GS base,
            // which holds the per-CPU block until the swapgs.
            "mov fs, {zero:x}",
            // Build iretq frame on current kernel stack FIRST, before wrmsr.
            // wrmsr uses ECX (MSR number), EDX:EAX (value) as IMPLICIT operands.
            // All iretq frame values must be pushed BEFORE wrmsr, because wrmsr
            // clobbers EAX/ECX/EDX.
            //
            // CRITICAL: fs_lo and fs_hi are bound to EAX and EDX via explicit
            // register constraints (`in("eax")` / `in("edx")`). This prevents
            // the compiler from allocating them to ECX, which gets overwritten
            // by `mov ecx, 0xC0000100`. Without this, the release optimizer can
            // allocate fs_hi to ECX, causing `mov edx, ecx` to load 0xC0000100
            // instead of the actual fs_hi value, which makes wrmsr write a
            // non-canonical address to FS_BASE and triggers a GP fault.
            //
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
            // Now restore FS_BASE for TLS if non-zero. All operand values are safely
            // on the stack. fs_lo is already in EAX, fs_hi is already in EDX.
            // Only ECX needs to be loaded with the MSR number.
            "test edx, edx",
            "jnz 2f",
            "test eax, eax",
            "jz 3f",
            "2:",
            "mov ecx, 0xC0000100",
            "wrmsr",
            "3:",
            // Ring 0 keeps the per-CPU block in GS_BASE; ring 3 gets its own.
            "swapgs",
            "iretq",
            ss = in(reg) user_ss,
            rsp = in(reg) user_stack,
            rflags = in(reg) 0x202u64,
            cs = in(reg) user_cs,
            rip = in(reg) entry_point,
            zero = in(reg) 0u64,
            in("eax") fs_lo,
            in("edx") fs_hi,
            options(noreturn)
        )
    };
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
/// Initialize the cached physical memory offset.
///
/// Called once during bootstrap (before any user process runs) to cache
/// the bootloader's physical memory offset. After this, `phys_offset()`
/// and `msr::phys_to_virt()` can be called from any context without
/// accessing BOOT_INFO (which may not be mapped in user page tables).
pub(crate) fn init_phys_offset(offset: u64) {
    PHYS_OFFSET.store(offset, core::sync::atomic::Ordering::Relaxed);
}

/// Returns the cached physical memory offset, or 0 if not yet initialized.
///
/// This is safe to call from any context (interrupt, syscall, IST) because
/// it reads an atomic value in kernel BSS, unlike BOOT_INFO which may be
/// unmapped when running with a user process's CR3.
pub(crate) fn phys_offset() -> u64 {
    PHYS_OFFSET.load(core::sync::atomic::Ordering::Relaxed)
}

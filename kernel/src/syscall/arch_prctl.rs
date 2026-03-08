//! Architecture-specific thread-local storage (TLS) base register management.
//!
//! Implements the `arch_prctl(2)` syscall for getting and setting the TLS base
//! pointer used by user-space threads.  On x86_64 this controls the FS
//! segment base; on AArch64 it maps to TPIDR_EL0; on RISC-V it maps to the
//! `tp` (thread pointer) register.
//!
//! The underlying `ThreadContext` trait provides a uniform `set_tls_base` /
//! `tls_base` interface, so the implementation is architecture-independent.
//! Only the constants (`ARCH_SET_FS`, `ARCH_GET_FS`) follow the x86_64 ABI
//! numbering; other architectures reuse the same codes for compatibility with
//! the VeridianOS libc.

use crate::{arch::context::ThreadContext, process, syscall::SyscallError};

// x86_64 arch_prctl codes (subset).  Reused on all architectures for a
// uniform syscall ABI.
const ARCH_SET_FS: usize = 0x1002;
const ARCH_GET_FS: usize = 0x1003;

/// Set or get the current thread's TLS base register.
///
/// # Arguments
///
/// * `code` — Operation to perform:
///   - `ARCH_SET_FS` (0x1002): set the TLS base to `addr`.
///   - `ARCH_GET_FS` (0x1003): write the current TLS base to the user-space
///     `u64` pointed to by `addr`.
/// * `addr` — For `SET_FS`: the new TLS base value.  For `GET_FS`: a user-space
///   pointer to a `u64` that will receive the current base.
///
/// # Returns
///
/// `Ok(0)` on success, or `SyscallError::InvalidArgument` for an unknown
/// code, or `SyscallError::InvalidPointer` if the `GET_FS` destination
/// pointer is invalid.
///
/// # Platform Notes
///
/// - **x86_64**: Controls the FS segment base (used by `%fs:`-relative TLS
///   accesses).
/// - **AArch64**: Controls TPIDR_EL0 (the user-mode thread ID register).
/// - **RISC-V**: Controls the `tp` register (thread pointer, x4).
pub fn sys_arch_prctl(code: usize, addr: usize) -> Result<isize, SyscallError> {
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let mut ctx = thread.context.lock();

    match code {
        ARCH_SET_FS => {
            ctx.set_tls_base(addr as u64);

            // Also write the MSR immediately so the FS base is active when
            // we return to user mode. iretq does not restore FS_BASE, so
            // without this write, %fs:-relative TLS accesses would fault.
            #[cfg(target_arch = "x86_64")]
            {
                let lo = addr as u32;
                let hi = (addr >> 32) as u32;
                // SAFETY: Writing IA32_FS_BASE (0xC0000100) is a privileged
                // operation executed in ring 0. `addr` is the TLS base
                // provided by the user process via arch_prctl.
                unsafe {
                    core::arch::asm!(
                        "wrmsr",
                        in("ecx") 0xC000_0100u32,
                        in("eax") lo,
                        in("edx") hi,
                    );
                }
            }

            Ok(0)
        }
        ARCH_GET_FS => {
            // SAFETY: `copy_to_user` validates that `addr` points to a
            // mapped, writable, user-space region large enough for a u64.
            // The `tls_base()` value is a plain integer read from the
            // thread context and does not involve any unsafe memory access
            // itself.
            unsafe { crate::syscall::userspace::copy_to_user(addr, &ctx.tls_base()) }
                .map_err(|_| SyscallError::InvalidPointer)?;
            Ok(0)
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

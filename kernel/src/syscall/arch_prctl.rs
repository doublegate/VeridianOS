//! Architecture-specific TLS base setters/getters for user threads.

use crate::{arch::context::ThreadContext, process, syscall::SyscallError};

// x86_64 arch_prctl codes (subset)
const ARCH_SET_FS: usize = 0x1002;
const ARCH_GET_FS: usize = 0x1003;

#[cfg(target_arch = "x86_64")]
pub fn sys_arch_prctl(code: usize, addr: usize) -> Result<isize, SyscallError> {
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let mut ctx = thread.context.lock();

    match code {
        ARCH_SET_FS => {
            ctx.set_tls_base(addr as u64);
            Ok(0)
        }
        ARCH_GET_FS => {
            unsafe { crate::syscall::userspace::copy_to_user(addr, &ctx.tls_base()) }
                .map_err(|_| SyscallError::InvalidPointer)?;
            Ok(0)
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

#[cfg(target_arch = "aarch64")]
pub fn sys_arch_prctl(code: usize, addr: usize) -> Result<isize, SyscallError> {
    // Use TPIDR_EL0 as TLS base
    if code != ARCH_SET_FS && code != ARCH_GET_FS {
        return Err(SyscallError::InvalidArgument);
    }
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let mut ctx = thread.context.lock();
    match code {
        ARCH_SET_FS => {
            ctx.set_tls_base(addr as u64);
            Ok(0)
        }
        ARCH_GET_FS => {
            unsafe { crate::syscall::userspace::copy_to_user(addr, &ctx.tls_base()) }
                .map_err(|_| SyscallError::InvalidPointer)?;
            Ok(0)
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

#[cfg(target_arch = "riscv64")]
pub fn sys_arch_prctl(code: usize, addr: usize) -> Result<isize, SyscallError> {
    // Use tp as TLS base
    if code != ARCH_SET_FS && code != ARCH_GET_FS {
        return Err(SyscallError::InvalidArgument);
    }
    let thread = process::current_thread().ok_or(SyscallError::InvalidState)?;
    let mut ctx = thread.context.lock();
    match code {
        ARCH_SET_FS => {
            ctx.set_tls_base(addr as u64);
            Ok(0)
        }
        ARCH_GET_FS => {
            unsafe { crate::syscall::userspace::copy_to_user(addr, &ctx.tls_base()) }
                .map_err(|_| SyscallError::InvalidPointer)?;
            Ok(0)
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

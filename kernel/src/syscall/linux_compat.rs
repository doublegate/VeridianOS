//! Linux ABI support shared by the system call handlers.
//!
//! The kernel's system call numbers are Linux's (ADR 0009), so there is no
//! number translation any more. What remains here is the Linux side of the
//! calling convention: errno values for errors, and the few calls whose
//! handlers exist only for the Linux interface (ppoll, prctl).

use super::SyscallResult;

/// Translate a VeridianOS SyscallError to a Linux errno value.
///
/// VeridianOS uses its own error numbering (e.g., InvalidArgument = -2,
/// ResourceNotFound = -4, BadFileDescriptor = -21). Linux uses different
/// errno values (EINVAL = -22, ENOENT = -2, EBADF = -9). When returning
/// errors to musl/glibc binaries (which use Linux errno numbering via
/// `__syscall_ret`), we must translate.
pub(crate) fn to_linux_errno(err: super::SyscallError) -> isize {
    use super::SyscallError;

    // Linux errno constants (negated for return values)
    const LINUX_EPERM: isize = -1;
    const LINUX_ENOENT: isize = -2;
    const LINUX_ESRCH: isize = -3;
    const LINUX_ECHILD: isize = -10;
    const LINUX_EINTR: isize = -4;
    const LINUX_ENOMEM: isize = -12;
    const LINUX_EACCES: isize = -13;
    const LINUX_EFAULT: isize = -14;
    const LINUX_EEXIST: isize = -17;
    const LINUX_ENOTDIR: isize = -20;
    const LINUX_EISDIR: isize = -21;
    const LINUX_EINVAL: isize = -22;
    const LINUX_EBADF: isize = -9;
    const LINUX_ENOTTY: isize = -25;
    const LINUX_EPIPE: isize = -32;
    const LINUX_EAGAIN: isize = -11;
    const LINUX_ENOSYS: isize = -38;
    const LINUX_ENOTEMPTY: isize = -39;
    const LINUX_ELOOP: isize = -40;
    const LINUX_EXDEV: isize = -18;
    const LINUX_ENODATA: isize = -61;
    const LINUX_EIO: isize = -5;
    const LINUX_E2BIG: isize = -7;
    const LINUX_EMFILE: isize = -24;
    const LINUX_ENOTSOCK: isize = -88;
    const LINUX_ENOPROTOOPT: isize = -92;
    const LINUX_ENOSPC: isize = -28;
    const LINUX_EBUSY: isize = -16;
    const LINUX_EFBIG: isize = -27;
    const LINUX_EOPNOTSUPP: isize = -95;
    const LINUX_ENODEV: isize = -19;
    const LINUX_ENOEXEC: isize = -8;
    const LINUX_ETIMEDOUT: isize = -110;
    const LINUX_EAFNOSUPPORT: isize = -97;
    const LINUX_EPROTONOSUPPORT: isize = -93;
    const LINUX_ESPIPE: isize = -29;
    const LINUX_ERANGE: isize = -34;
    const LINUX_ENAMETOOLONG: isize = -36;
    const LINUX_EADDRINUSE: isize = -98;
    const LINUX_ECONNREFUSED: isize = -111;
    const LINUX_ENOTCONN: isize = -107;
    const LINUX_EISCONN: isize = -106;
    const LINUX_EINPROGRESS: isize = -115;
    const LINUX_ECONNRESET: isize = -104;
    const LINUX_ECANCELED: isize = -125;

    match err {
        SyscallError::InvalidSyscall => LINUX_ENOSYS,
        SyscallError::InvalidArgument => LINUX_EINVAL,
        SyscallError::PermissionDenied => LINUX_EACCES,
        SyscallError::ResourceNotFound => LINUX_ENOENT,
        SyscallError::OutOfMemory => LINUX_ENOMEM,
        SyscallError::WouldBlock => LINUX_EAGAIN,
        SyscallError::Interrupted => LINUX_EINTR,
        SyscallError::InvalidState => LINUX_EINVAL,
        SyscallError::InvalidPointer => LINUX_EFAULT,
        SyscallError::InvalidCapability => LINUX_EPERM,
        SyscallError::CapabilityRevoked => LINUX_EPERM,
        SyscallError::InsufficientRights => LINUX_EPERM,
        SyscallError::CapabilityNotFound => LINUX_ENOENT,
        SyscallError::CapabilityAlreadyExists => LINUX_EEXIST,
        SyscallError::InvalidCapabilityObject => LINUX_EINVAL,
        SyscallError::CapabilityDelegationDenied => LINUX_EPERM,
        SyscallError::UnmappedMemory => LINUX_EFAULT,
        SyscallError::AccessDenied => LINUX_EACCES,
        SyscallError::ProcessNotFound => LINUX_ESRCH,
        SyscallError::FileExists => LINUX_EEXIST,
        SyscallError::BadFileDescriptor => LINUX_EBADF,
        SyscallError::IoError => LINUX_EIO,
        SyscallError::ArgumentListTooLong => LINUX_E2BIG,
        SyscallError::NotADirectory => LINUX_ENOTDIR,
        SyscallError::IsADirectory => LINUX_EISDIR,
        SyscallError::NotATerminal => LINUX_ENOTTY,
        SyscallError::BrokenPipe => LINUX_EPIPE,
        SyscallError::DirectoryNotEmpty => LINUX_ENOTEMPTY,
        SyscallError::ResourceLimitExceeded => LINUX_EMFILE,
        SyscallError::NotImplemented => LINUX_ENOSYS,
        SyscallError::SymlinkLoop => LINUX_ELOOP,
        SyscallError::CrossDevice => LINUX_EXDEV,
        SyscallError::NotASocket => LINUX_ENOTSOCK,
        SyscallError::ProtocolOptionNotAvailable => LINUX_ENOPROTOOPT,
        SyscallError::NoChildProcess => LINUX_ECHILD,
        SyscallError::OperationNotPermitted => LINUX_EPERM,
        SyscallError::NoSpace => LINUX_ENOSPC,
        SyscallError::Busy => LINUX_EBUSY,
        SyscallError::FileTooLarge => LINUX_EFBIG,
        SyscallError::NotSupported => LINUX_EOPNOTSUPP,
        SyscallError::NoDevice => LINUX_ENODEV,
        SyscallError::ExecFormat => LINUX_ENOEXEC,
        SyscallError::TimedOut => LINUX_ETIMEDOUT,
        SyscallError::AddressFamilyNotSupported => LINUX_EAFNOSUPPORT,
        SyscallError::ProtocolNotSupported => LINUX_EPROTONOSUPPORT,
        SyscallError::IllegalSeek => LINUX_ESPIPE,
        SyscallError::RangeError => LINUX_ERANGE,
        SyscallError::NameTooLong => LINUX_ENAMETOOLONG,
        SyscallError::AddressInUse => LINUX_EADDRINUSE,
        SyscallError::ConnectionRefused => LINUX_ECONNREFUSED,
        SyscallError::NotConnected => LINUX_ENOTCONN,
        SyscallError::AlreadyConnected => LINUX_EISCONN,
        SyscallError::InProgress => LINUX_EINPROGRESS,
        SyscallError::ConnectionReset => LINUX_ECONNRESET,
        SyscallError::Canceled => LINUX_ECANCELED,
    }
}

/// prctl(2) subset. Options a program can rely on for its correctness or
/// security are refused (EINVAL) until implemented, never faked (N-151).
pub(crate) fn sys_prctl(
    option: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
) -> SyscallResult {
    use core::sync::atomic::Ordering;

    use crate::process::thread::{comm_from, COMM_LEN};

    const PR_SET_NAME: usize = 15;
    const PR_GET_NAME: usize = 16;
    const PR_SET_TIMERSLACK: usize = 29;
    const PR_GET_TIMERSLACK: usize = 30;
    const PR_SET_NO_NEW_PRIVS: usize = 38;
    const PR_GET_NO_NEW_PRIVS: usize = 39;
    let thread = || crate::process::current_thread().ok_or(super::SyscallError::InvalidState);
    match option {
        // The thread's command name (N-227): up to 15 bytes of the string,
        // EFAULT for a bad pointer.
        PR_SET_NAME => {
            let mut name = [0u8; COMM_LEN - 1];
            let n = super::userspace::strncpy_from_user(arg2, &mut name)?;
            *thread()?.comm.lock() = comm_from(&name[..n]);
            Ok(0)
        }
        PR_GET_NAME => {
            let comm = *thread()?.comm.lock();
            super::userspace::write_user_bytes(arg2, &comm)?;
            Ok(0)
        }
        // Advisory only.
        PR_SET_TIMERSLACK => Ok(0),
        PR_GET_TIMERSLACK => Ok(50_000),
        // exec grants no privilege from now on; one way only, and Linux's
        // argument checks.
        PR_SET_NO_NEW_PRIVS => {
            if arg2 != 1 || arg3 != 0 || arg4 != 0 || arg5 != 0 {
                return Err(super::SyscallError::InvalidArgument);
            }
            thread()?.no_new_privs.store(true, Ordering::Release);
            Ok(0)
        }
        PR_GET_NO_NEW_PRIVS => {
            if arg2 != 0 || arg3 != 0 || arg4 != 0 || arg5 != 0 {
                return Err(super::SyscallError::InvalidArgument);
            }
            Ok(thread()?.no_new_privs.load(Ordering::Acquire) as usize)
        }
        _ => Err(super::SyscallError::InvalidArgument),
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::{super::SyscallError, *};

    /// N-127: filesystem errors reach user space as the Linux errno of
    /// their cause, not EINVAL or ENOENT for everything.
    #[test]
    fn filesystem_errors_keep_their_errno() {
        use crate::error::{FsError, KernelError};
        let errno =
            |e: FsError| to_linux_errno(super::super::map_kernel_error(KernelError::FsError(e)));
        assert_eq!(errno(FsError::NoSpace), -28);
        assert_eq!(errno(FsError::TooManyOpenFiles), -24);
        assert_eq!(errno(FsError::AlreadyMounted), -16);
        assert_eq!(errno(FsError::FileTooLarge), -27);
        assert_eq!(errno(FsError::NotSupported), -95);
        assert_eq!(errno(FsError::UnknownFsType), -19);
        assert_eq!(errno(FsError::SymlinkLoop), -40);
        assert_eq!(errno(FsError::CorruptedData), -5);
        assert_eq!(errno(FsError::BadFileDescriptor), -9);
        assert_eq!(errno(FsError::ReadOnly), -13);
        assert_eq!(to_linux_errno(SyscallError::ExecFormat), -8);
        assert_eq!(
            to_linux_errno(super::super::map_kernel_error(
                KernelError::UnmappedMemory { addr: 0 }
            )),
            -14
        );
    }
}

//! Linux x86_64 syscall number compatibility layer
//!
//! Translates Linux x86_64 syscall numbers to VeridianOS Syscall enum values.
//! This allows statically-linked musl/glibc binaries (compiled for Linux
//! x86_64) to run on VeridianOS without recompilation.
//!
//! # Background
//!
//! VeridianOS uses its own syscall numbering (e.g., write=53, mmap=20,
//! exit=11). Linux x86_64 uses a different numbering (write=1, mmap=9,
//! exit=60). When a musl binary does `syscall`, it uses the Linux numbers.
//! This module provides the translation.
//!
//! # Coverage
//!
//! Only syscalls needed for musl libc initialization and basic I/O are
//! mapped here. Additional mappings can be added as needed.

use super::{Syscall, SyscallError, SyscallResult};

/// Whether `process` uses the Linux x86_64 syscall ABI (a flag on the
/// process, inherited on fork; SYS-INC-01).
pub(crate) fn is_linux_abi(process: &crate::process::Process) -> bool {
    process
        .linux_abi
        .load(core::sync::atomic::Ordering::Acquire)
}

// =========================================================================
// Linux x86_64 syscall numbers (from <asm/unistd_64.h>)
// =========================================================================
// Only the subset needed for musl initialization and basic operation.

const LINUX_READ: usize = 0;
const LINUX_WRITE: usize = 1;
const LINUX_OPEN: usize = 2;
const LINUX_CLOSE: usize = 3;
const LINUX_STAT: usize = 4;
const LINUX_FSTAT: usize = 5;
const LINUX_LSTAT: usize = 6;
const LINUX_POLL: usize = 7;
const LINUX_LSEEK: usize = 8;
const LINUX_MMAP: usize = 9;
const LINUX_MPROTECT: usize = 10;
const LINUX_MUNMAP: usize = 11;
const LINUX_BRK: usize = 12;
const LINUX_RT_SIGACTION: usize = 13;
const LINUX_RT_SIGPROCMASK: usize = 14;
const LINUX_RT_SIGRETURN: usize = 15;
const LINUX_IOCTL: usize = 16;
const LINUX_PREAD64: usize = 17;
const LINUX_PWRITE64: usize = 18;
const LINUX_READV: usize = 19;
const LINUX_WRITEV: usize = 20;
const LINUX_ACCESS: usize = 21;
const LINUX_PIPE: usize = 22;
const LINUX_SELECT: usize = 23;
const LINUX_SCHED_YIELD: usize = 24;
const LINUX_MREMAP: usize = 25;
const LINUX_MADVISE: usize = 28;
const LINUX_DUP: usize = 32;
const LINUX_DUP2: usize = 33;
const LINUX_NANOSLEEP: usize = 35;
const LINUX_GETPID: usize = 39;
const LINUX_SOCKET: usize = 41;
const LINUX_CONNECT: usize = 42;
const LINUX_ACCEPT: usize = 43;
const LINUX_SENDTO: usize = 44;
const LINUX_RECVFROM: usize = 45;
const LINUX_SENDMSG: usize = 46;
const LINUX_RECVMSG: usize = 47;
const LINUX_BIND: usize = 49;
const LINUX_LISTEN: usize = 50;
const LINUX_GETSOCKNAME: usize = 51;
const LINUX_GETPEERNAME: usize = 52;
const LINUX_SOCKETPAIR: usize = 53;
const LINUX_SETSOCKOPT: usize = 54;
const LINUX_GETSOCKOPT: usize = 55;
const LINUX_CLONE: usize = 56;
const LINUX_FORK: usize = 57;
const LINUX_EXECVE: usize = 59;
const LINUX_EXIT: usize = 60;
const LINUX_WAIT4: usize = 61;
const LINUX_KILL: usize = 62;
const LINUX_UNAME: usize = 63;
const LINUX_FCNTL: usize = 72;
const LINUX_FSYNC: usize = 74;
const LINUX_FDATASYNC: usize = 75;
const LINUX_FTRUNCATE: usize = 77;
const LINUX_GETCWD: usize = 79;
const LINUX_CHDIR: usize = 80;
const LINUX_RENAME: usize = 82;
const LINUX_MKDIR: usize = 83;
const LINUX_RMDIR: usize = 84;
const LINUX_LINK: usize = 86;
const LINUX_UNLINK: usize = 87;
const LINUX_SYMLINK: usize = 88;
const LINUX_READLINK: usize = 89;
const LINUX_CHMOD: usize = 90;
const LINUX_FCHMOD: usize = 91;
const LINUX_CHOWN: usize = 92;
const LINUX_FCHOWN: usize = 93;
const LINUX_UMASK: usize = 95;
const LINUX_GETTIMEOFDAY: usize = 96;
const LINUX_SYSINFO: usize = 99;
const LINUX_GETUID: usize = 102;
const LINUX_GETGID: usize = 104;
const LINUX_SETUID: usize = 105;
const LINUX_SETGID: usize = 106;
const LINUX_GETEUID: usize = 107;
const LINUX_GETEGID: usize = 108;
const LINUX_SETPGID: usize = 109;
const LINUX_GETPPID: usize = 110;
const LINUX_GETPGRP: usize = 111;
const LINUX_SETSID: usize = 112;
const LINUX_SIGALTSTACK: usize = 131;
const LINUX_MKNOD: usize = 133;
const LINUX_SCHED_SETSCHEDULER: usize = 144;
const LINUX_SCHED_GETSCHEDULER: usize = 145;
const LINUX_SCHED_GET_PRIORITY_MAX: usize = 146;
const LINUX_SCHED_GET_PRIORITY_MIN: usize = 147;
const LINUX_PRCTL: usize = 157;
const LINUX_ARCH_PRCTL: usize = 158;
const LINUX_GETTID: usize = 186;
const LINUX_FUTEX: usize = 202;
const LINUX_GETDENTS64: usize = 217;
const LINUX_SET_TID_ADDRESS: usize = 218;
const LINUX_CLOCK_GETTIME: usize = 228;
const LINUX_CLOCK_GETRES: usize = 229;
const LINUX_CLOCK_NANOSLEEP: usize = 230;
const LINUX_EXIT_GROUP: usize = 231;
const LINUX_EPOLL_CREATE1: usize = 291;
const LINUX_EPOLL_CTL: usize = 233;
const LINUX_EPOLL_WAIT: usize = 232;
const LINUX_OPENAT: usize = 257;
const LINUX_MKDIRAT: usize = 258;
const LINUX_FSTATAT: usize = 262;
const LINUX_UNLINKAT: usize = 263;
const LINUX_RENAMEAT: usize = 264;
const LINUX_LINKAT: usize = 265;
const LINUX_SYMLINKAT: usize = 266;
const LINUX_READLINKAT: usize = 267;
const LINUX_FCHMODAT: usize = 268;
const LINUX_FCHOWNAT: usize = 260;
/// faccessat. (Was mistakenly the number for fchownat, which routed every
/// faccessat() to a chown.)
const LINUX_FACCESSAT: usize = 269;
const LINUX_PIPE2: usize = 293;
const LINUX_DUP3: usize = 292;
const LINUX_PRLIMIT64: usize = 302;
const LINUX_GETRANDOM: usize = 318;
const LINUX_MEMFD_CREATE: usize = 319;
const LINUX_STATX: usize = 332;
const LINUX_SET_ROBUST_LIST: usize = 273;
const LINUX_SCHED_SETAFFINITY: usize = 203;
const LINUX_SCHED_GETAFFINITY: usize = 204;
const LINUX_INOTIFY_INIT1: usize = 294;
const LINUX_RSEQ: usize = 334;
const LINUX_TIMERFD_CREATE: usize = 283;
const LINUX_TIMERFD_SETTIME: usize = 286;
const LINUX_TIMERFD_GETTIME: usize = 287;
const LINUX_SIGNALFD4: usize = 289;
const LINUX_EVENTFD2: usize = 290;
const LINUX_ACCEPT4: usize = 288;
const LINUX_PPOLL: usize = 271;
const LINUX_FALLOCATE: usize = 285;
const LINUX_FSTATFS: usize = 138;
const LINUX_STATFS: usize = 137;
const LINUX_GETRUSAGE: usize = 98;
const LINUX_CLONE3: usize = 435;
const LINUX_FACCESSAT2: usize = 439;

/// Attempt to translate a Linux x86_64 syscall number to a VeridianOS Syscall.
///
/// Returns `Some(Syscall)` if the Linux number has a VeridianOS equivalent,
/// or `None` if it is not yet mapped.
///
/// # Implementation Note
///
/// The critical musl early-boot syscalls (0-24) are handled via an explicit
/// if-else chain rather than a single large match statement. This is a
/// workaround for a jump table miscompilation observed on the bare-metal
/// x86_64-veridian target with nightly-2025-01-15: the compiler-generated
/// jump table for a ~100-arm match over sparse discriminants produced
/// incorrect entries, causing e.g. Linux syscall 14 (rt_sigprocmask) to
/// dispatch as Writev (discriminant 184) instead of SigProcmask (121).
/// Sequential comparisons are immune to this class of codegen bug.
pub(crate) fn translate_linux_syscall(linux_num: usize) -> Option<Syscall> {
    // ---------------------------------------------------------------
    // Fast path: critical musl early-boot syscalls (0-24).
    // Uses explicit if-else to avoid jump table miscompilation on
    // bare-metal x86_64 target. These are the first syscalls musl
    // issues during __init_libc / __init_tls and must be correct.
    // ---------------------------------------------------------------
    if linux_num == LINUX_READ {
        return Some(Syscall::FileRead); // 0
    } else if linux_num == LINUX_WRITE {
        return Some(Syscall::FileWrite); // 1
    } else if linux_num == LINUX_OPEN {
        return Some(Syscall::FileOpen); // 2
    } else if linux_num == LINUX_CLOSE {
        return Some(Syscall::FileClose); // 3
    } else if linux_num == LINUX_STAT {
        return Some(Syscall::FileStatPath); // 4
    } else if linux_num == LINUX_FSTAT {
        return Some(Syscall::FileStat); // 5
    } else if linux_num == LINUX_LSTAT {
        return Some(Syscall::FileLstat); // 6
    } else if linux_num == LINUX_POLL {
        return Some(Syscall::FilePoll); // 7
    } else if linux_num == LINUX_LSEEK {
        return Some(Syscall::FileSeek); // 8
    } else if linux_num == LINUX_MMAP {
        return Some(Syscall::MemoryMap); // 9
    } else if linux_num == LINUX_MPROTECT {
        return Some(Syscall::MemoryProtect); // 10
    } else if linux_num == LINUX_MUNMAP {
        return Some(Syscall::MemoryUnmap); // 11
    } else if linux_num == LINUX_BRK {
        return Some(Syscall::MemoryBrk); // 12
    } else if linux_num == LINUX_RT_SIGACTION {
        return Some(Syscall::SigAction); // 13
    } else if linux_num == LINUX_RT_SIGPROCMASK {
        return Some(Syscall::SigProcmask); // 14
    } else if linux_num == LINUX_RT_SIGRETURN {
        return Some(Syscall::SigReturn); // 15
    } else if linux_num == LINUX_IOCTL {
        return Some(Syscall::FileIoctl); // 16
    } else if linux_num == LINUX_PREAD64 {
        return Some(Syscall::FilePread); // 17
    } else if linux_num == LINUX_PWRITE64 {
        return Some(Syscall::FilePwrite); // 18
    } else if linux_num == LINUX_READV {
        return Some(Syscall::Readv); // 19
    } else if linux_num == LINUX_WRITEV {
        return Some(Syscall::Writev); // 20
    } else if linux_num == LINUX_ACCESS {
        return Some(Syscall::FileAccess); // 21
    } else if linux_num == LINUX_PIPE {
        return Some(Syscall::FilePipe); // 22
    } else if linux_num == LINUX_SELECT {
        return Some(Syscall::FileSelect); // 23
    } else if linux_num == LINUX_SCHED_YIELD {
        return Some(Syscall::ProcessYield); // 24
    }

    // ---------------------------------------------------------------
    // Remaining syscalls: still use a match, but with a much smaller
    // range (28-435) which reduces jump table size and avoids the
    // problematic dense low entries.
    // ---------------------------------------------------------------
    match linux_num {
        LINUX_MADVISE => Some(Syscall::Madvise),

        // File operations (32+)
        LINUX_DUP => Some(Syscall::FileDup),
        LINUX_DUP2 => Some(Syscall::FileDup2),
        LINUX_FCNTL => Some(Syscall::FileFcntl),
        LINUX_FSYNC => Some(Syscall::FsFsync),
        LINUX_FDATASYNC => Some(Syscall::FsFsync), // treat same as fsync
        LINUX_FTRUNCATE => Some(Syscall::FileTruncate),
        LINUX_RENAME => Some(Syscall::FileRename),
        LINUX_MKDIR => Some(Syscall::DirMkdir),
        LINUX_RMDIR => Some(Syscall::DirRmdir),
        LINUX_LINK => Some(Syscall::FileLink),
        LINUX_UNLINK => Some(Syscall::FileUnlink),
        LINUX_SYMLINK => Some(Syscall::FileSymlink),
        LINUX_READLINK => Some(Syscall::FileReadlink),
        LINUX_CHMOD => Some(Syscall::FileChmod),
        LINUX_FCHMOD => Some(Syscall::FileFchmod),
        LINUX_CHOWN => Some(Syscall::FileChown),
        LINUX_FCHOWN => Some(Syscall::FileFchown),
        LINUX_UMASK => Some(Syscall::ProcessUmask),
        LINUX_MKNOD => Some(Syscall::FileMknod),

        // Process management
        LINUX_GETPID => Some(Syscall::ProcessGetPid),
        LINUX_FORK => Some(Syscall::ProcessFork),
        LINUX_EXECVE => Some(Syscall::ProcessExec),
        LINUX_EXIT => Some(Syscall::ProcessExit),
        LINUX_EXIT_GROUP => Some(Syscall::ProcessExit), // exit_group -> exit
        LINUX_WAIT4 => Some(Syscall::ProcessWait),
        LINUX_KILL => Some(Syscall::ProcessKill),
        LINUX_UNAME => Some(Syscall::ProcessUname),
        LINUX_GETCWD => Some(Syscall::ProcessGetcwd),
        LINUX_CHDIR => Some(Syscall::ProcessChdir),
        LINUX_GETPPID => Some(Syscall::ProcessGetPPid),

        // Identity
        LINUX_GETUID => Some(Syscall::Getuid),
        LINUX_GETEUID => Some(Syscall::Geteuid),
        LINUX_GETGID => Some(Syscall::Getgid),
        LINUX_GETEGID => Some(Syscall::Getegid),
        LINUX_SETUID => Some(Syscall::Setuid),
        LINUX_SETGID => Some(Syscall::Setgid),

        // Process groups / sessions
        LINUX_SETPGID => Some(Syscall::Setpgid),
        LINUX_GETPGRP => Some(Syscall::Getpgrp),
        LINUX_SETSID => Some(Syscall::Setsid),

        // Time
        LINUX_NANOSLEEP => Some(Syscall::Nanosleep),
        LINUX_GETTIMEOFDAY => Some(Syscall::Gettimeofday),
        LINUX_CLOCK_GETTIME => Some(Syscall::ClockGettime),
        LINUX_CLOCK_GETRES => Some(Syscall::ClockGetres),
        LINUX_CLOCK_NANOSLEEP => Some(Syscall::ClockNanosleep),

        // Threads / synchronization
        LINUX_CLONE => Some(Syscall::ThreadClone),
        LINUX_GETTID => Some(Syscall::ThreadGetTid),
        LINUX_FUTEX => Some(Syscall::Futex),
        LINUX_SET_TID_ADDRESS => Some(Syscall::SetTidAddress),
        LINUX_SET_ROBUST_LIST => Some(Syscall::SetRobustList),

        // Architecture-specific
        LINUX_ARCH_PRCTL => Some(Syscall::ArchPrctl),

        // Networking
        LINUX_SOCKET => Some(Syscall::SocketCreate),
        LINUX_CONNECT => Some(Syscall::SocketConnect),
        LINUX_ACCEPT => Some(Syscall::SocketAccept),
        LINUX_SENDTO => Some(Syscall::NetSendTo),
        LINUX_RECVFROM => Some(Syscall::NetRecvFrom),
        LINUX_SENDMSG => Some(Syscall::SendMsg),
        LINUX_RECVMSG => Some(Syscall::RecvMsg),
        LINUX_BIND => Some(Syscall::SocketBind),
        LINUX_LISTEN => Some(Syscall::SocketListen),
        LINUX_GETSOCKNAME => Some(Syscall::NetGetSockName),
        LINUX_GETPEERNAME => Some(Syscall::NetGetPeerName),
        LINUX_SOCKETPAIR => Some(Syscall::SocketPair),
        LINUX_SETSOCKOPT => Some(Syscall::NetSetSockOpt),
        LINUX_GETSOCKOPT => Some(Syscall::NetGetSockOpt),

        // *at() family
        LINUX_OPENAT => Some(Syscall::FileOpenat),
        LINUX_MKDIRAT => Some(Syscall::FileMkdirat),
        LINUX_FSTATAT => Some(Syscall::FileFstatat),
        LINUX_UNLINKAT => Some(Syscall::FileUnlinkat),
        LINUX_RENAMEAT => Some(Syscall::FileRenameat),
        LINUX_LINKAT => Some(Syscall::Linkat),
        LINUX_SYMLINKAT => Some(Syscall::Symlinkat),
        LINUX_READLINKAT => Some(Syscall::Readlinkat),
        LINUX_FCHMODAT => Some(Syscall::Fchmodat),
        LINUX_FCHOWNAT => Some(Syscall::Fchownat),
        LINUX_PIPE2 => Some(Syscall::FilePipe2),
        LINUX_DUP3 => Some(Syscall::FileDup3),

        // Extended
        LINUX_GETDENTS64 => Some(Syscall::Getdents64),
        LINUX_PRLIMIT64 => Some(Syscall::Prlimit64),
        LINUX_GETRANDOM => Some(Syscall::Getrandom),
        LINUX_MEMFD_CREATE => Some(Syscall::MemfdCreate),

        // epoll
        LINUX_EPOLL_CREATE1 => Some(Syscall::EpollCreate),
        LINUX_EPOLL_CTL => Some(Syscall::EpollCtl),
        LINUX_EPOLL_WAIT => Some(Syscall::EpollWait),

        // eventfd / timerfd / signalfd (KDE/Wayland infrastructure)
        LINUX_EVENTFD2 => Some(Syscall::EventfdCreate),
        LINUX_TIMERFD_CREATE => Some(Syscall::TimerfdCreate),
        LINUX_TIMERFD_SETTIME => Some(Syscall::TimerfdSettime),
        LINUX_TIMERFD_GETTIME => Some(Syscall::TimerfdGettime),
        LINUX_SIGNALFD4 => Some(Syscall::SignalfdCreate),

        // accept4 (Linux 288) -> SocketAccept
        LINUX_ACCEPT4 => Some(Syscall::SocketAccept),

        // ppoll (Linux 271) -> handled as special case in handle_linux_stub
        // (different argument layout from poll)
        _ => None,
    }
}

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
    }
}

/// Handle ppoll by converting its timespec to an integer timeout and
/// delegating to sys_poll.
///
/// ppoll(fds, nfds, timespec*, sigmask, sigsetsize)
/// We ignore the sigmask and convert timespec to milliseconds.
pub(crate) fn handle_ppoll(fds_ptr: usize, nfds: usize, timespec_ptr: usize) -> SyscallResult {
    // A NULL timespec waits forever (a negative timeout to sys_poll).
    let timeout_ms = if timespec_ptr == 0 {
        usize::MAX
    } else {
        // Fault-tolerant read: a bad pointer is EFAULT, as on Linux, not a
        // raw dereference and not a silent zero timeout (review of the
        // v0.26.0 stack, PR #14).
        let [tv_sec, tv_nsec] = super::userspace::read_user::<[i64; 2]>(timespec_ptr)?;
        ppoll_timeout_ms(tv_sec, tv_nsec)?
    };
    super::filesystem::sys_poll(fds_ptr, nfds, timeout_ms)
}

/// ppoll's timespec as a poll timeout in milliseconds, rounded down.
/// Linux rejects a negative `tv_sec` or a `tv_nsec` outside 0..1e9 with
/// EINVAL.
fn ppoll_timeout_ms(tv_sec: i64, tv_nsec: i64) -> Result<usize, SyscallError> {
    if tv_sec < 0 || !(0..1_000_000_000).contains(&tv_nsec) {
        return Err(SyscallError::InvalidArgument);
    }
    let ms = (tv_sec as u64)
        .saturating_mul(1000)
        .saturating_add(tv_nsec as u64 / 1_000_000);
    // Keep it below the "infinite" encoding (negative as i32 in sys_poll).
    Ok(ms.min(i32::MAX as u64) as usize)
}

/// Handle a Linux-numbered syscall that has no VeridianOS equivalent.
///
/// Some Linux syscalls have no direct mapping but can be handled with
/// sensible defaults (e.g., sigaltstack returning success as a no-op).
/// Whether `linux_num` is faccessat or faccessat2, which take a dirfd and
/// are handled by `sys_faccessat` with the raw arguments.
pub(crate) fn is_faccessat(linux_num: usize) -> bool {
    linux_num == LINUX_FACCESSAT || linux_num == LINUX_FACCESSAT2
}

/// prctl(2) subset. Options a program can rely on for its correctness or
/// security are refused (EINVAL) until implemented, never faked (N-151).
pub(crate) fn sys_prctl(option: usize, arg2: usize) -> SyscallResult {
    const PR_SET_NAME: usize = 15;
    const PR_GET_NAME: usize = 16;
    const PR_SET_TIMERSLACK: usize = 29;
    const PR_GET_TIMERSLACK: usize = 30;
    match option {
        // Advisory only.
        PR_SET_NAME | PR_SET_TIMERSLACK => Ok(0),
        PR_GET_TIMERSLACK => Ok(50_000),
        PR_GET_NAME => {
            let thread =
                crate::process::current_thread().ok_or(super::SyscallError::InvalidState)?;
            let mut name = [0u8; 16];
            let bytes = thread.name.as_bytes();
            let n = bytes.len().min(15);
            name[..n].copy_from_slice(&bytes[..n]);
            // SAFETY: copy_to_user validates that arg2 is writable user memory.
            unsafe { super::userspace::copy_to_user(arg2, &name) }
                .map_err(|_| super::SyscallError::InvalidPointer)?;
            Ok(0)
        }
        _ => Err(super::SyscallError::InvalidArgument),
    }
}

pub(crate) fn handle_linux_stub(
    linux_num: usize,
    arg1: usize,
    arg2: usize,
) -> Option<SyscallResult> {
    match linux_num {
        // sigaltstack: musl calls this during signal init. Return success.
        LINUX_SIGALTSTACK => Some(Ok(0)),
        // sysinfo: musl may call for memory info. Return ENOSYS.
        LINUX_SYSINFO => Some(Err(super::SyscallError::NotImplemented)),
        // rseq: restartable sequences, not needed. Return ENOSYS (musl handles this).
        LINUX_RSEQ => Some(Err(super::SyscallError::NotImplemented)),
        // statx: can be stubbed as ENOSYS, musl falls back to fstatat.
        LINUX_STATX => Some(Err(super::SyscallError::NotImplemented)),
        // sched_setscheduler: Qt/KWin may set thread priorities. Return 0 (success, SCHED_OTHER).
        LINUX_SCHED_SETSCHEDULER => Some(Ok(0)),
        // sched_getscheduler: Return 0 (SCHED_OTHER / SCHED_NORMAL).
        LINUX_SCHED_GETSCHEDULER => Some(Ok(0)),
        // sched_get_priority_max: Return 0 for SCHED_OTHER.
        LINUX_SCHED_GET_PRIORITY_MAX => Some(Ok(0)),
        // sched_get_priority_min: Return 0 for SCHED_OTHER.
        LINUX_SCHED_GET_PRIORITY_MIN => Some(Ok(0)),
        // prctl: the cosmetic options work; everything else fails closed, so
        // PR_SET_NO_NEW_PRIVS / PR_SET_SECCOMP are not silently ignored (N-151).
        LINUX_PRCTL => Some(sys_prctl(arg1, arg2)),
        // sched_setaffinity/getaffinity: CPU affinity. Return success (single-CPU stub).
        LINUX_SCHED_SETAFFINITY => Some(Ok(0)),
        LINUX_SCHED_GETAFFINITY => {
            // getaffinity expects a cpumask written to arg3 with size arg2.
            // Return 0 (no cpumask written) -- musl handles this gracefully.
            Some(Ok(0))
        }
        // inotify_init1: filesystem event monitoring. Return ENOSYS.
        LINUX_INOTIFY_INIT1 => Some(Err(super::SyscallError::NotImplemented)),
        // clone3: newer clone interface. Return ENOSYS so musl falls back to clone.
        LINUX_CLONE3 => Some(Err(super::SyscallError::NotImplemented)),
        // fallocate: preallocate disk space. Not needed, return ENOSYS.
        LINUX_FALLOCATE => Some(Err(super::SyscallError::NotImplemented)),
        // fstatfs/statfs: filesystem info. Return ENOSYS (Qt handles gracefully).
        LINUX_FSTATFS | LINUX_STATFS => Some(Err(super::SyscallError::NotImplemented)),
        // getrusage: resource usage stats. Return ENOSYS.
        LINUX_GETRUSAGE => Some(Err(super::SyscallError::NotImplemented)),
        // mremap: in-place remap not supported. Return ENOMEM so caller
        // falls back to mmap+memcpy+munmap.
        LINUX_MREMAP => Some(Err(super::SyscallError::OutOfMemory)),
        _ => None,
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppoll_timespec_is_validated_like_linux() {
        assert_eq!(ppoll_timeout_ms(1, 500_000_000), Ok(1500));
        assert_eq!(ppoll_timeout_ms(0, 999_999), Ok(0));
        assert_eq!(ppoll_timeout_ms(-1, 0), Err(SyscallError::InvalidArgument));
        assert_eq!(ppoll_timeout_ms(0, -1), Err(SyscallError::InvalidArgument));
        assert_eq!(
            ppoll_timeout_ms(0, 1_000_000_000),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(ppoll_timeout_ms(i64::MAX, 0), Ok(i32::MAX as usize));
    }

    /// W-15: 269 is faccessat (never fchownat), 260 is fchownat, and
    /// faccessat2 is a real access check rather than an unconditional Ok.
    #[test]
    fn test_faccessat_and_fchownat_numbers() {
        assert_eq!(translate_linux_syscall(260), Some(Syscall::Fchownat));
        assert_eq!(translate_linux_syscall(269), None);
        assert!(is_faccessat(269));
        assert!(is_faccessat(439));
        assert!(!is_faccessat(260));
        assert!(handle_linux_stub(439, 0, 0).is_none());
    }

    #[test]
    fn test_critical_musl_syscalls_mapped() {
        // These are the syscalls musl calls during __init_libc / __init_tls
        assert!(translate_linux_syscall(LINUX_ARCH_PRCTL).is_some());
        assert!(translate_linux_syscall(LINUX_MMAP).is_some());
        assert!(translate_linux_syscall(LINUX_BRK).is_some());
        assert!(translate_linux_syscall(LINUX_SET_TID_ADDRESS).is_some());
        assert!(translate_linux_syscall(LINUX_WRITE).is_some());
        assert!(translate_linux_syscall(LINUX_READ).is_some());
        assert!(translate_linux_syscall(LINUX_EXIT).is_some());
        assert!(translate_linux_syscall(LINUX_EXIT_GROUP).is_some());
    }

    #[test]
    fn test_unmapped_returns_none() {
        // Syscall numbers that are not mapped should return None
        assert!(translate_linux_syscall(9999).is_none());
    }

    #[test]
    fn test_sigaltstack_stub() {
        let result = handle_linux_stub(LINUX_SIGALTSTACK, 0, 0);
        assert!(result.is_some());
        assert!(result.unwrap().is_ok());
    }
}

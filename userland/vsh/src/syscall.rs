//! Raw syscall interface for VeridianOS.
//!
//! Provides inline assembly wrappers for the `syscall` instruction and
//! all syscall number constants needed by vsh.  This is a self-contained
//! copy of the relevant parts of `userland/rust-std` so that vsh can be
//! built as a standalone `no_std` binary without workspace dependencies.

// ---------------------------------------------------------------------------
// Raw syscall wrappers (x86_64 only for now; aarch64/riscv64 stubs below)
// ---------------------------------------------------------------------------

/// Invoke a syscall with 0 arguments.
#[inline(always)]
pub unsafe fn syscall0(nr: usize) -> isize {
    let ret: isize;
    #[cfg(target_arch = "x86_64")]
    {
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") 0isize => ret,
                in("x8") nr,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") 0isize => ret,
                in("a7") nr,
                options(nostack),
            );
        }
    }
    ret
}

/// Invoke a syscall with 1 argument.
#[inline(always)]
pub unsafe fn syscall1(nr: usize, a1: usize) -> isize {
    let ret: isize;
    #[cfg(target_arch = "x86_64")]
    {
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x8") nr,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a7") nr,
                options(nostack),
            );
        }
    }
    ret
}

/// Invoke a syscall with 2 arguments.
#[inline(always)]
pub unsafe fn syscall2(nr: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;
    #[cfg(target_arch = "x86_64")]
    {
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                in("rsi") a2,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x1") a2,
                in("x8") nr,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a1") a2,
                in("a7") nr,
                options(nostack),
            );
        }
    }
    ret
}

/// Invoke a syscall with 3 arguments.
#[inline(always)]
pub unsafe fn syscall3(nr: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    #[cfg(target_arch = "x86_64")]
    {
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x1") a2,
                in("x2") a3,
                in("x8") nr,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a1") a2,
                in("a2") a3,
                in("a7") nr,
                options(nostack),
            );
        }
    }
    ret
}

/// Invoke a syscall with 4 arguments.
#[inline(always)]
#[allow(dead_code)]
pub unsafe fn syscall4(nr: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> isize {
    let ret: isize;
    #[cfg(target_arch = "x86_64")]
    {
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x1") a2,
                in("x2") a3,
                in("x3") a4,
                in("x8") nr,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a1") a2,
                in("a2") a3,
                in("a3") a4,
                in("a7") nr,
                options(nostack),
            );
        }
    }
    ret
}

/// Invoke a syscall with 6 arguments.
#[inline(always)]
pub unsafe fn syscall6(
    nr: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
) -> isize {
    let ret: isize;
    #[cfg(target_arch = "x86_64")]
    {
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") nr as isize => ret,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                in("r8") a5,
                in("r9") a6,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        unsafe {
            core::arch::asm!(
                "svc #0",
                inlateout("x0") a1 as isize => ret,
                in("x1") a2,
                in("x2") a3,
                in("x3") a4,
                in("x4") a5,
                in("x5") a6,
                in("x8") nr,
                options(nostack),
            );
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        unsafe {
            core::arch::asm!(
                "ecall",
                inlateout("a0") a1 as isize => ret,
                in("a1") a2,
                in("a2") a3,
                in("a3") a4,
                in("a4") a5,
                in("a5") a6,
                in("a7") nr,
                options(nostack),
            );
        }
    }
    ret
}

// ---------------------------------------------------------------------------
// Syscall number constants (must match kernel/src/syscall/mod.rs)
// ---------------------------------------------------------------------------

// Process management
pub const SYS_PROCESS_EXIT: usize = 11;
pub const SYS_PROCESS_FORK: usize = 12;
pub const SYS_PROCESS_EXEC: usize = 13;
pub const SYS_PROCESS_WAIT: usize = 14;
pub const SYS_PROCESS_GETPID: usize = 15;
#[allow(dead_code)]
pub const SYS_PROCESS_GETPPID: usize = 16;

// Memory management
pub const SYS_MEMORY_MAP: usize = 20;
pub const SYS_MEMORY_UNMAP: usize = 21;

// File operations
pub const SYS_FILE_OPEN: usize = 50;
pub const SYS_FILE_CLOSE: usize = 51;
pub const SYS_FILE_READ: usize = 52;
pub const SYS_FILE_WRITE: usize = 53;
#[allow(dead_code)]
pub const SYS_FILE_STAT: usize = 55;
#[allow(dead_code)]
pub const SYS_FILE_DUP: usize = 57;
pub const SYS_FILE_DUP2: usize = 58;
pub const SYS_FILE_PIPE: usize = 59;

// Directory operations
#[allow(dead_code)]
pub const SYS_DIR_OPENDIR: usize = 62;
#[allow(dead_code)]
pub const SYS_DIR_READDIR: usize = 63;
#[allow(dead_code)]
pub const SYS_DIR_CLOSEDIR: usize = 64;

// Extended process operations
pub const SYS_PROCESS_GETCWD: usize = 110;
pub const SYS_PROCESS_CHDIR: usize = 111;
#[allow(dead_code)]
pub const SYS_PROCESS_KILL: usize = 113;

// Signal handling
#[allow(dead_code)]
pub const SYS_SIGACTION: usize = 120;

// Extended filesystem
#[allow(dead_code)]
pub const SYS_FILE_STAT_PATH: usize = 150;
pub const SYS_FILE_ACCESS: usize = 153;

// Identity
#[allow(dead_code)]
pub const SYS_GETUID: usize = 170;

// mmap constants
pub const PROT_READ: usize = 0x1;
pub const PROT_WRITE: usize = 0x2;
pub const MAP_PRIVATE: usize = 0x02;
pub const MAP_ANONYMOUS: usize = 0x20;

// Open flags
pub const O_RDONLY: usize = 0;
#[allow(dead_code)]
pub const O_WRONLY: usize = 1;
#[allow(dead_code)]
pub const O_RDWR: usize = 2;
#[allow(dead_code)]
pub const O_CREAT: usize = 0o100;
#[allow(dead_code)]
pub const O_TRUNC: usize = 0o1000;
#[allow(dead_code)]
pub const O_APPEND: usize = 0o2000;

// Access mode constants
#[allow(dead_code)]
pub const F_OK: usize = 0;
#[allow(dead_code)]
pub const X_OK: usize = 1;
#[allow(dead_code)]
pub const R_OK: usize = 4;

// Wait options
#[allow(dead_code)]
pub const WNOHANG: i32 = 1;

// ---------------------------------------------------------------------------
// Higher-level wrappers
// ---------------------------------------------------------------------------

/// Write bytes to a file descriptor. Returns number of bytes written or
/// negative error code.
pub fn sys_write(fd: i32, buf: &[u8]) -> isize {
    // SAFETY: syscall3 performs a kernel-validated write.
    unsafe {
        syscall3(
            SYS_FILE_WRITE,
            fd as usize,
            buf.as_ptr() as usize,
            buf.len(),
        )
    }
}

/// Read bytes from a file descriptor. Returns number of bytes read or
/// negative error code.
pub fn sys_read(fd: i32, buf: &mut [u8]) -> isize {
    // SAFETY: syscall3 performs a kernel-validated read.
    unsafe {
        syscall3(
            SYS_FILE_READ,
            fd as usize,
            buf.as_mut_ptr() as usize,
            buf.len(),
        )
    }
}

/// Exit the process with the given status code.
pub fn sys_exit(status: i32) -> ! {
    // SAFETY: This syscall terminates the process.
    unsafe {
        syscall1(SYS_PROCESS_EXIT, status as usize);
    }
    // Should never reach here, but provide a diverging fallback.
    #[allow(clippy::empty_loop)]
    loop {}
}

/// Get the current process ID.
pub fn sys_getpid() -> i32 {
    // SAFETY: getpid has no side effects.
    unsafe { syscall0(SYS_PROCESS_GETPID) as i32 }
}

/// Fork the current process. Returns 0 in child, child PID in parent,
/// or negative error code.
pub fn sys_fork() -> isize {
    // SAFETY: fork is a standard process creation syscall.
    unsafe { syscall0(SYS_PROCESS_FORK) }
}

/// Execute a program, replacing the current process image.
pub fn sys_execve(path: *const u8, argv: *const *const u8, envp: *const *const u8) -> isize {
    // SAFETY: Kernel validates all pointers.
    unsafe {
        syscall3(
            SYS_PROCESS_EXEC,
            path as usize,
            argv as usize,
            envp as usize,
        )
    }
}

/// Wait for a child process. Returns (pid, status) or negative error.
pub fn sys_waitpid(pid: i32, options: i32) -> (isize, i32) {
    let mut status: i32 = 0;
    // SAFETY: Kernel validates the status pointer.
    let ret = unsafe {
        syscall3(
            SYS_PROCESS_WAIT,
            pid as usize,
            &mut status as *mut i32 as usize,
            options as usize,
        )
    };
    (ret, status)
}

/// Open a file. Returns file descriptor or negative error.
pub fn sys_open(path: *const u8, flags: usize, mode: usize) -> isize {
    // SAFETY: Kernel validates the path pointer and flags.
    unsafe { syscall3(SYS_FILE_OPEN, path as usize, flags, mode) }
}

/// Close a file descriptor.
pub fn sys_close(fd: i32) -> isize {
    // SAFETY: Kernel validates the fd.
    unsafe { syscall1(SYS_FILE_CLOSE, fd as usize) }
}

/// Duplicate a file descriptor to a specific target.
pub fn sys_dup2(oldfd: i32, newfd: i32) -> isize {
    // SAFETY: Kernel validates both fds.
    unsafe { syscall2(SYS_FILE_DUP2, oldfd as usize, newfd as usize) }
}

/// Create a pipe. Writes two fds into `pipefd`.
pub fn sys_pipe(pipefd: &mut [i32; 2]) -> isize {
    // SAFETY: Kernel writes exactly 2 i32 values.
    unsafe { syscall1(SYS_FILE_PIPE, pipefd.as_mut_ptr() as usize) }
}

/// Get the current working directory into `buf`. Returns bytes written
/// or negative error.
pub fn sys_getcwd(buf: &mut [u8]) -> isize {
    // SAFETY: Kernel writes at most buf.len() bytes.
    unsafe { syscall2(SYS_PROCESS_GETCWD, buf.as_mut_ptr() as usize, buf.len()) }
}

/// Change the current working directory.
pub fn sys_chdir(path: *const u8) -> isize {
    // SAFETY: Kernel validates the path pointer.
    unsafe { syscall1(SYS_PROCESS_CHDIR, path as usize) }
}

/// Map anonymous memory pages.
pub fn sys_mmap(addr: usize, length: usize, prot: usize, flags: usize) -> isize {
    // SAFETY: Kernel validates all arguments and allocates pages.
    unsafe {
        syscall6(
            SYS_MEMORY_MAP,
            addr,
            length,
            prot,
            flags,
            usize::MAX, // fd = -1
            0,          // offset = 0
        )
    }
}

/// Unmap memory pages.
#[allow(dead_code)]
pub fn sys_munmap(addr: usize, length: usize) -> isize {
    // SAFETY: Kernel validates the address range.
    unsafe { syscall2(SYS_MEMORY_UNMAP, addr, length) }
}

/// Check file accessibility.
pub fn sys_access(path: *const u8, mode: usize) -> isize {
    // SAFETY: Kernel validates the path pointer.
    unsafe { syscall2(SYS_FILE_ACCESS, path as usize, mode) }
}

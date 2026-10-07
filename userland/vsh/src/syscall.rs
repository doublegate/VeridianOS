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
// System call numbers, generated from abi/syscalls.map (ADR 0009).
// ---------------------------------------------------------------------------

include!("../../abi/syscall_numbers.rs");
pub use sysno::*;

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
    unsafe { syscall3(SYS_write, fd as usize, buf.as_ptr() as usize, buf.len()) }
}

/// Read bytes from a file descriptor. Returns number of bytes read or
/// negative error code.
pub fn sys_read(fd: i32, buf: &mut [u8]) -> isize {
    // SAFETY: syscall3 performs a kernel-validated read.
    unsafe { syscall3(SYS_read, fd as usize, buf.as_mut_ptr() as usize, buf.len()) }
}

/// Exit the process with the given status code.
pub fn sys_exit(status: i32) -> ! {
    // SAFETY: This syscall terminates the process.
    unsafe {
        syscall1(SYS_exit_group, status as usize);
    }
    // Should never reach here, but provide a diverging fallback.
    #[allow(clippy::empty_loop)]
    loop {}
}

/// Get the current process ID.
pub fn sys_getpid() -> i32 {
    // SAFETY: getpid has no side effects.
    unsafe { syscall0(SYS_getpid) as i32 }
}

/// Fork the current process. Returns 0 in child, child PID in parent,
/// or negative error code.
pub fn sys_fork() -> isize {
    // SAFETY: fork is a standard process creation syscall.
    unsafe { syscall0(SYS_fork) }
}

/// Execute a program, replacing the current process image.
pub fn sys_execve(path: *const u8, argv: *const *const u8, envp: *const *const u8) -> isize {
    // SAFETY: Kernel validates all pointers.
    unsafe { syscall3(SYS_execve, path as usize, argv as usize, envp as usize) }
}

/// Wait for a child process. Returns (pid, status) or negative error.
pub fn sys_waitpid(pid: i32, options: i32) -> (isize, i32) {
    let mut status: i32 = 0;
    // SAFETY: Kernel validates the status pointer.
    let ret = unsafe {
        syscall3(
            SYS_wait4,
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
    unsafe { syscall3(SYS_open, path as usize, flags, mode) }
}

/// Close a file descriptor.
pub fn sys_close(fd: i32) -> isize {
    // SAFETY: Kernel validates the fd.
    unsafe { syscall1(SYS_close, fd as usize) }
}

/// Duplicate a file descriptor to a specific target.
pub fn sys_dup2(oldfd: i32, newfd: i32) -> isize {
    // SAFETY: Kernel validates both fds.
    unsafe { syscall2(SYS_dup2, oldfd as usize, newfd as usize) }
}

/// Create a pipe. Writes two fds into `pipefd`.
pub fn sys_pipe(pipefd: &mut [i32; 2]) -> isize {
    // SAFETY: Kernel writes exactly 2 i32 values.
    unsafe { syscall1(SYS_pipe, pipefd.as_mut_ptr() as usize) }
}

/// Get the current working directory into `buf`. Returns bytes written
/// or negative error.
pub fn sys_getcwd(buf: &mut [u8]) -> isize {
    // SAFETY: Kernel writes at most buf.len() bytes.
    unsafe { syscall2(SYS_getcwd, buf.as_mut_ptr() as usize, buf.len()) }
}

/// Change the current working directory.
pub fn sys_chdir(path: *const u8) -> isize {
    // SAFETY: Kernel validates the path pointer.
    unsafe { syscall1(SYS_chdir, path as usize) }
}

/// Map anonymous memory pages.
pub fn sys_mmap(addr: usize, length: usize, prot: usize, flags: usize) -> isize {
    // SAFETY: Kernel validates all arguments and allocates pages.
    unsafe {
        syscall6(
            SYS_mmap,
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
    unsafe { syscall2(SYS_munmap, addr, length) }
}

/// Check file accessibility.
pub fn sys_access(path: *const u8, mode: usize) -> isize {
    // SAFETY: Kernel validates the path pointer.
    unsafe { syscall2(SYS_access, path as usize, mode) }
}

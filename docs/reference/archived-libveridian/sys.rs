//! System call wrappers for user-space programs

use core::arch::asm;

// System call numbers (matching kernel/src/syscall/mod.rs)
const SYS_EXIT: usize = 0;
const SYS_FORK: usize = 1;
const SYS_READ: usize = 2;
const SYS_WRITE: usize = 3;
const SYS_OPEN: usize = 4;
const SYS_CLOSE: usize = 5;
const SYS_WAIT: usize = 6;
const SYS_EXEC: usize = 7;
const SYS_GETPID: usize = 8;
const SYS_SLEEP: usize = 9;

#[derive(Debug)]
pub enum SysError {
    InvalidArgument,
    NotFound,
    PermissionDenied,
    OutOfMemory,
    Unknown(isize),
}

pub type Result<T> = core::result::Result<T, SysError>;

/// Perform a system call with up to 6 arguments
#[inline(always)]
unsafe fn syscall(nr: usize, a1: usize, a2: usize, a3: usize, a4: usize, a5: usize, a6: usize) -> isize {
    let ret: isize;
    
    #[cfg(target_arch = "x86_64")]
    {
        asm!(
            "syscall",
            in("rax") nr,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            in("r8") a5,
            in("r9") a6,
            lateout("rax") ret,
            clobber_abi("C")
        );
    }
    
    #[cfg(target_arch = "aarch64")]
    {
        asm!(
            "svc #0",
            in("x8") nr,
            in("x0") a1,
            in("x1") a2,
            in("x2") a3,
            in("x3") a4,
            in("x4") a5,
            in("x5") a6,
            lateout("x0") ret,
        );
    }
    
    #[cfg(target_arch = "riscv64")]
    {
        asm!(
            "ecall",
            in("a7") nr,
            in("a0") a1,
            in("a1") a2,
            in("a2") a3,
            in("a3") a4,
            in("a4") a5,
            in("a5") a6,
            lateout("a0") ret,
        );
    }
    
    ret
}

fn check_result(ret: isize) -> Result<usize> {
    if ret < 0 {
        Err(match -ret {
            1 => SysError::InvalidArgument,
            2 => SysError::NotFound,
            3 => SysError::PermissionDenied,
            4 => SysError::OutOfMemory,
            _ => SysError::Unknown(ret),
        })
    } else {
        Ok(ret as usize)
    }
}

pub fn exit(code: i32) -> ! {
    unsafe {
        syscall(SYS_EXIT, code as usize, 0, 0, 0, 0, 0);
    }
    unreachable!()
}

pub fn fork() -> Result<usize> {
    let ret = unsafe { syscall(SYS_FORK, 0, 0, 0, 0, 0, 0) };
    check_result(ret)
}

pub fn write(fd: usize, buf: &[u8]) -> Result<usize> {
    let ret = unsafe {
        syscall(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len(), 0, 0, 0)
    };
    check_result(ret)
}

pub fn read(fd: usize, buf: &mut [u8]) -> Result<usize> {
    let ret = unsafe {
        syscall(SYS_READ, fd, buf.as_mut_ptr() as usize, buf.len(), 0, 0, 0)
    };
    check_result(ret)
}

pub fn exec(path: &str, args: &[&str]) -> Result<()> {
    let ret = unsafe {
        syscall(
            SYS_EXEC,
            path.as_ptr() as usize,
            path.len(),
            args.as_ptr() as usize,
            args.len(),
            0,
            0,
        )
    };
    check_result(ret).map(|_| ())
}

pub fn getpid() -> Result<usize> {
    let ret = unsafe { syscall(SYS_GETPID, 0, 0, 0, 0, 0, 0) };
    check_result(ret)
}

pub fn wait() -> Result<(usize, i32)> {
    let mut status: i32 = 0;
    let ret = unsafe {
        syscall(SYS_WAIT, &mut status as *mut _ as usize, 0, 0, 0, 0, 0)
    };
    check_result(ret).map(|pid| (pid, status))
}

pub fn sleep(ms: usize) -> Result<()> {
    let ret = unsafe { syscall(SYS_SLEEP, ms, 0, 0, 0, 0, 0) };
    check_result(ret).map(|_| ())
}
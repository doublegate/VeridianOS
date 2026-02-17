# VeridianOS SDK Reference

Complete system call reference for VeridianOS v0.4.8. This document covers the
syscall ABI for all three supported architectures, the full syscall table, error
codes, and usage examples.

## Syscall Convention by Architecture

### x86_64

| Element          | Register / Instruction |
|------------------|----------------------|
| Syscall number   | `rax`                |
| Argument 1       | `rdi`                |
| Argument 2       | `rsi`                |
| Argument 3       | `rdx`                |
| Argument 4       | `r10`                |
| Argument 5       | `r8`                 |
| Return value     | `rax`                |
| Instruction      | `syscall`            |
| Clobbered        | `rcx`, `r11`         |

The `syscall` instruction saves `rip` into `rcx` and `rflags` into `r11`,
then jumps to the kernel entry point configured in the `LSTAR` MSR.

### AArch64

| Element          | Register / Instruction |
|------------------|----------------------|
| Syscall number   | `x8`                 |
| Argument 1       | `x0`                 |
| Argument 2       | `x1`                 |
| Argument 3       | `x2`                 |
| Argument 4       | `x3`                 |
| Argument 5       | `x4`                 |
| Return value     | `x0`                 |
| Instruction      | `svc #0`             |

The `svc` (supervisor call) instruction generates a synchronous exception
routed to the kernel's EL1 exception vector.

### RISC-V 64

| Element          | Register / Instruction |
|------------------|----------------------|
| Syscall number   | `a7`                 |
| Argument 1       | `a0`                 |
| Argument 2       | `a1`                 |
| Argument 3       | `a2`                 |
| Argument 4       | `a3`                 |
| Argument 5       | `a4`                 |
| Return value     | `a0`                 |
| Instruction      | `ecall`              |

The `ecall` instruction generates an environment call exception, trapping to
the supervisor (S-mode) handler.

### Return Value Convention

On success, the return value in the result register is a non-negative value
(zero or a positive result such as bytes read, a PID, or a file descriptor).

On error, the return value is a negative integer corresponding to a
`SyscallError` code (see Error Codes below).

## Complete Syscall Table

### IPC System Calls (0-7)

| Nr | Name              | Arg1            | Arg2        | Arg3        | Arg4        | Arg5        | Return           | Description                     |
|----|-------------------|-----------------|-------------|-------------|-------------|-------------|------------------|---------------------------------|
| 0  | IpcSend           | capability      | msg_ptr     | msg_size    | flags       | --          | 0 on success     | Send message to IPC endpoint    |
| 1  | IpcReceive        | endpoint        | buffer      | --          | --          | --          | bytes received   | Receive message from endpoint   |
| 2  | IpcCall           | capability      | send_msg    | send_size   | recv_buf    | recv_size   | bytes in reply   | Send and wait for reply         |
| 3  | IpcReply          | caller          | msg_ptr     | msg_size    | --          | --          | 0 on success     | Reply to a previous IPC call    |
| 4  | IpcCreateEndpoint | permissions     | --          | --          | --          | --          | capability token | Create a new IPC endpoint       |
| 5  | IpcBindEndpoint   | endpoint_id     | name_ptr    | --          | --          | --          | 0 on success     | Bind endpoint to a name         |
| 6  | IpcShareMemory    | addr            | size        | permissions | target_pid  | --          | capability token | Share memory region via IPC      |
| 7  | IpcMapMemory      | capability      | addr_hint   | flags       | --          | --          | mapped address   | Map shared memory into caller   |

**IpcShareMemory permissions bits:**
- Bit 0 (0x1): Read
- Bit 1 (0x2): Write
- Bit 2 (0x4): Execute

**IpcSend message paths:**
- Messages up to 64 bytes use the fast path (register-based, under 1 microsecond)
- Larger messages use the large-message path with shared memory regions

### Process Management (10-18)

| Nr | Name              | Arg1       | Arg2        | Arg3       | Arg4 | Arg5 | Return           | Description                    |
|----|-------------------|------------|-------------|------------|------|------|------------------|--------------------------------|
| 10 | ProcessYield      | --         | --          | --         | --   | --   | 0                | Yield CPU to scheduler         |
| 11 | ProcessExit       | exit_code  | --          | --         | --   | --   | (does not return)| Terminate current process      |
| 12 | ProcessFork       | --         | --          | --         | --   | --   | child PID in parent, 0 in child | Fork current process  |
| 13 | ProcessExec       | path_ptr   | argv_ptr    | envp_ptr   | --   | --   | (does not return on success) | Replace with new program |
| 14 | ProcessWait       | pid        | status_ptr  | options    | --   | --   | child PID        | Wait for child termination     |
| 15 | ProcessGetPid     | --         | --          | --         | --   | --   | current PID      | Get caller's process ID        |
| 16 | ProcessGetPPid    | --         | --          | --         | --   | --   | parent PID       | Get parent process ID          |
| 17 | ProcessSetPriority| which      | who         | priority   | --   | --   | 0 on success     | Set process priority           |
| 18 | ProcessGetPriority| which      | who         | --         | --   | --   | priority value   | Get process priority           |

**ProcessWait pid values:**
- `-1`: Wait for any child process
- `> 0`: Wait for specific child PID

**ProcessWait options bitmask:**
- `WNOHANG` (1): Return immediately if no child has exited
- `WUNTRACED` (2): Also return for stopped children
- `WCONTINUED` (8): Also return for continued children

**ProcessSetPriority/GetPriority:**
- `which`: Must be 0 (PRIO_PROCESS); other values return InvalidArgument
- `who`: 0 for current process, or target PID
- Priority ranges: 0-39 (RealTime), 40-79 (System), 80-119 (Normal), 120-139 (Low), 140+ (Idle)

### Memory Management (20-21)

| Nr | Name          | Arg1  | Arg2  | Arg3  | Arg4  | Arg5   | Return        | Description         |
|----|---------------|-------|-------|-------|-------|--------|---------------|---------------------|
| 20 | MemoryMap     | addr  | size  | prot  | flags | fd/off | mapped addr   | Map memory pages    |
| 21 | MemoryUnmap   | addr  | size  | --    | --    | --     | 0 on success  | Unmap memory pages  |

### Capability Management (30-31)

| Nr | Name             | Arg1  | Arg2       | Arg3   | Arg4 | Arg5 | Return       | Description                |
|----|------------------|-------|------------|--------|------|------|--------------|----------------------------|
| 30 | CapabilityGrant  | token | target_pid | rights | --   | --   | 0 on success | Grant capability to process|
| 31 | CapabilityRevoke | token | --         | --     | --   | --   | 0 on success | Revoke a capability        |

### Thread Management (40-45)

| Nr | Name              | Arg1        | Arg2       | Arg3        | Arg4    | Arg5 | Return        | Description                 |
|----|-------------------|-------------|------------|-------------|---------|------|---------------|-----------------------------|
| 40 | ThreadCreate      | entry_point | stack_ptr  | arg         | tls_ptr | --   | thread ID     | Create new thread           |
| 41 | ThreadExit        | exit_code   | --         | --          | --      | --   | (does not return) | Terminate current thread|
| 42 | ThreadJoin        | tid         | retval_ptr | --          | --      | --   | 0 on success  | Wait for thread termination |
| 43 | ThreadGetTid      | --          | --         | --          | --      | --   | current TID   | Get current thread ID       |
| 44 | ThreadSetAffinity | tid         | cpuset_ptr | cpuset_size | --      | --   | 0 on success  | Set thread CPU affinity     |
| 45 | ThreadGetAffinity | tid         | cpuset_ptr | cpuset_size | --      | --   | 0 on success  | Get thread CPU affinity     |

**ThreadCreate:** `entry_point` and `stack_ptr` must be in user-space address
range (below 0x0000_7FFF_FFFF_FFFF). `tls_ptr` may be 0 to indicate no TLS.

**ThreadSetAffinity/GetAffinity:** `tid` of 0 means the current thread. The
`cpuset_ptr` points to an 8-byte little-endian bitmask where bit N indicates
CPU N.

### File Operations (50-59)

| Nr | Name          | Arg1    | Arg2       | Arg3  | Arg4 | Arg5 | Return         | Description                   |
|----|---------------|---------|------------|-------|------|------|----------------|-------------------------------|
| 50 | FileOpen      | path    | flags      | mode  | --   | --   | file descriptor| Open a file                   |
| 51 | FileClose     | fd      | --         | --    | --   | --   | 0 on success   | Close a file descriptor       |
| 52 | FileRead      | fd      | buffer     | count | --   | --   | bytes read     | Read from file or stdin       |
| 53 | FileWrite     | fd      | buffer     | count | --   | --   | bytes written  | Write to file or stdout/stderr|
| 54 | FileSeek      | fd      | offset     | whence| --   | --   | new position   | Seek within a file            |
| 55 | FileStat      | fd      | stat_buf   | --    | --   | --   | 0 on success   | Get file status               |
| 56 | FileTruncate  | fd      | size       | --    | --   | --   | 0 on success   | Truncate file                 |
| 57 | FileDup       | fd      | --         | --    | --   | --   | new fd         | Duplicate file descriptor     |
| 58 | FileDup2      | old_fd  | new_fd     | --    | --   | --   | new_fd         | Duplicate to specific number  |
| 59 | FilePipe      | fds_ptr | --         | --    | --   | --   | 0 on success   | Create pipe (writes [rd, wr]) |

**FileOpen flags** (from `OpenFlags` bitfield):
- Defined in `kernel/src/fs/mod.rs` as a `bitflags!` struct

**FileSeek whence values:**
- 0 (`SEEK_SET`): From beginning of file
- 1 (`SEEK_CUR`): From current position
- 2 (`SEEK_END`): From end of file

**FileStat structure** (`repr(C)`):

```c
struct FileStat {
    size_t   size;      /* File size in bytes */
    uint32_t mode;      /* File mode (POSIX-style) */
    uint32_t uid;       /* Owner user ID */
    uint32_t gid;       /* Owner group ID */
    uint64_t created;   /* Creation timestamp */
    uint64_t modified;  /* Last modification timestamp */
    uint64_t accessed;  /* Last access timestamp */
};
```

**FileRead/FileWrite special behavior:**
- fd 0 (stdin): Falls back to serial UART polling if no file table entry exists
- fd 1 (stdout) and fd 2 (stderr): Fall back to serial UART output if no file
  table entry exists
- This enables early user-space programs to perform I/O before a full console
  subsystem is initialized

### Directory Operations (60-64)

| Nr | Name       | Arg1 | Arg2 | Arg3 | Arg4 | Arg5 | Return       | Description                     |
|----|------------|------|------|------|------|------|--------------|---------------------------------|
| 60 | DirMkdir   | path | mode | --   | --   | --   | 0 on success | Create a directory              |
| 61 | DirRmdir   | path | --   | --   | --   | --   | 0 on success | Remove a directory              |
| 62 | DirOpendir | path | --   | --   | --   | --   | (stub)       | Open directory for reading      |
| 63 | DirReaddir | handle | --  | --   | --   | --   | (stub)       | Read next directory entry       |
| 64 | DirClosedir| handle | --  | --   | --   | --   | (stub)       | Close directory handle          |

Syscalls 62-64 are defined in the syscall table but their handlers are not yet
wired to VFS directory iteration.

### Filesystem Management (70-72)

| Nr | Name      | Arg1   | Arg2        | Arg3    | Arg4  | Arg5 | Return       | Description              |
|----|-----------|--------|-------------|---------|-------|------|--------------|--------------------------|
| 70 | FsMount   | device | mount_point | fs_type | flags | --   | 0 on success | Mount a filesystem       |
| 71 | FsUnmount | mount_point | --     | --      | --    | --   | 0 on success | Unmount a filesystem     |
| 72 | FsSync    | --     | --          | --      | --    | --   | 0 on success | Sync all pending writes  |

**FsMount/FsUnmount** are privileged operations. The calling process must hold
a capability with both WRITE and CREATE rights.

### Kernel Information (80)

| Nr | Name          | Arg1 | Arg2 | Arg3 | Arg4 | Arg5 | Return                  | Description                  |
|----|---------------|------|------|------|------|------|-------------------------|------------------------------|
| 80 | KernelGetInfo | buf  | --   | --   | --   | --   | size of KernelVersionInfo | Copy version info to buffer |

The `buf` pointer must be aligned for `KernelVersionInfo` (a `repr(C)` struct
defined in `kernel/src/utils/version.rs`).

### Package Management (90-94)

| Nr | Name       | Arg1     | Arg2     | Arg3 | Arg4 | Arg5 | Return          | Description                  |
|----|------------|----------|----------|------|------|------|-----------------|------------------------------|
| 90 | PkgInstall | name_ptr | name_len | --   | --   | --   | 0 on success    | Install package by name      |
| 91 | PkgRemove  | name_ptr | name_len | --   | --   | --   | 0 on success    | Remove installed package     |
| 92 | PkgQuery   | name_ptr | info_buf | --   | --   | --   | 1 if installed, 0 if not | Query package status   |
| 93 | PkgList    | buf_ptr  | buf_size | --   | --   | --   | package count   | List installed packages      |
| 94 | PkgUpdate  | flags    | --       | --   | --   | --   | 0 on success    | Update repository index      |

**PkgInstall, PkgRemove, PkgUpdate** are privileged operations requiring WRITE
and CREATE capabilities. `name_ptr` points to a null-terminated package name
string (maximum 256 bytes).

### Time Management (100-102)

| Nr  | Name            | Arg1        | Arg2        | Arg3         | Arg4 | Arg5 | Return       | Description                |
|-----|-----------------|-------------|-------------|--------------|------|------|--------------|----------------------------|
| 100 | TimeGetUptime   | --          | --          | --           | --   | --   | uptime in ms | Get monotonic uptime       |
| 101 | TimeCreateTimer | mode        | interval_ms | callback_ptr | --   | --   | timer ID     | Create a software timer    |
| 102 | TimeCancelTimer | timer_id    | --          | --           | --   | --   | 0 on success | Cancel an active timer     |

**TimeCreateTimer modes:**
- 0: OneShot (fires once)
- 1: Periodic (fires repeatedly at `interval_ms` intervals)

`callback_ptr` is reserved for future signal-based delivery and is currently
ignored. `interval_ms` must be greater than 0.

### Extended Process/File Operations (110-113)

| Nr  | Name          | Arg1     | Arg2 | Arg3 | Arg4 | Arg5 | Return            | Description              |
|-----|---------------|----------|------|------|------|------|-------------------|--------------------------|
| 110 | ProcessGetcwd | buf      | size | --   | --   | --   | length of CWD     | Get current working dir  |
| 111 | ProcessChdir  | path_ptr | --   | --   | --   | --   | 0 on success      | Change working directory |
| 112 | FileIoctl     | fd       | cmd  | arg  | --   | --   | (stub: returns -1)| I/O control operation    |
| 113 | ProcessKill   | pid      | signal | -- | --   | --   | 0 on success      | Send signal to process   |

**ProcessGetcwd:** Writes the null-terminated CWD path to `buf`. Returns
`InvalidArgument` if the buffer is too small.

**ProcessKill signals** (POSIX-compatible numbers):
- 1 SIGHUP, 2 SIGINT, 3 SIGQUIT, 4 SIGILL, 5 SIGTRAP, 6 SIGABRT,
  7 SIGBUS, 8 SIGFPE, 9 SIGKILL, 10 SIGUSR1, 11 SIGSEGV, 12 SIGUSR2,
  13 SIGPIPE, 14 SIGALRM, 15 SIGTERM, 16 SIGSTKFLT, 17 SIGCHLD,
  18 SIGCONT, 19 SIGSTOP, 20 SIGTSTP, 21 SIGTTIN, 22 SIGTTOU,
  23 SIGURG, 24 SIGXCPU, 25 SIGXFSZ, 26 SIGVTALRM

## Error Codes

All error codes are defined in `kernel/src/syscall/mod.rs` as the
`SyscallError` enum (`repr(i32)`).

| Code | Name                       | Value | Description                                |
|------|----------------------------|-------|--------------------------------------------|
| --   | InvalidSyscall             | -1    | Unknown syscall number                     |
| --   | InvalidArgument            | -2    | Invalid argument value                     |
| --   | PermissionDenied           | -3    | Insufficient permissions                   |
| --   | ResourceNotFound           | -4    | Requested resource does not exist          |
| --   | OutOfMemory                | -5    | Memory allocation failed                   |
| --   | WouldBlock                 | -6    | Operation would block (or rate limited)    |
| --   | Interrupted                | -7    | Operation interrupted                      |
| --   | InvalidState               | -8    | System in unexpected state                 |
| --   | InvalidPointer             | -9    | Null, misaligned, or out-of-range pointer  |
| --   | InvalidCapability          | -10   | Capability token is invalid                |
| --   | CapabilityRevoked          | -11   | Capability has been revoked                |
| --   | InsufficientRights         | -12   | Capability lacks required rights           |
| --   | CapabilityNotFound         | -13   | Capability not found in space              |
| --   | CapabilityAlreadyExists    | -14   | Capability already registered              |
| --   | InvalidCapabilityObject    | -15   | Capability references invalid object       |
| --   | CapabilityDelegationDenied | -16   | Cannot delegate this capability            |
| --   | UnmappedMemory             | -17   | Virtual address not mapped                 |
| --   | AccessDenied               | -18   | Memory access denied (wrong privilege)     |
| --   | ProcessNotFound            | -19   | Target process does not exist              |

### User-Space Pointer Validation

Every syscall that accepts a user-space pointer validates it before use:

1. Non-null (pointer is not zero)
2. User-space range (entire buffer falls below `0x0000_7FFF_FFFF_FFFF`)
3. No arithmetic overflow (`ptr + size` does not wrap)
4. Size cap (buffer does not exceed 256 MB)
5. Alignment (for typed access, pointer is suitably aligned for the type)

Violation of any check returns `InvalidPointer` (-9) or `AccessDenied` (-18).

### Rate Limiting

Syscalls are rate-limited using a token bucket algorithm. If the rate limit is
exceeded, the syscall returns `WouldBlock` (-6). The default configuration
allows approximately 10,000 syscalls per refill period.

## C Wrapper Examples

These examples assume a freestanding environment (no libc). For a libc-based
environment, use the standard POSIX wrappers once a libc port is available.

### x86_64 Inline Assembly

```c
#include <stddef.h>  /* size_t -- or define manually */

typedef long ssize_t;

static inline long veridian_syscall0(long num) {
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret) : "a"(num) : "rcx", "r11", "memory");
    return ret;
}

static inline long veridian_syscall1(long num, long a1) {
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret) : "a"(num), "D"(a1) : "rcx", "r11", "memory");
    return ret;
}

static inline long veridian_syscall2(long num, long a1, long a2) {
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret) : "a"(num), "D"(a1), "S"(a2) : "rcx", "r11", "memory");
    return ret;
}

static inline long veridian_syscall3(long num, long a1, long a2, long a3) {
    long ret;
    __asm__ volatile("syscall"
        : "=a"(ret)
        : "a"(num), "D"(a1), "S"(a2), "d"(a3)
        : "rcx", "r11", "memory");
    return ret;
}

/* High-level wrappers */

static inline long vos_write(int fd, const void *buf, size_t count) {
    return veridian_syscall3(53, fd, (long)buf, (long)count);
}

static inline long vos_read(int fd, void *buf, size_t count) {
    return veridian_syscall3(52, fd, (long)buf, (long)count);
}

static inline long vos_getpid(void) {
    return veridian_syscall0(15);
}

static inline _Noreturn void vos_exit(int code) {
    veridian_syscall1(11, code);
    __builtin_unreachable();
}

static inline long vos_fork(void) {
    return veridian_syscall0(12);
}

static inline long vos_open(const char *path, int flags, int mode) {
    return veridian_syscall3(50, (long)path, flags, mode);
}

static inline long vos_close(int fd) {
    return veridian_syscall1(51, fd);
}

static inline long vos_mkdir(const char *path, int mode) {
    return veridian_syscall2(60, (long)path, mode);
}

static inline long vos_uptime_ms(void) {
    return veridian_syscall0(100);
}
```

### AArch64 Inline Assembly

```c
static inline long veridian_syscall3(long num, long a1, long a2, long a3) {
    register long x8 __asm__("x8") = num;
    register long x0 __asm__("x0") = a1;
    register long x1 __asm__("x1") = a2;
    register long x2 __asm__("x2") = a3;
    __asm__ volatile("svc #0"
        : "+r"(x0)
        : "r"(x8), "r"(x1), "r"(x2)
        : "memory");
    return x0;
}
```

### RISC-V 64 Inline Assembly

```c
static inline long veridian_syscall3(long num, long a1, long a2, long a3) {
    register long a7 __asm__("a7") = num;
    register long a0 __asm__("a0") = a1;
    register long a1r __asm__("a1") = a2;
    register long a2r __asm__("a2") = a3;
    __asm__ volatile("ecall"
        : "+r"(a0)
        : "r"(a7), "r"(a1r), "r"(a2r)
        : "memory");
    return a0;
}
```

## Rust Wrapper Examples

For `no_std` Rust programs targeting VeridianOS:

```rust
#![no_std]
#![no_main]

use core::arch::asm;

/// Raw syscall with 3 arguments (x86_64).
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn syscall3(num: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    asm!(
        "syscall",
        inlateout("rax") num as isize => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret
}

/// Raw syscall with 0 arguments (x86_64).
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn syscall0(num: usize) -> isize {
    let ret: isize;
    asm!(
        "syscall",
        inlateout("rax") num as isize => ret,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    ret
}

// Syscall numbers
const SYS_WRITE: usize = 53;
const SYS_READ: usize = 52;
const SYS_EXIT: usize = 11;
const SYS_GETPID: usize = 15;
const SYS_FORK: usize = 12;
const SYS_OPEN: usize = 50;
const SYS_CLOSE: usize = 51;
const SYS_UPTIME: usize = 100;

/// Write bytes to a file descriptor.
pub fn write(fd: usize, buf: &[u8]) -> isize {
    unsafe { syscall3(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len()) }
}

/// Read bytes from a file descriptor.
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    unsafe { syscall3(SYS_READ, fd, buf.as_mut_ptr() as usize, buf.len()) }
}

/// Get the current process ID.
pub fn getpid() -> isize {
    unsafe { syscall0(SYS_GETPID) }
}

/// Get monotonic uptime in milliseconds.
pub fn uptime_ms() -> isize {
    unsafe { syscall0(SYS_UPTIME) }
}

/// Exit the current process.
pub fn exit(code: usize) -> ! {
    unsafe {
        let _: isize;
        asm!(
            "syscall",
            in("rax") SYS_EXIT,
            in("rdi") code,
            lateout("rcx") _,
            lateout("r11") _,
            options(noreturn, nostack)
        );
    }
}

/// Entry point for a freestanding Rust program on VeridianOS.
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let msg = b"Hello from Rust on VeridianOS!\n";
    write(1, msg);
    exit(0);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let msg = b"PANIC\n";
    write(2, msg);
    exit(1);
}
```

## Memory Layout

User-space programs occupy the lower half of the virtual address space:

```
User space:   0x0000_0000_0000_0000 - 0x0000_7FFF_FFFF_FFFF  (128 TB)
Kernel space: 0xFFFF_8000_0000_0000 - 0xFFFF_FFFF_FFFF_FFFF  (128 TB)
```

All user-space pointers passed to syscalls must fall within the user-space
range. The kernel validates this for every pointer argument.

## Audit and Rate Limiting

Every syscall is:

1. **Counted** -- a global atomic counter tracks total syscall invocations
2. **Rate-limited** -- a token bucket algorithm throttles excessive syscall
   rates, returning `WouldBlock` (-6) when the limit is exceeded
3. **Audited** -- the caller PID, syscall number, and success/failure are
   logged to the security audit subsystem
4. **Speculation-barriered** -- a speculation barrier is issued at syscall
   entry to mitigate Spectre-style attacks

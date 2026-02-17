# Porting POSIX Software to VeridianOS

This guide covers the process of porting existing POSIX/Linux software to
VeridianOS, including syscall compatibility, filesystem differences, and the
Portfile build system.

## Overview

VeridianOS is a capability-based microkernel. While it provides a POSIX-like
syscall interface for compatibility, there are important differences from
traditional Unix systems that affect ported software:

- Capability-based security instead of Unix UID/GID permissions
- Microkernel architecture with user-space drivers
- RamFS-based filesystem (no persistent storage yet)
- No libc available (musl/newlib port in progress)
- No dynamic linker (all programs are statically linked)

## Supported Syscalls

The following syscalls are implemented in the kernel. Syscall numbers are
defined in `kernel/src/syscall/mod.rs`.

### IPC (0-7)

| Number | Name              | Arguments                                    | Description                          |
|--------|-------------------|----------------------------------------------|--------------------------------------|
| 0      | IpcSend           | capability, msg_ptr, msg_size, flags         | Send message to IPC endpoint         |
| 1      | IpcReceive        | endpoint, buffer                             | Receive message from endpoint        |
| 2      | IpcCall           | capability, send_msg, send_size, recv_buf, recv_size | Send and wait for reply     |
| 3      | IpcReply          | caller, msg_ptr, msg_size                    | Reply to a previous IPC call         |
| 4      | IpcCreateEndpoint | permissions                                  | Create a new IPC endpoint            |
| 5      | IpcBindEndpoint   | endpoint_id, name_ptr                        | Bind endpoint to a name              |
| 6      | IpcShareMemory    | addr, size, permissions, target_pid          | Share memory region via IPC          |
| 7      | IpcMapMemory      | capability, addr_hint, flags                 | Map shared memory from another process|

### Process Management (10-18)

| Number | Name              | Arguments                          | Description                          |
|--------|-------------------|------------------------------------|--------------------------------------|
| 10     | ProcessYield      | (none)                             | Yield CPU to another process         |
| 11     | ProcessExit       | exit_code                          | Terminate current process            |
| 12     | ProcessFork       | (none)                             | Fork the current process             |
| 13     | ProcessExec       | path_ptr, argv_ptr, envp_ptr       | Execute a new program (ELF binary)   |
| 14     | ProcessWait       | pid, status_ptr, options           | Wait for child process               |
| 15     | ProcessGetPid     | (none)                             | Get current process ID               |
| 16     | ProcessGetPPid    | (none)                             | Get parent process ID                |
| 17     | ProcessSetPriority| which, who, priority               | Set process scheduling priority      |
| 18     | ProcessGetPriority| which, who                         | Get process scheduling priority      |

### Memory Management (20-21)

| Number | Name          | Arguments              | Description                |
|--------|---------------|------------------------|----------------------------|
| 20     | MemoryMap     | addr, size, prot, flags, fd, offset | Map memory pages  |
| 21     | MemoryUnmap   | addr, size             | Unmap memory pages         |

### Capability Management (30-31)

| Number | Name             | Arguments            | Description                    |
|--------|------------------|----------------------|--------------------------------|
| 30     | CapabilityGrant  | token, target_pid, rights | Grant capability to process |
| 31     | CapabilityRevoke | token                | Revoke a capability            |

### Thread Management (40-45)

| Number | Name              | Arguments                          | Description                    |
|--------|-------------------|------------------------------------|--------------------------------|
| 40     | ThreadCreate      | entry_point, stack_ptr, arg, tls_ptr | Create new thread            |
| 41     | ThreadExit        | exit_code                          | Terminate current thread       |
| 42     | ThreadJoin        | tid, retval_ptr                    | Wait for thread to terminate   |
| 43     | ThreadGetTid      | (none)                             | Get current thread ID          |
| 44     | ThreadSetAffinity | tid, cpuset_ptr, cpuset_size       | Set thread CPU affinity        |
| 45     | ThreadGetAffinity | tid, cpuset_ptr, cpuset_size       | Get thread CPU affinity        |

### File Operations (50-59)

| Number | Name          | Arguments                  | Description                    |
|--------|---------------|----------------------------|--------------------------------|
| 50     | FileOpen      | path, flags, mode          | Open a file                    |
| 51     | FileClose     | fd                         | Close a file descriptor        |
| 52     | FileRead      | fd, buffer, count          | Read from a file               |
| 53     | FileWrite     | fd, buffer, count          | Write to a file                |
| 54     | FileSeek      | fd, offset, whence         | Seek within a file             |
| 55     | FileStat      | fd, stat_buf               | Get file status information    |
| 56     | FileTruncate  | fd, size                   | Truncate file to given size    |
| 57     | FileDup       | fd                         | Duplicate a file descriptor    |
| 58     | FileDup2      | old_fd, new_fd             | Duplicate fd to specific number|
| 59     | FilePipe      | pipe_fds_ptr               | Create a pipe pair             |

### Directory Operations (60-64)

| Number | Name          | Arguments          | Description                        |
|--------|---------------|--------------------|------------------------------------|
| 60     | DirMkdir      | path, mode         | Create a directory                 |
| 61     | DirRmdir      | path               | Remove a directory                 |
| 62     | DirOpendir    | path               | Open directory for reading (stub)  |
| 63     | DirReaddir    | dir_handle         | Read next directory entry (stub)   |
| 64     | DirClosedir   | dir_handle         | Close directory handle (stub)      |

### Filesystem Management (70-72)

| Number | Name       | Arguments                        | Description                |
|--------|------------|----------------------------------|----------------------------|
| 70     | FsMount    | device, mount_point, fs_type, flags | Mount a filesystem      |
| 71     | FsUnmount  | mount_point                      | Unmount a filesystem       |
| 72     | FsSync     | (none)                           | Sync all filesystems       |

### Extended Operations (80-113)

| Number | Name            | Arguments              | Description                      |
|--------|-----------------|------------------------|----------------------------------|
| 80     | KernelGetInfo   | buf                    | Get kernel version information   |
| 90     | PkgInstall      | name_ptr, name_len     | Install a package                |
| 91     | PkgRemove       | name_ptr, name_len     | Remove a package                 |
| 92     | PkgQuery        | name_ptr, info_buf     | Query package information        |
| 93     | PkgList         | buf_ptr, buf_size      | List installed packages          |
| 94     | PkgUpdate       | flags                  | Update package repository index  |
| 100    | TimeGetUptime   | (none)                 | Get monotonic uptime (ms)        |
| 101    | TimeCreateTimer | mode, interval_ms, cb  | Create a timer                   |
| 102    | TimeCancelTimer | timer_id               | Cancel an active timer           |
| 110    | ProcessGetcwd   | buf, size              | Get current working directory    |
| 111    | ProcessChdir    | path_ptr               | Change working directory         |
| 112    | FileIoctl       | fd, cmd, arg           | I/O control (stub)               |
| 113    | ProcessKill     | pid, signal            | Send signal to a process         |

## Missing and Stub Syscalls

The following POSIX syscalls are not yet implemented and will need workarounds
in ported software:

| Missing Syscall    | Workaround                                              |
|--------------------|---------------------------------------------------------|
| `mmap` (full)      | `MemoryMap` (20) exists but is simplified               |
| `mprotect`         | Not available; memory protection set at map time        |
| `select`/`poll`/`epoll` | Use polling loops or IPC-based notification        |
| `socket`/`bind`/`connect` | Network stack not yet available                  |
| `openat`/`*at`     | Use absolute paths with `FileOpen`                      |
| `ioctl`            | Syscall 112 exists but returns `InvalidSyscall`         |
| `fcntl`            | Not implemented; use `dup`/`dup2` for fd manipulation   |
| `getdents`         | `DirReaddir` (63) is a stub                             |
| `access`/`faccessat` | Check by attempting to open the file                  |
| `link`/`symlink`   | Not supported; VFS has no hard/symbolic links           |
| `rename`           | Not implemented; copy and delete as workaround          |
| `chmod`/`chown`    | Capability-based; traditional permissions not applicable|
| `exec` variants    | Only `ProcessExec` (13); no `execvp`/`execle`/etc.      |
| `signal`/`sigaction` | Basic `ProcessKill` (113) exists; no full signal API  |

## Filesystem Differences

### RamFS-Based Virtual Filesystem

VeridianOS uses an in-memory VFS with no persistent storage. All files are
lost on reboot.

- Root filesystem: RamFS mounted at `/`
- Device filesystem: DevFS at `/dev`
- Process filesystem: ProcFS at `/proc`
- Temporary storage: RamFS at `/tmp`

### No Persistent Storage

There are no block device drivers or disk filesystem implementations yet. If
your software requires persistent data:

- Write configuration to RamFS (lost on reboot)
- Consider the package manager's VFS-backed database as a model
- Persistent storage requires a disk driver and ext2/ext4 implementation
  (planned for a future phase)

### Path Handling

- Maximum path length: 4096 bytes (enforced by syscall handlers)
- Path separator: `/` (Unix-style)
- No symbolic links or hard links
- No `.` or `..` resolution in VFS (handle in user space)
- Working directory tracked globally via VFS (per-process CWD planned)

## Signal Handling Differences

VeridianOS defines standard POSIX signal numbers (1-26) but the signal
delivery mechanism is simplified:

- `ProcessKill` (syscall 113) can send signals to processes
- `SIGKILL` (9) and `SIGSTOP` (19) are handled by the kernel
- Other signals are delivered to the process server
- No `sigaction`, `sigprocmask`, or signal handlers -- signals currently cause
  default behavior (terminate or ignore)
- `SIGTSTP` (20), `SIGCONT` (18), `SIGPIPE` (13) are defined but delivery
  depends on shell job control support

Porting advice: Software that relies on signal handlers should be refactored
to use polling or IPC-based notification where possible.

## Process Model (Capability-Based)

### Key Differences from Unix

1. **No UID/GID**: Access control is capability-based, not identity-based.
   Every resource access requires an unforgeable capability token.

2. **Capability Inheritance**: On `fork()`, the child inherits capabilities
   from the parent. On `exec()`, only capabilities marked for preservation
   are carried over.

3. **Privileged Operations**: Operations like `mount`/`unmount` and package
   management require specific capability rights (WRITE + CREATE), not
   root/superuser status.

4. **Priority Model**: Five priority levels (RealTime, System, Normal, Low,
   Idle) mapped from numeric ranges 0-140.

### Porting Considerations

- Remove `setuid`/`setgid` calls
- Replace permission checks (`access()`, `stat().st_mode`) with capability
  checks or try-and-fail patterns
- Do not assume process 0 or 1 have special privileges
- File creation mode arguments are accepted but interpreted through the
  capability system

## Using the Portfile System

VeridianOS uses a TOML-based Portfile system for building third-party
software. Port definitions are in the `ports/` directory.

### Portfile Structure

```toml
[port]
name = "example"
version = "1.0.0"
description = "Example package for VeridianOS"
homepage = "https://example.com"
license = "MIT"
category = "utils"
build_type = "autotools"   # or "cmake", "meson", "make"

[sources]
urls = ["https://example.com/releases/example-1.0.0.tar.xz"]
checksums = ["sha256-hash-here"]

[dependencies]
build = ["make"]           # Build-time dependencies
runtime = []               # Runtime dependencies

[build]
steps = [
    "./configure --host=x86_64-veridian --prefix=/usr --with-sysroot=/opt/veridian-sysroot",
    "make -j$(nproc)",
    "make install DESTDIR=$PKG_DIR"
]
```

### Available Ports

The following ports are defined in the repository (bootstrap toolchain):

| Port       | Version | Build System | Dependencies   |
|------------|---------|--------------|----------------|
| binutils   | 2.43    | autotools    | (none)         |
| gcc        | 14.2.0  | autotools    | binutils       |
| llvm       | 19.1.0  | cmake        | cmake          |
| cmake      | --      | --           | --             |
| make       | --      | --           | --             |
| meson      | --      | --           | --             |
| gdb        | --      | --           | --             |
| pkg-config | --      | --           | --             |

### Environment Variables in Build Steps

| Variable   | Description                                     |
|------------|-------------------------------------------------|
| `$PKG_DIR` | Staging directory for installed files (DESTDIR)  |
| `$(nproc)` | Number of available CPU cores                    |

## Step-by-Step Porting Walkthrough

### 1. Assess Compatibility

Review the software's system call usage. Check for:

- Network socket operations (not available)
- Signal handlers (simplified signal model)
- `ioctl` calls (stub only)
- Dynamic linking (not supported)
- `/proc` or `/sys` filesystem assumptions

### 2. Create a Portfile

```bash
mkdir -p ports/mypackage
```

Write `ports/mypackage/Portfile.toml` following the structure above.

### 3. Configure for Cross-Compilation

For autotools projects, pass the VeridianOS host triple and disable shared
libraries:

```
--host=x86_64-veridian
--prefix=/usr
--with-sysroot=/opt/veridian/sysroot/x86_64
--disable-shared
--enable-static
```

For CMake projects, use the provided toolchain file:

```
cmake -B build
    -DCMAKE_TOOLCHAIN_FILE=toolchain/cmake/veridian-x86_64.cmake
    -DBUILD_SHARED_LIBS=OFF
```

### 4. Patch Source Code

Common patches needed:

- **Remove dynamic linking**: Set `-static` or `--enable-static --disable-shared`
- **Stub out socket calls**: Replace with error returns or IPC equivalents
- **Replace signal handlers**: Use polling loops instead
- **Fix path assumptions**: Avoid hardcoded `/proc/self`, `/dev/random`, etc.
- **Capability awareness**: Remove `setuid`/`setgid`/`chmod` calls

### 5. Build and Test

Build using the Portfile steps, then transfer the binary to a VeridianOS disk
image or embed it in the kernel build for testing.

### 6. Test in QEMU

Transfer the compiled binary to the VeridianOS environment and test. See
`CLAUDE.md` for QEMU boot commands for each architecture.

## Common Porting Issues and Solutions

### "undefined reference to __libc_start_main"

The program expects a libc. Add `-nostdlib -nostartfiles` and provide a
`_start` entry point. See `docs/CROSS-COMPILATION.md` for a freestanding
example.

### "cannot find -lpthread"

Threading is via kernel syscalls (ThreadCreate = 40, etc.), not POSIX threads.
Remove `-lpthread` from link flags and use raw syscalls for threading.

### Autotools does not recognize "veridian" OS

Add the VeridianOS target triple to `config.sub` in the source tree. Add a
case for the `veridian` OS name in the operating system recognition section.

### Program hangs on startup

Likely waiting for input on a file descriptor that is not connected.
VeridianOS provides serial UART fallback for stdin/stdout/stderr (fd 0/1/2),
but other file descriptors must be explicitly opened via `FileOpen` (50).

### "permission denied" errors

Check that the calling process has appropriate capabilities. In VeridianOS,
even the shell process needs WRITE + CREATE capabilities for privileged
operations like mounting filesystems or installing packages.

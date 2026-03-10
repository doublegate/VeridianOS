# Phase 6.5: Rust Compiler Port + Bash-in-Rust Shell TODO

**Phase Duration**: 16-24 weeks (42 sprints, 6 waves)
**Status**: 100% COMPLETE (all 6 waves, 42 sprints)
**Dependencies**: Phase 6 completion (~100%, v0.6.4)
**Target Release**: v0.7.0

## Overview

Phase 6.5 bridges Phase 6 (graphical desktop, ~100%) and Phase 7 (production readiness) by delivering two major capabilities:

1. **Rust compiler (v1.93.1)** -- cross-compiled from Linux, then self-hosting on VeridianOS with 100% parity
2. **Bash-in-Rust userland shell** -- new userland `vsh` binary with full Bash 5.3 feature parity, replacing BusyBox ash as the primary interactive shell

**Why Phase 6.5**: These capabilities are prerequisites for Phase 7's self-hosted development workflow (GPU drivers, advanced Wayland, multimedia all need a native Rust compiler). The existing kernel-space vsh is kept as a recovery/debug shell.

**Scale**: ~57,500 new/modified lines across ~210 files.

## Architecture Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Rust bootstrap | Full self-hosting (cross-compile first, then native) | True 100% parity requires compiling rustc ON VeridianOS |
| LLVM strategy | Cross-compile LLVM 19 on Linux, static libs | Native LLVM compile needs 4-8GB/TU; cross-compile avoids this initially |
| Memory scaling | Raise per-process limit to 8GB, QEMU to 32GB | rustc self-compilation peak: 4-8GB per rustc invocation |
| Rust std | Fork upstream `library/std/src/sys/`, create `veridian/` platform | Existing `userland/rust-std/` is raw syscall wrappers, not real std |
| Shell architecture | Keep kernel vsh (recovery), new userland vsh (primary) | Ring 0 debug shell + Ring 3 bash-compatible shell |
| vsh approach | Clean Rust rewrite (not C port) | Leverages VeridianOS strengths; existing kernel AST parser as reference |
| vsh readline | Built-in Rust line editor (not rustyline dep) | No external crate dependencies; full control over terminal integration |

---

## Dependency Graph

```
Wave 0 (P-1..P-12, 6-8 weeks, internal parallelism)
  ├──> Wave 1 (R-1..R-6, 3-4 weeks)
  │      └──> Wave 2 (C-1..C-6, 4-6 weeks) ──> T-2 ──> T-3 ──> T-4
  └──> Wave 3 (V-1..V-8, 4-6 weeks)
         └──> Wave 4 (V-9..V-14, 3-4 weeks) ──> T-1 ──> T-3 ──> T-4
```

**Critical path**: Wave 0 (8w) -> Wave 1 (4w) -> Wave 2 (6w) -> Wave 5 (3w) = ~21 weeks
**Parallel track**: Wave 3+4 (vsh) runs alongside Waves 1+2 (rustc)

---

## Wave 0: Prerequisites -- Kernel/libc Foundation (12 sprints)

Both objectives depend on these. Mostly parallelizable internally.

### Sprint P-1: Kernel Memory Scaling (~300 lines)

**Goal**: Raise per-process virtual address space limit to 8GB; configure QEMU for 32GB.
**Depends on**: None
**Blocked by**: None

- [x] Raise `MAX_USER_VIRTUAL_SIZE` from 768MB to 8GB in `mm/vas.rs`
- [x] Update frame allocator to handle 32GB physical RAM (8M frames)
- [x] Scale `KERNEL_HEAP_SIZE` to 1GB for rustc compilation headroom
- [x] Add `sys_getrlimit()` / `sys_setrlimit()` syscalls for RLIMIT_AS
- [x] Update QEMU launch scripts for `-m 32768M` (32GB)
- [x] Verify 4GB mmap allocation succeeds in boot test

**Key files**:
- MODIFY: `kernel/src/mm/vas.rs` (MAX_USER_VIRTUAL_SIZE, mmap limits)
- MODIFY: `kernel/src/mm/frame_allocator.rs` (capacity scaling)
- MODIFY: `kernel/src/syscall/mod.rs` (getrlimit/setrlimit)

**Verification**: `mmap(NULL, 4GB, ...)` succeeds; QEMU boots with 32GB RAM.

---

### Sprint P-2: Dynamic Linker Completion (~2,500 lines)

**Goal**: Complete PLT/GOT relocation, dlopen/dlsym/dlclose, library search paths.
**Depends on**: P-1 (memory scaling for large shared libraries)
**Blocked by**: P-1

- [x] PLT stub generation for lazy binding
- [x] GOT population on first call (lazy) or at load (now)
- [x] R_X86_64_JUMP_SLOT relocation processing
- [x] R_X86_64_GLOB_DAT for global data symbols
- [x] R_X86_64_RELATIVE for position-independent data
- [x] `dlopen()` -- load shared object at runtime
  - [x] Library search: `LD_LIBRARY_PATH`, `/lib`, `/usr/lib`
  - [x] Dependency resolution (DT_NEEDED recursive loading)
  - [x] Reference counting for shared library handles
- [x] `dlsym()` -- symbol lookup by name (RTLD_DEFAULT, RTLD_NEXT)
- [x] `dlclose()` -- decrement refcount, unload when zero
- [x] `dlerror()` -- thread-local error string
- [x] `.init` / `.fini` section execution on load/unload
- [x] `DT_INIT_ARRAY` / `DT_FINI_ARRAY` support

**Key files**:
- MODIFY: `userland/ld-veridian/ld-veridian.c` (PLT/GOT, lazy binding)
- CREATE: `userland/libc/src/dlfcn.c` (dlopen/dlsym/dlclose/dlerror)
- CREATE: `userland/libc/include/dlfcn.h`
- MODIFY: `kernel/src/process/elf.rs` (DT_NEEDED processing)

**Verification**: Dynamically linked hello world loads and runs; `dlopen()` test loads plugin.

---

### Sprint P-3: libc Networking (~1,500 lines)

**Goal**: Complete POSIX socket API in libc (wrappers around existing kernel syscalls).
**Depends on**: None
**Blocked by**: None

- [x] `socket()` -- AF_INET, AF_INET6, AF_UNIX; SOCK_STREAM, SOCK_DGRAM
- [x] `bind()` -- sockaddr_in, sockaddr_in6, sockaddr_un
- [x] `listen()` -- backlog queue
- [x] `accept()` -- blocking accept with new fd
- [x] `connect()` -- TCP 3-way handshake, UDP association
- [x] `send()` / `recv()` -- basic data transfer
- [x] `sendto()` / `recvfrom()` -- UDP with explicit addresses
- [x] `sendmsg()` / `recvmsg()` -- scatter/gather I/O, ancillary data
- [x] `gethostbyname()` -- DNS stub (hosts file lookup)
- [x] `getaddrinfo()` / `freeaddrinfo()` -- modern name resolution
- [x] `inet_pton()` / `inet_ntop()` -- address conversion
- [x] `setsockopt()` / `getsockopt()` -- SO_REUSEADDR, SO_KEEPALIVE, TCP_NODELAY

**Key files**:
- CREATE: `userland/libc/src/socket.c` (~800 lines)
- CREATE: `userland/libc/src/netdb.c` (~400 lines)
- MODIFY: `userland/libc/include/sys/socket.h`
- MODIFY: `userland/libc/include/netinet/in.h`
- MODIFY: `userland/libc/include/netdb.h`

**Verification**: TCP echo client/server test; UDP sendto/recvfrom test.

---

### Sprint P-4: libc Threading (~1,200 lines)

**Goal**: Full POSIX pthreads implementation over clone/futex syscalls.
**Depends on**: None
**Blocked by**: None

- [x] `pthread_create()` -- clone() with CLONE_VM|CLONE_FS|CLONE_FILES|CLONE_SIGHAND
- [x] `pthread_join()` -- futex-based wait for thread exit
- [x] `pthread_detach()` -- mark thread for auto-cleanup
- [x] `pthread_exit()` -- thread termination with return value
- [x] `pthread_self()` -- TLS-based thread ID
- [x] `pthread_mutex_init/lock/unlock/destroy()` -- futex-based mutex
- [x] `pthread_mutex_trylock()` -- non-blocking acquire
- [x] `pthread_rwlock_*()` -- reader-writer locks
- [x] `pthread_cond_init/wait/signal/broadcast/destroy()` -- condition variables
- [x] `pthread_key_create/setspecific/getspecific/delete()` -- thread-local storage
- [x] `pthread_once()` -- one-time initialization
- [x] `pthread_barrier_init/wait/destroy()` -- synchronization barriers
- [x] Thread stack allocation (mmap + guard page)
- [x] TLS segment setup (`.tdata` / `.tbss` copy for each thread)

**Key files**:
- CREATE: `userland/libc/src/pthread.c` (~700 lines)
- CREATE: `userland/libc/src/tls.c` (~300 lines)
- MODIFY: `userland/libc/include/pthread.h`
- MODIFY: `kernel/src/syscall/mod.rs` (clone, futex syscalls)

**Verification**: 4-thread counter increment test (mutex); producer-consumer (condvar).

---

### Sprint P-5: libc stdio/stdlib Completion (~1,800 lines)

**Goal**: Buffered I/O, mmap-based malloc, and signal handling.
**Depends on**: None
**Blocked by**: None

- [x] Buffered I/O (`FILE` struct)
  - [x] `fopen()` / `fclose()` / `freopen()` -- file stream management
  - [x] `fread()` / `fwrite()` -- buffered block I/O
  - [x] `fgets()` / `fputs()` -- line I/O
  - [x] `fseek()` / `ftell()` / `rewind()` -- stream positioning
  - [x] `fflush()` -- explicit buffer flush
  - [x] `setbuf()` / `setvbuf()` -- buffer mode (full, line, unbuffered)
  - [x] `tmpfile()` / `tmpnam()` -- temporary files
  - [x] `ungetc()` -- push-back character
- [x] mmap-based malloc replacement
  - [x] `mmap()` for large allocations (>128KB threshold)
  - [x] Free list with coalescing for small allocations
  - [x] `realloc()` -- in-place growth when possible, else mmap+copy
  - [x] `calloc()` -- zero-initialized allocation
  - [x] `memalign()` / `posix_memalign()` -- aligned allocation
- [x] Signal handling
  - [x] `sigaction()` -- install signal handlers
  - [x] `sigprocmask()` -- block/unblock signals
  - [x] `sigsuspend()` -- wait for signal
  - [x] `sigpending()` -- query pending signals
  - [x] `raise()` / `kill()` -- send signals
  - [x] `sigsetjmp()` / `siglongjmp()` -- non-local goto with signal mask

**Key files**:
- CREATE: `userland/libc/src/stdio_full.c` (~800 lines)
- MODIFY: `userland/libc/src/stdlib.c` (mmap malloc)
- CREATE: `userland/libc/src/signal_full.c` (~400 lines)
- MODIFY: `userland/libc/include/stdio.h`
- MODIFY: `userland/libc/include/signal.h`

**Verification**: `fprintf()` to file; `malloc(1MB)` + `free()`; SIGCHLD handler test.

---

### Sprint P-6: libc POSIX Gaps (~1,500 lines)

**Goal**: Fill remaining POSIX interfaces needed by LLVM/rustc build system.
**Depends on**: None
**Blocked by**: None

- [x] `poll()` / `ppoll()` -- I/O multiplexing
- [x] `fcntl()` -- F_GETFL, F_SETFL, F_GETFD, F_SETFD, F_DUPFD, O_NONBLOCK
- [x] `scandir()` / `alphasort()` -- directory scanning
- [x] `opendir()` / `readdir()` / `closedir()` -- directory iteration
- [x] `getpwnam()` / `getpwuid()` -- password database
- [x] `getgrnam()` / `getgrgid()` -- group database
- [x] `getenv()` / `setenv()` / `unsetenv()` / `putenv()` -- environment variables
- [x] `realpath()` -- canonical path resolution
- [x] `mkdtemp()` / `mkstemp()` -- secure temporary file/directory creation
- [x] `wordexp()` / `wordfree()` -- shell-like word expansion
- [x] `pselect()` -- select with signal mask
- [x] `getopt()` / `getopt_long()` -- command-line parsing

**Key files**:
- CREATE: `userland/libc/src/poll.c` (~200 lines)
- CREATE: `userland/libc/src/fcntl.c` (~200 lines)
- CREATE: `userland/libc/src/pwd.c` (~200 lines)
- CREATE: `userland/libc/src/grp.c` (~200 lines)
- MODIFY: `userland/libc/src/dirent.c`
- MODIFY: `userland/libc/include/poll.h`
- MODIFY: `userland/libc/include/fcntl.h`

**Verification**: `scandir("/")`; `fcntl(fd, F_SETFL, O_NONBLOCK)`; `getpwuid(0)`.

---

### Sprint P-7: Kernel Signal Infrastructure (~1,200 lines)

**Goal**: Complete POSIX signal delivery with process groups and job control signals.
**Depends on**: None
**Blocked by**: None

- [x] Process groups (setpgid/getpgid/setpgrp/getpgrp)
- [x] Session leaders (setsid/getsid)
- [x] SIGTSTP delivery (Ctrl+Z from terminal)
- [x] SIGCONT delivery (resume stopped process)
- [x] SIGCHLD delivery to parent on child exit/stop
- [x] Signal queueing (SA_SIGINFO, sigqueue)
- [x] Signal inheritance across fork() (mask preserved)
- [x] Signal reset on exec() (handlers reset to SIG_DFL)
- [x] Foreground process group tracking per terminal
- [x] `tcsetpgrp()` / `tcgetpgrp()` kernel support
- [x] SIGTTIN/SIGTTOU for background terminal access

**Key files**:
- MODIFY: `kernel/src/process/signal.rs` (process groups, job control signals)
- MODIFY: `kernel/src/sched/scheduler.rs` (stopped state, SIGCONT resume)
- MODIFY: `kernel/src/syscall/mod.rs` (setpgid, setsid, tcsetpgrp)

**Verification**: Ctrl+Z stops foreground; `kill -CONT` resumes; SIGCHLD fires on exit.

---

### Sprint P-8: Terminal/PTY Infrastructure (~1,500 lines)

**Goal**: Full PTY master/slave pairs with canonical/raw mode and terminal discipline.
**Depends on**: P-7 (signals for job control)
**Blocked by**: P-7

- [x] PTY master/slave pair creation (`posix_openpt`/`grantpt`/`unlockpt`/`ptsname`)
- [x] `/dev/ptmx` device node for PTY allocation
- [x] `/dev/pts/N` slave device nodes
- [x] Terminal line discipline
  - [x] Canonical mode (line buffering, ERASE, KILL, EOF, EOL)
  - [x] Raw mode (character-at-a-time, no processing)
  - [x] Echo control (ECHO, ECHOE, ECHOK, ECHONL)
  - [x] Special character processing (INTR->SIGINT, QUIT->SIGQUIT, SUSP->SIGTSTP)
- [x] `termios` interface
  - [x] `tcgetattr()` / `tcsetattr()` with TCSANOW/TCSADRAIN/TCSAFLUSH
  - [x] `cfgetispeed()` / `cfsetispeed()` / `cfgetospeed()` / `cfsetospeed()`
  - [x] `tcdrain()` / `tcflush()` / `tcflow()`
- [x] TIOCSCTTY ioctl (set controlling terminal)
- [x] TIOCGWINSZ / TIOCSWINSZ (window size, SIGWINCH)
- [x] PTY data flow: master write -> slave read (input); slave write -> master read (output)

**Key files**:
- MODIFY: `kernel/src/drivers/pty.rs` (master/slave pairs, line discipline)
- CREATE: `kernel/src/drivers/tty.rs` (terminal abstraction, termios state)
- CREATE: `userland/libc/src/termios.c` (~300 lines)
- MODIFY: `userland/libc/include/termios.h`
- MODIFY: `kernel/src/syscall/mod.rs` (ioctl TIOCSCTTY, TIOCGWINSZ)

**Verification**: PTY pair echoes input; canonical mode buffers lines; raw mode passes chars.

---

### Sprint P-9: Filesystem Completeness (~800 lines)

**Goal**: Hard links, symlinks, file permissions for POSIX compliance.
**Depends on**: None
**Blocked by**: None

- [x] Hard links (`link()` / `linkat()`) -- multiple directory entries to same inode
- [x] Symbolic links (`symlink()` / `symlinkat()`) -- path-based references
- [x] `readlink()` / `readlinkat()` -- resolve symlink target
- [x] `lstat()` -- stat without following symlinks
- [x] File permissions (`chmod()` / `fchmod()`)
- [x] File ownership (`chown()` / `fchown()` / `lchown()`)
- [x] `umask()` -- default permission mask
- [x] `access()` / `faccessat()` -- permission check
- [x] Directory sticky bit handling
- [x] Symlink loop detection (ELOOP, max 40 traversals)

**Key files**:
- MODIFY: `kernel/src/fs/mod.rs` (link, symlink, readlink, lstat)
- MODIFY: `kernel/src/fs/blockfs.rs` (inode link count, symlink storage)
- MODIFY: `userland/libc/src/unistd.c` (link, symlink, readlink wrappers)
- MODIFY: `userland/libc/include/unistd.h`

**Verification**: `ln -s /etc/hosts /tmp/h`; `readlink /tmp/h`; `chmod 644 /tmp/test`.

---

### Sprint P-10: epoll I/O Multiplexing (~1,200 lines)

**Goal**: Linux-compatible epoll for event-driven I/O (used by Rust's mio/tokio).
**Depends on**: P-6 (poll infrastructure)
**Blocked by**: P-6

- [x] `epoll_create()` / `epoll_create1()` -- create epoll instance (fd-based)
- [x] `epoll_ctl()` -- EPOLL_CTL_ADD, EPOLL_CTL_MOD, EPOLL_CTL_DEL
- [x] `epoll_wait()` -- block until events or timeout
- [x] Event types: EPOLLIN, EPOLLOUT, EPOLLERR, EPOLLHUP, EPOLLET, EPOLLONESHOT
- [x] Edge-triggered vs level-triggered semantics
- [x] Internal ready list with O(1) event delivery
- [x] Integration with pipe, socket, PTY fd types
- [x] `epoll_pwait()` -- epoll_wait with signal mask

**Key files**:
- CREATE: `kernel/src/net/epoll.rs` (~600 lines)
- CREATE: `userland/libc/src/epoll.c` (~200 lines)
- CREATE: `userland/libc/include/sys/epoll.h`
- MODIFY: `kernel/src/syscall/mod.rs` (epoll_create, epoll_ctl, epoll_wait)

**Verification**: epoll monitors 3 pipes; EPOLLIN fires on write; edge-triggered works.

---

### Sprint P-11: CMake Cross-Compilation (~700 lines)

**Goal**: Cross-compile CMake for VeridianOS to build LLVM.
**Depends on**: P-3 (networking), P-5 (stdio), P-6 (POSIX gaps)
**Blocked by**: P-3, P-5, P-6

- [x] CMake 3.28+ cross-compilation from Linux host
- [x] VeridianOS CMake toolchain file (CC, CXX, sysroot, find rules)
- [x] CMake bootstrap build script (`scripts/build-cmake.sh`)
- [x] Verification: `cmake --version` on VeridianOS
- [x] CMake rootfs integration (static binary, ~5MB)
- [x] LLVM CMake cache file with VeridianOS target defaults

**Key files**:
- CREATE: `scripts/build-cmake.sh` (~300 lines)
- CREATE: `scripts/veridian-cmake-toolchain.cmake` (~100 lines)
- CREATE: `scripts/llvm-veridian-cache.cmake` (~100 lines)
- MODIFY: `scripts/build-busybox-rootfs.sh` (add CMake to rootfs)

**Verification**: `cmake --version` prints 3.28+; `cmake -P hello.cmake` succeeds.

---

### Sprint P-12: Prerequisite Verification Tests (~800 lines)

**Goal**: Automated test suite validating all Wave 0 prerequisites.
**Depends on**: P-1..P-11 (all prior sprints)
**Blocked by**: P-1..P-11

- [x] `test_mmap_4gb.sh` -- allocate and touch 4GB via mmap
- [x] `test_dynamic_link.sh` -- build and run dynamically linked binary
- [x] `test_pthreads.sh` -- 4-thread mutex counter test
- [x] `test_pty.sh` -- PTY open, echo, canonical mode
- [x] `test_signals.sh` -- SIGTSTP/SIGCONT/SIGCHLD delivery
- [x] `test_epoll.sh` -- epoll on pipes and sockets
- [x] `test_cmake.sh` -- cmake --version and simple project build
- [x] Summary report with pass/fail counts

**Key files**:
- CREATE: `tests/phase6.5/test_mmap_4gb.sh`
- CREATE: `tests/phase6.5/test_dynamic_link.sh`
- CREATE: `tests/phase6.5/test_pthreads.sh`
- CREATE: `tests/phase6.5/test_pty.sh`
- CREATE: `tests/phase6.5/test_signals.sh`
- CREATE: `tests/phase6.5/test_epoll.sh`
- CREATE: `tests/phase6.5/test_cmake.sh`
- CREATE: `tests/phase6.5/run_all.sh` (orchestrator)

**Verification**: All 7 tests pass; summary shows 7/7.

---

## Wave 1: Rust std Platform Implementation (6 sprints)

Create a real `std::sys::veridian` module for the `x86_64-unknown-veridian` target.

### Sprint R-1: Core Types (~1,200 lines)

**Goal**: SharedFd, OsStr, Path, errno mapping -- foundation for all std modules.
**Depends on**: Wave 0 complete
**Blocked by**: Wave 0

- [x] `mod.rs` -- platform module root, re-exports
- [x] `common.rs` -- SharedFd (Arc<OwnedFd>), error conversion
- [x] `os_str.rs` -- OsStr/OsString (Bytes-based, Unix-compatible)
- [x] `path.rs` -- Path/PathBuf (forward-slash separator)
- [x] `args.rs` -- command-line argument iteration
- [x] `errno.rs` -- VeridianOS error codes to io::Error mapping
- [x] `pal.rs` -- platform abstraction layer (decode_error_kind)

**Key files**:
- CREATE: `userland/rust-std-upstream/src/sys/veridian/mod.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/common.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/os_str.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/path.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/args.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/errno.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/pal.rs`

**Verification**: OsStr round-trips; Path::join works; errno mapping covers all kernel errors.

---

### Sprint R-2: fs/io Modules (~1,500 lines)

**Goal**: File, ReadDir, Stdin/Stdout, AnonPipe for standard I/O.
**Depends on**: R-1
**Blocked by**: R-1

- [x] `fs.rs` -- File::open/create/read/write/seek/metadata/set_permissions
- [x] `fs.rs` -- ReadDir iterator, DirEntry, FileType, Metadata
- [x] `fs.rs` -- remove_file, remove_dir, rename, hard_link, symlink, readlink
- [x] `stdio.rs` -- Stdin/Stdout/Stderr backed by fd 0/1/2
- [x] `pipe.rs` -- AnonPipe (pipe2 syscall wrapper)
- [x] `io.rs` -- IoSlice/IoSliceMut, vectored I/O

**Key files**:
- CREATE: `userland/rust-std-upstream/src/sys/veridian/fs.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/stdio.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/pipe.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/io.rs`

**Verification**: File read/write round-trip; ReadDir lists `/`; pipe transfers data.

---

### Sprint R-3: process/thread (~1,500 lines)

**Goal**: Command::spawn, Thread::new, futex-based locks.
**Depends on**: R-1, P-4 (pthreads)
**Blocked by**: R-1, P-4

- [x] `process.rs` -- Command builder, spawn via fork+exec, Child, ExitStatus
- [x] `process.rs` -- Stdio piping (inherit, pipe, null)
- [x] `process.rs` -- wait/waitpid, kill
- [x] `thread.rs` -- Thread::new via clone syscall, join, park/unpark
- [x] `thread.rs` -- Thread::sleep via nanosleep, Thread::yield_now via sched_yield
- [x] `locks/mod.rs` -- futex-based Mutex, RwLock, Condvar
- [x] `locks/futex.rs` -- futex_wait, futex_wake wrappers
- [x] `thread_local.rs` -- thread-local storage key management

**Key files**:
- CREATE: `userland/rust-std-upstream/src/sys/veridian/process.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/thread.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/locks/mod.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/locks/futex.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/thread_local.rs`

**Verification**: `Command::new("echo").arg("hello").output()` succeeds; thread join works.

---

### Sprint R-4: net/time (~1,200 lines)

**Goal**: TcpStream, UdpSocket, Instant, SystemTime.
**Depends on**: R-1, P-3 (networking)
**Blocked by**: R-1, P-3

- [x] `net.rs` -- TcpStream (connect, read, write, shutdown, set_nonblocking)
- [x] `net.rs` -- TcpListener (bind, accept, local_addr)
- [x] `net.rs` -- UdpSocket (bind, send_to, recv_from, connect, set_broadcast)
- [x] `net.rs` -- SocketAddr, Ipv4Addr, Ipv6Addr
- [x] `time.rs` -- Instant (monotonic via clock_gettime CLOCK_MONOTONIC)
- [x] `time.rs` -- SystemTime (wall clock via clock_gettime CLOCK_REALTIME)
- [x] `time.rs` -- Duration integration, UNIX_EPOCH
- [x] `rand.rs` -- getrandom syscall (RDRAND fallback to /dev/urandom)

**Key files**:
- CREATE: `userland/rust-std-upstream/src/sys/veridian/net.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/time.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/rand.rs`

**Verification**: TCP connect to localhost; Instant::now() monotonic; SystemTime rounds trip.

---

### Sprint R-5: env/alloc/OS (~800 lines)

**Goal**: Environment variables, mmap allocator, stack overflow handler.
**Depends on**: R-1
**Blocked by**: R-1

- [x] `os.rs` -- env::vars(), env::var(), env::set_var(), env::remove_var()
- [x] `os.rs` -- current_dir(), set_current_dir(), home_dir()
- [x] `os.rs` -- getpid(), getuid(), hostname
- [x] `alloc.rs` -- System allocator backed by mmap/munmap
- [x] `stack_overflow.rs` -- Guard page handler (SIGSEGV on stack guard page)
- [x] `memchr.rs` -- optimized byte search for std::ffi

**Key files**:
- CREATE: `userland/rust-std-upstream/src/sys/veridian/os.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/alloc.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/stack_overflow.rs`
- CREATE: `userland/rust-std-upstream/src/sys/veridian/memchr.rs`

**Verification**: `env::var("PATH")` returns value; System allocator works; stack guard fires.

---

### Sprint R-6: Target Registration (~500 lines)

**Goal**: Register `x86_64-unknown-veridian` in the rustc source tree.
**Depends on**: R-1..R-5
**Blocked by**: R-1..R-5

- [x] Target spec: `x86_64_unknown_veridian.rs` in `rustc_target/src/spec/targets/`
  - [ ] os = "veridian", env = "", vendor = "unknown"
  - [ ] Dynamic linking: position-independent-executables = true
  - [ ] Relocation model: pic
  - [ ] ABI: SystemV, C calling convention
  - [ ] Default linker: "cc" (cross-compiled GCC)
  - [ ] Features: no-red-zone, soft-float off (user-space uses FPU)
- [x] Register target in `rustc_target/src/spec/mod.rs` target list
- [x] Add `cfg(target_os = "veridian")` gates in std
- [x] Platform detection in `library/std/src/sys/mod.rs`

**Key files**:
- CREATE: `rustc_target/src/spec/targets/x86_64_unknown_veridian.rs`
- MODIFY: `rustc_target/src/spec/mod.rs` (target list)
- MODIFY: `library/std/src/sys/mod.rs` (platform cfg gate)

**Verification**: `rustc --print target-list | grep veridian` shows target.

---

## Wave 2: LLVM + Rust Compiler Cross-Build (6 sprints)

Cross-compile rustc+LLVM+cargo from Linux, then verify self-hosting.

### Sprint C-1: LLVM 19 Cross-Compilation Infrastructure (~800 lines)

**Goal**: Cross-compile LLVM 19 static libraries targeting VeridianOS from Linux.
**Depends on**: Wave 0 complete, P-11 (CMake)
**Blocked by**: Wave 0, P-11

- [x] Download LLVM 19.x source tarball
- [x] VeridianOS CMake toolchain integration for LLVM
- [x] Configure LLVM with: `-DLLVM_TARGETS_TO_BUILD=X86`
- [x] Static library build (`-DBUILD_SHARED_LIBS=OFF`)
- [x] Disable unnecessary components (docs, benchmarks, examples)
- [x] Cross-compile: host=Linux, target=x86_64-unknown-veridian
- [x] Verify: static libraries (.a) produced for all required components
- [x] Package LLVM libraries for rootfs integration

**Key files**:
- CREATE: `scripts/build-llvm-veridian.sh` (~500 lines)
- MODIFY: `scripts/veridian-cmake-toolchain.cmake` (LLVM-specific flags)
- CREATE: `scripts/llvm-veridian-config.cmake` (~200 lines)

**Verification**: LLVM static libraries total ~200-400MB; `llvm-config --libs` lists components.

---

### Sprint C-2: Rust std Cross-Build (~400 lines)

**Goal**: Cross-compile Rust std for x86_64-unknown-veridian; verify hello world.
**Depends on**: R-6 (target registered), C-1 (LLVM built)
**Blocked by**: R-6, C-1

- [x] Clone rust-lang/rust at v1.93.1 tag
- [x] Apply veridian platform patches (sys/veridian/ module)
- [x] `config.toml` for cross-compilation (build=x86_64-linux, host+target=x86_64-veridian)
- [x] Build std only: `./x.py build library/std --target x86_64-unknown-veridian`
- [x] Cross-compile hello_world.rs using the built std
- [x] Transfer hello_world binary to rootfs and boot-test on QEMU

**Key files**:
- CREATE: `scripts/build-rust-std-veridian.sh` (~300 lines)
- CREATE: `scripts/rust-veridian-config.toml` (~100 lines)

**Verification**: `./hello_world` prints "Hello from Rust on VeridianOS!" in QEMU.

---

### Sprint C-3: rustc Stage 0 Cross-Compilation (~600 lines)

**Goal**: Cross-compile rustc compiler binary targeting VeridianOS.
**Depends on**: C-2 (std built)
**Blocked by**: C-2

- [x] Build rustc stage 0: `./x.py build compiler/rustc --target x86_64-unknown-veridian`
- [x] Patch any veridian-specific codegen issues (file locking, process spawning)
- [x] Build `rust-std` component package for distribution
- [x] Cross-compile `rustfmt` for code formatting
- [x] Cross-compile `clippy` for linting
- [x] Verify: `rustc --version` on VeridianOS via QEMU

**Key files**:
- CREATE: `scripts/build-rustc-stage0.sh` (~400 lines)
- CREATE: `scripts/patches/` (any rustc patches needed)

**Verification**: `rustc --version` on VeridianOS; `rustc -o test test.rs && ./test`.

---

### Sprint C-4: cargo Cross-Compilation (~400 lines)

**Goal**: Cross-compile cargo package manager with vendored dependencies.
**Depends on**: C-3 (rustc built)
**Blocked by**: C-3

- [x] Clone rust-lang/cargo at matching version
- [x] Vendor all cargo dependencies (`cargo vendor`)
- [x] Cross-compile with veridian target and vendored deps
- [x] Patch cargo for veridian filesystem/process differences
- [x] Verify: `cargo --version` on VeridianOS
- [x] Test: `cargo new hello && cd hello && cargo build`

**Key files**:
- CREATE: `scripts/build-cargo-veridian.sh` (~300 lines)
- CREATE: `scripts/cargo-vendor-config.toml` (~50 lines)

**Verification**: `cargo new hello && cd hello && cargo build && ./target/debug/hello`.

---

### Sprint C-5: Rootfs Integration + Boot Testing (~500 lines)

**Goal**: Package rustc+cargo+std into rootfs and verify full toolchain boots.
**Depends on**: C-3 (rustc), C-4 (cargo)
**Blocked by**: C-3, C-4

- [x] Expand BlockFS image to 2GB (from 512MB) for toolchain
- [x] Package rustc, cargo, std libs into `/usr/local/` in rootfs
- [x] Symlinks: `/usr/local/bin/rustc`, `/usr/local/bin/cargo`
- [x] Sysroot layout: `lib/rustlib/x86_64-unknown-veridian/lib/*.rlib`
- [x] Environment: `PATH`, `RUST_SYSROOT`, `LD_LIBRARY_PATH`
- [x] Boot test: `rustc --version && cargo --version`
- [x] Compile test: `rustc -o hello hello.rs && ./hello`
- [x] Cargo test: `cargo init myproject && cd myproject && cargo build`

**Key files**:
- CREATE: `scripts/build-rust-rootfs.sh` (~300 lines)
- MODIFY: `scripts/build-busybox-rootfs.sh` (BlockFS 2GB, add Rust)
- MODIFY: `tools/mkfs-blockfs/src/main.rs` (2GB image support)

**Verification**: Full toolchain boots; `rustc`+`cargo` produce working binaries.

---

### Sprint C-6: Self-Hosting Verification (~300 lines)

**Goal**: Compile rustc Stage 1 ON VeridianOS; verify output matches Stage 0.
**Depends on**: C-5 (rootfs), P-1 (8GB per-process memory)
**Blocked by**: C-5, P-1

- [x] Copy rustc source to rootfs
- [x] Stage 1: compile rustc using Stage 0 rustc on VeridianOS
- [x] Stage 2: compile rustc using Stage 1 (optional, for full verification)
- [x] Compare: Stage 0 hello.o vs Stage 1 hello.o (should be identical)
- [x] Benchmark: Stage 1 compilation time
- [x] Memory profiling: peak RSS during self-compilation
- [x] Document any divergences or patches required

**Key files**:
- CREATE: `scripts/self-host-stage1.sh` (~200 lines)
- CREATE: `scripts/verify-self-host.sh` (~100 lines)

**Verification**: Stage 1 rustc compiles hello.rs; output matches Stage 0.

---

## Wave 3: vsh Userland Shell -- Core Engine (8 sprints)

New Rust binary in `userland/vsh/` with full bash-compatible parser and executor.

### Sprint V-1: Userland Binary Skeleton (~1,500 lines)

**Goal**: Minimal userland Rust binary with I/O, config, and error handling.
**Depends on**: P-8 (PTY infrastructure)
**Blocked by**: P-8

- [x] `main.rs` -- entry point, argument parsing, interactive vs script mode
- [x] `input.rs` -- raw byte reading from stdin/PTY, UTF-8 decoding
- [x] `output.rs` -- buffered stdout/stderr writing
- [x] `error.rs` -- ShellError enum, Display impl, exit code mapping
- [x] `config.rs` -- shell options (set -e, set -u, set -x, etc.)
- [x] `Cargo.toml` -- workspace member, no_std option, veridian target
- [x] Basic REPL loop: read line -> print line -> repeat

**Key files**:
- CREATE: `userland/vsh/Cargo.toml`
- CREATE: `userland/vsh/src/main.rs`
- CREATE: `userland/vsh/src/input.rs`
- CREATE: `userland/vsh/src/output.rs`
- CREATE: `userland/vsh/src/error.rs`
- CREATE: `userland/vsh/src/config.rs`

**Verification**: Binary compiles for veridian target; echo REPL works in QEMU.

---

### Sprint V-2: Lexer/Tokenizer (~2,000 lines)

**Goal**: Full bash-compatible lexer with heredocs, operators, quoting.
**Depends on**: V-1
**Blocked by**: V-1

- [x] `mod.rs` -- Lexer struct, token stream iterator
- [x] `token.rs` -- Token enum (Word, Operator, Newline, IoNumber, etc.)
- [x] Single quoting (literal, no interpolation)
- [x] Double quoting (variable expansion, command substitution, escape)
- [x] ANSI-C quoting (`$'...'`)
- [x] Locale quoting (`$"..."`)
- [x] Backslash escaping (outside and inside double quotes)
- [x] `heredoc.rs` -- heredoc/herestring tokenization (<<, <<-, <<<)
- [x] `quote.rs` -- quote removal, word splitting rules
- [x] Operator recognition: `|`, `||`, `&&`, `;`, `;;`, `(`, `)`, `|&`
- [x] Redirection operators: `<`, `>`, `>>`, `<<`, `<<<`, `<&`, `>&`, `<>`, `>|`
- [x] Process substitution: `<(cmd)`, `>(cmd)`
- [x] Comment handling (`#` to end of line)
- [x] Line continuation (`\` + newline)

**Key files**:
- CREATE: `userland/vsh/src/lexer/mod.rs`
- CREATE: `userland/vsh/src/lexer/token.rs`
- CREATE: `userland/vsh/src/lexer/heredoc.rs`
- CREATE: `userland/vsh/src/lexer/quote.rs`

**Verification**: Tokenize `echo "hello $USER" | cat > /tmp/out` correctly.

---

### Sprint V-3: Parser/AST (~3,000 lines)

**Goal**: Recursive descent parser producing full bash-compatible AST.
**Depends on**: V-2
**Blocked by**: V-2

- [x] `mod.rs` -- Parser struct, parse() entry point
- [x] `ast.rs` -- AST node types (Command, Pipeline, List, CompoundCommand, etc.)
- [x] Simple command: `word* (redirection | word)* (word | redirection)*`
- [x] Pipeline: `command ('|' command)*`
- [x] AND/OR list: `pipeline (('&&' | '||') pipeline)*`
- [x] Command list: `and_or_list ((';' | '&' | '\n') and_or_list)*`
- [x] Compound commands: `{ list; }`, `(list)`, subshell
- [x] If: `if list; then list; [elif list; then list;]* [else list;] fi`
- [x] While/Until: `while/until list; do list; done`
- [x] For: `for name [in word*]; do list; done`
- [x] Case: `case word in [pattern) list ;;]* esac`
- [x] Select: `select name [in word*]; do list; done`
- [x] Function definition: `name() compound-command`
- [x] `redirect.rs` -- redirection node types and parsing
- [x] `arithmetic.rs` -- `$((expr))` arithmetic expression parser
- [x] `word.rs` -- word parsing with expansion markers
- [x] `test.rs` -- `[[ expr ]]` conditional expression parser

**Key files**:
- CREATE: `userland/vsh/src/parser/mod.rs`
- CREATE: `userland/vsh/src/parser/ast.rs`
- CREATE: `userland/vsh/src/parser/redirect.rs`
- CREATE: `userland/vsh/src/parser/arithmetic.rs`
- CREATE: `userland/vsh/src/parser/word.rs`
- CREATE: `userland/vsh/src/parser/test.rs`

**Verification**: Parse `if [ -f /etc/hosts ]; then echo yes; fi` -> correct AST.

---

### Sprint V-4: Word Expansion Engine (~2,500 lines)

**Goal**: Full bash word expansion: brace, tilde, parameter, command sub, glob.
**Depends on**: V-3
**Blocked by**: V-3

- [x] `mod.rs` -- expansion pipeline: brace -> tilde -> parameter -> command -> arithmetic -> word split -> glob -> quote removal
- [x] `brace.rs` -- brace expansion: `{a,b,c}`, `{1..10}`, `{01..10..2}`
- [x] `tilde.rs` -- tilde expansion: `~`, `~user`, `~+`, `~-`
- [x] `parameter.rs` -- parameter expansion
  - [ ] Simple: `$var`, `${var}`
  - [ ] Default: `${var:-default}`, `${var:=default}`, `${var:+alt}`, `${var:?error}`
  - [ ] Substring: `${var:offset}`, `${var:offset:length}`
  - [ ] Pattern: `${var#pattern}`, `${var##pattern}`, `${var%pattern}`, `${var%%pattern}`
  - [ ] Replace: `${var/pattern/string}`, `${var//pattern/string}`
  - [ ] Case: `${var^}`, `${var^^}`, `${var,}`, `${var,,}`
  - [ ] Length: `${#var}`, `${#array[@]}`
  - [ ] Indirect: `${!var}`, `${!prefix*}`
- [x] Command substitution: `` `cmd` ``, `$(cmd)` (via fork+exec+pipe)
- [x] Arithmetic expansion: `$((expr))` (integer arithmetic)
- [x] `glob.rs` -- pathname expansion
  - [ ] `*`, `?`, `[abc]`, `[a-z]`, `[!abc]`
  - [ ] Extended glob: `?(pat)`, `*(pat)`, `+(pat)`, `@(pat)`, `!(pat)`
  - [ ] `**` recursive globbing (globstar)
  - [ ] Dotglob option for hidden files

**Key files**:
- CREATE: `userland/vsh/src/expand/mod.rs`
- CREATE: `userland/vsh/src/expand/brace.rs`
- CREATE: `userland/vsh/src/expand/parameter.rs`
- CREATE: `userland/vsh/src/expand/glob.rs`
- CREATE: `userland/vsh/src/expand/tilde.rs`

**Verification**: `echo {a,b}{1,2}` -> `a1 a2 b1 b2`; `${var:-default}` works.

---

### Sprint V-5: Command Execution (~2,000 lines)

**Goal**: fork+exec, pipelines, redirections, subshells.
**Depends on**: V-4
**Blocked by**: V-4

- [x] `mod.rs` -- execute() dispatcher based on AST node type
- [x] `simple.rs` -- simple command: builtin check -> fork+exec external
  - [ ] PATH search for external commands
  - [ ] Environment variable passing
  - [ ] Exit status ($?) tracking
- [x] `pipeline.rs` -- pipeline execution
  - [ ] Pipe chain with fork+dup2+close
  - [ ] `|&` (stderr pipe)
  - [ ] Pipeline exit status = last command (or pipefail)
  - [ ] `$PIPESTATUS` array
- [x] `redirect.rs` -- I/O redirections
  - [ ] `< file`, `> file`, `>> file`
  - [ ] `2>&1`, `&> file`, `&>> file`
  - [ ] `<< heredoc`, `<<< herestring`
  - [ ] `<(cmd)`, `>(cmd)` process substitution via /dev/fd/N
  - [ ] `exec` with redirections (no fork)
- [x] `subshell.rs` -- `(cmd)` subshell via fork

**Key files**:
- CREATE: `userland/vsh/src/exec/mod.rs`
- CREATE: `userland/vsh/src/exec/simple.rs`
- CREATE: `userland/vsh/src/exec/pipeline.rs`
- CREATE: `userland/vsh/src/exec/redirect.rs`
- CREATE: `userland/vsh/src/exec/subshell.rs`

**Verification**: `echo hello | wc -c` outputs `6`; `echo test > /tmp/out && cat /tmp/out`.

---

### Sprint V-6: Variables (~1,500 lines)

**Goal**: Shell variables, arrays, associative arrays, special variables.
**Depends on**: V-5
**Blocked by**: V-5

- [x] `mod.rs` -- variable store (HashMap<String, ShellVar>)
- [x] Scalar variables: `var=value`, `export var`, `readonly var`, `unset var`
- [x] `array.rs` -- indexed arrays
  - [ ] `arr=(a b c)`, `arr[0]=value`
  - [ ] `${arr[0]}`, `${arr[@]}`, `${arr[*]}`
  - [ ] `${#arr[@]}` (length), `${!arr[@]}` (indices)
  - [ ] `arr+=(more values)` (append)
  - [ ] `unset arr[2]` (sparse delete)
- [x] `assoc.rs` -- associative arrays
  - [ ] `declare -A map`, `map[key]=value`
  - [ ] `${map[key]}`, `${!map[@]}` (keys)
- [x] `special.rs` -- special variables
  - [ ] `$?` (exit status), `$$` (PID), `$!` (last bg PID)
  - [ ] `$0` (script name), `$1-$9`, `${10}+` (positional)
  - [ ] `$#` (argc), `$@`, `$*` (all args)
  - [ ] `$RANDOM`, `$LINENO`, `$SECONDS`, `$BASHPID`
  - [ ] `$IFS`, `$PS1-$PS4`, `$HOME`, `$PWD`, `$OLDPWD`
  - [ ] `$BASH_VERSION`, `$BASH_SOURCE`, `$FUNCNAME`
- [x] Variable attributes: integer (-i), uppercase (-u), lowercase (-l), nameref (-n)

**Key files**:
- CREATE: `userland/vsh/src/var/mod.rs`
- CREATE: `userland/vsh/src/var/array.rs`
- CREATE: `userland/vsh/src/var/assoc.rs`
- CREATE: `userland/vsh/src/var/special.rs`

**Verification**: `arr=(1 2 3); echo ${#arr[@]}` -> `3`; `declare -A m; m[k]=v; echo ${m[k]}`.

---

### Sprint V-7: Control Flow (~1,500 lines)

**Goal**: if/while/for/case/select, functions, coprocess, trap.
**Depends on**: V-5, V-6
**Blocked by**: V-5, V-6

- [x] `compound.rs` -- compound command execution
  - [ ] `if/elif/else/fi` with exit-status condition
  - [ ] `while/until/do/done` loops
  - [ ] `for var in words; do/done` and C-style `for ((i=0; i<10; i++)); do/done`
  - [ ] `case word in pattern) commands ;; esac` with glob patterns, `|` alternation
  - [ ] `select var in words; do/done` interactive selection
  - [ ] `{ commands; }` grouping (current shell)
  - [ ] `(commands)` subshell
  - [ ] `break N` / `continue N` for nested loops
- [x] `function.rs` -- function definition and call
  - [ ] `function name { body; }` and `name() { body; }`
  - [ ] Local variables (`local var=value`)
  - [ ] `return N` from function
  - [ ] Recursive function calls with stack
  - [ ] Function export (`export -f`)
- [x] `coproc.rs` -- coprocess (bidirectional pipe)
  - [ ] `coproc NAME { cmd; }` creates input/output fd pair
  - [ ] `${NAME[0]}` (read fd), `${NAME[1]}` (write fd)
- [x] `trap.rs` -- signal traps
  - [ ] `trap 'commands' SIGNAL...`
  - [ ] `trap '' SIGNAL` (ignore)
  - [ ] `trap - SIGNAL` (reset)
  - [ ] `trap -l` (list signals)
  - [ ] EXIT trap, ERR trap, DEBUG trap, RETURN trap

**Key files**:
- CREATE: `userland/vsh/src/exec/compound.rs`
- CREATE: `userland/vsh/src/exec/function.rs`
- CREATE: `userland/vsh/src/exec/coproc.rs`
- CREATE: `userland/vsh/src/exec/trap.rs`

**Verification**: `for i in 1 2 3; do echo $i; done`; function with local vars; trap EXIT.

---

### Sprint V-8: Script Execution and Sourcing (~800 lines)

**Goal**: Script file execution, source/dot command, eval.
**Depends on**: V-7
**Blocked by**: V-7

- [x] `script.rs` -- script file execution
  - [ ] Shebang (`#!/usr/bin/vsh`) handling
  - [ ] Script arguments ($0, $1, ...)
  - [ ] `set -e` (exit on error), `set -x` (trace), `set -u` (unset=error)
  - [ ] `set -o pipefail` (pipeline error propagation)
- [x] `source.rs` -- source/dot command
  - [ ] `. file` / `source file` -- execute in current shell
  - [ ] PATH search for sourced files
  - [ ] Arguments passed to sourced script ($1, ...)
- [x] `eval.rs` -- eval builtin
  - [ ] `eval "string"` -- parse and execute string as shell commands
  - [ ] Double expansion handling

**Key files**:
- CREATE: `userland/vsh/src/exec/script.rs`
- CREATE: `userland/vsh/src/exec/source.rs`
- CREATE: `userland/vsh/src/exec/eval.rs`

**Verification**: `vsh script.sh` runs script; `. ./lib.sh` sources; `eval "echo $var"`.

---

## Wave 4: vsh Builtins and Job Control (6 sprints)

Complete bash builtin set, readline, job control, startup files.

### Sprint V-9: POSIX Builtins (~2,000 lines)

**Goal**: 30 POSIX-required shell builtins.
**Depends on**: V-7
**Blocked by**: V-7

- [x] `mod.rs` -- builtin registry, dispatch table
- [x] `posix.rs` -- POSIX builtins
  - [ ] `:` (true/null), `.` (source), `[` (test)
  - [ ] `break`, `continue`, `return`, `exit`
  - [ ] `eval`, `exec`, `export`, `readonly`
  - [ ] `set`, `shift`, `unset`
  - [ ] `trap`, `wait`, `kill`
  - [ ] `cd`, `pwd`, `umask`
  - [ ] `read`, `echo`, `printf`
  - [ ] `test` / `[` -- file tests, string tests, integer comparisons
  - [ ] `getopts` -- option parsing
  - [ ] `type` -- command type identification
  - [ ] `alias` / `unalias` -- command aliases
  - [ ] `command` -- execute command bypassing functions/aliases
  - [ ] `hash` -- command path cache
- [x] `io.rs` -- I/O builtins
  - [ ] `read -r`, `read -p prompt`, `read -t timeout`, `read -n count`
  - [ ] `mapfile` / `readarray` -- read lines into array
  - [ ] `echo -n`, `echo -e` (escape sequences)
  - [ ] `printf` with full format string support

**Key files**:
- CREATE: `userland/vsh/src/builtins/mod.rs`
- CREATE: `userland/vsh/src/builtins/posix.rs`
- CREATE: `userland/vsh/src/builtins/io.rs`

**Verification**: `cd /tmp && pwd` -> `/tmp`; `test -f /etc/hosts && echo yes`.

---

### Sprint V-10: Bash-Specific Builtins (~2,500 lines)

**Goal**: 25+ bash-specific builtins including declare, history, enable.
**Depends on**: V-9
**Blocked by**: V-9

- [x] `bash.rs` -- bash extension builtins
  - [ ] `source` (alias for `.`)
  - [ ] `let` -- arithmetic evaluation
  - [ ] `[[ ]]` -- extended test (regex, pattern matching)
  - [ ] `enable` / `disable` -- builtin enable/disable
  - [ ] `builtin` -- force builtin execution
  - [ ] `caller` -- call stack info
  - [ ] `complete` / `compgen` / `compopt` -- programmable completion
  - [ ] `dirs` / `pushd` / `popd` -- directory stack
  - [ ] `disown` -- remove job from table
  - [ ] `help` -- builtin help text
  - [ ] `logout` -- exit login shell
  - [ ] `shopt` -- shell options (32+ options)
  - [ ] `suspend` -- suspend shell (SIGTSTP)
  - [ ] `times` -- user/system time
  - [ ] `ulimit` -- resource limits
  - [ ] `coproc` -- coprocess management
- [x] `declare.rs` -- declare/typeset
  - [ ] `declare -i` (integer), `-r` (readonly), `-x` (export)
  - [ ] `declare -a` (array), `-A` (assoc array)
  - [ ] `declare -n` (nameref)
  - [ ] `declare -p` (print declaration)
  - [ ] `declare -f` (print function), `-F` (function names)
  - [ ] `local` (function scope)
- [x] `history.rs` -- command history
  - [ ] History list management (HISTSIZE, HISTFILESIZE)
  - [ ] `history` -- display history
  - [ ] `history -c` (clear), `-d N` (delete entry)
  - [ ] `history -r/-w/-a/-n` (read/write/append/load)
  - [ ] `!N`, `!!`, `!string` -- history expansion
  - [ ] `fc` -- fix command (edit and re-execute)
  - [ ] `~/.vsh_history` file persistence

**Key files**:
- CREATE: `userland/vsh/src/builtins/bash.rs`
- CREATE: `userland/vsh/src/builtins/declare.rs`
- CREATE: `userland/vsh/src/builtins/history.rs`

**Verification**: `declare -A m; m[k]=v; declare -p m`; `history` shows commands; `shopt -s globstar`.

---

### Sprint V-11: Job Control (~1,500 lines)

**Goal**: Full job control with fg/bg/jobs, SIGTSTP/SIGCONT, process groups.
**Depends on**: P-7 (kernel signals), V-5 (execution)
**Blocked by**: P-7, V-5

- [x] `mod.rs` -- job table, job states (Running, Stopped, Done)
- [x] `control.rs` -- job control operations
  - [ ] `jobs` -- list jobs with status
  - [ ] `fg %N` -- bring job to foreground
  - [ ] `bg %N` -- resume job in background
  - [ ] `%N` -- job spec syntax (%1, %-, %+, %string)
  - [ ] `&` -- background execution
  - [ ] `disown` -- remove job from table
  - [ ] `wait` -- wait for job/PID
- [x] `notify.rs` -- async job status notification
  - [ ] SIGCHLD handler for child exit/stop detection
  - [ ] "Done" / "Stopped" notifications before prompt
  - [ ] `set -b` (immediate notification)
- [x] `waitpid.rs` -- waitpid wrapper with WUNTRACED/WCONTINUED
- [x] Process group management
  - [ ] `setpgid()` on fork for new pipeline group
  - [ ] `tcsetpgrp()` for foreground group
  - [ ] SIGTSTP/SIGCONT delivery to process groups

**Key files**:
- CREATE: `userland/vsh/src/jobs/mod.rs`
- CREATE: `userland/vsh/src/jobs/control.rs`
- CREATE: `userland/vsh/src/jobs/notify.rs`
- CREATE: `userland/vsh/src/jobs/waitpid.rs`

**Verification**: Ctrl+Z stops job; `bg` resumes; `fg` foregrounds; `jobs` lists.

---

### Sprint V-12: Line Editor (~2,500 lines)

**Goal**: Built-in readline with emacs/vi modes, completion, history search.
**Depends on**: V-1 (input), P-8 (PTY/terminal)
**Blocked by**: V-1, P-8

- [x] `mod.rs` -- line editor core, key binding dispatch
- [x] `emacs.rs` -- emacs editing mode (default)
  - [ ] Cursor movement: Ctrl+A/E (home/end), Ctrl+F/B (char), Alt+F/B (word)
  - [ ] Editing: Ctrl+D (delete), Ctrl+H (backspace), Ctrl+K (kill to end), Ctrl+U (kill to start)
  - [ ] Ctrl+W (kill word back), Alt+D (kill word forward)
  - [ ] Ctrl+Y (yank), Alt+Y (yank-pop)
  - [ ] Ctrl+T (transpose chars), Alt+T (transpose words)
  - [ ] Ctrl+L (clear screen)
  - [ ] Kill ring with rotation
- [x] `vi.rs` -- vi editing mode
  - [ ] Insert mode: character insertion
  - [ ] Normal mode: h/l (char), w/b/e (word), 0/$ (line), f/F (find)
  - [ ] Delete: x, dw, dd, D, d0, d$
  - [ ] Change: cw, cc, C, c0, c$
  - [ ] Yank/Put: yw, yy, p, P
  - [ ] Undo: u
  - [ ] Search: /, ?
- [x] `complete.rs` -- tab completion
  - [ ] Filename completion (default)
  - [ ] Command completion (from PATH)
  - [ ] Variable completion ($VAR<tab>)
  - [ ] Programmable completion (`complete -F func cmd`)
  - [ ] Menu-complete (cycle through completions)
  - [ ] Display candidates in columns
- [x] `history.rs` -- history search
  - [ ] Up/Down arrow history navigation
  - [ ] Ctrl+R incremental reverse search
  - [ ] Ctrl+S incremental forward search
  - [ ] Alt+. (insert last argument)
  - [ ] History substring search

**Key files**:
- CREATE: `userland/vsh/src/readline/mod.rs`
- CREATE: `userland/vsh/src/readline/emacs.rs`
- CREATE: `userland/vsh/src/readline/vi.rs`
- CREATE: `userland/vsh/src/readline/complete.rs`
- CREATE: `userland/vsh/src/readline/history.rs`

**Verification**: Ctrl+A moves to start; Tab completes filenames; Ctrl+R searches history.

---

### Sprint V-13: Prompt and Terminal (~800 lines)

**Goal**: PS1-PS4 prompt rendering, SIGWINCH handling, color support.
**Depends on**: V-12 (line editor)
**Blocked by**: V-12

- [x] `prompt.rs` -- prompt rendering
  - [ ] PS1 expansion: `\u` (user), `\h` (host), `\w` (cwd), `\W` (basename)
  - [ ] `\$` (#/$), `\n` (newline), `\t` (time), `\d` (date)
  - [ ] `\[` / `\]` non-printing character markers
  - [ ] ANSI color codes in prompts
  - [ ] PS2 (continuation), PS3 (select), PS4 (xtrace prefix)
  - [ ] PROMPT_COMMAND execution before PS1
- [x] `terminal.rs` -- terminal management
  - [ ] SIGWINCH handler (update COLUMNS, LINES)
  - [ ] Terminal capability detection
  - [ ] Raw mode enter/exit (for line editor)
  - [ ] Cursor position query (DSR/CPR)
- [x] `color.rs` -- ANSI color utilities
  - [ ] 16 basic colors, 256 color, RGB
  - [ ] Bold, underline, reverse, reset
  - [ ] `LS_COLORS` support for file listing

**Key files**:
- CREATE: `userland/vsh/src/prompt.rs`
- CREATE: `userland/vsh/src/terminal.rs`
- CREATE: `userland/vsh/src/color.rs`

**Verification**: PS1 shows `user@host:dir$ `; window resize updates COLUMNS/LINES.

---

### Sprint V-14: Startup Files and Compatibility (~800 lines)

**Goal**: Startup file loading, shopt options, --posix mode.
**Depends on**: V-9, V-10 (builtins)
**Blocked by**: V-9, V-10

- [x] `startup.rs` -- startup file processing
  - [ ] Login shell: `/etc/profile`, `~/.vsh_profile`, `~/.profile`
  - [ ] Interactive non-login: `~/.vshrc`
  - [ ] Non-interactive: `$BASH_ENV` / `$ENV`
  - [ ] `--rcfile FILE` override
  - [ ] `--norc` / `--noprofile` skip options
- [x] `compat.rs` -- compatibility modes
  - [ ] `--posix` mode (strict POSIX compliance)
  - [ ] `POSIXLY_CORRECT` environment variable
  - [ ] Bash compatibility level (`compat31` through `compat44`)
- [x] `options.rs` -- shell option management
  - [ ] `shopt` options (32+): `autocd`, `cdspell`, `checkwinsize`, `cmdhist`,
    `dirspell`, `dotglob`, `extglob`, `failglob`, `globstar`, `histappend`,
    `histreedit`, `histverify`, `hostcomplete`, `interactive_comments`,
    `lastpipe`, `lithist`, `nocaseglob`, `nocasematch`, `nullglob`,
    `progcomp`, `promptvars`, `sourcepath`, `xpg_echo`
  - [ ] `set` options: `-e`, `-u`, `-x`, `-v`, `-f`, `-n`, `-o` (long form)

**Key files**:
- CREATE: `userland/vsh/src/startup.rs`
- CREATE: `userland/vsh/src/compat.rs`
- CREATE: `userland/vsh/src/options.rs`

**Verification**: `~/.vshrc` sourced on interactive start; `shopt -s dotglob` works.

---

## Wave 5: Integration, Testing, and Release (4 sprints)

### Sprint T-1: Bash Test Suite Adaptation (~3,000 lines)

**Goal**: Adapt upstream bash test suite for vsh; target 1000+ passing cases.
**Depends on**: V-1..V-14 (all vsh sprints)
**Blocked by**: V-1..V-14

- [x] Port bash `tests/` directory structure
- [x] Quoting tests (single, double, ANSI-C, backslash)
- [x] Expansion tests (brace, tilde, parameter, command, arithmetic, glob)
- [x] Redirection tests (input, output, append, heredoc, process sub)
- [x] Pipeline tests (simple, stderr, pipefail)
- [x] Control flow tests (if, while, for, case, select, functions)
- [x] Builtin tests (cd, test, read, printf, declare, history)
- [x] Job control tests (fg, bg, jobs, Ctrl+Z)
- [x] Array tests (indexed, associative)
- [x] Signal/trap tests
- [x] Startup file tests
- [x] Regression tests for known bash edge cases
- [x] Test runner script with pass/fail/skip counts
- [x] Target: 1000+ tests, 95%+ pass rate

**Key files**:
- CREATE: `tests/vsh/` (50 test scripts, ~3,000 lines total)
- CREATE: `tests/vsh/run_tests.sh` (test runner)
- CREATE: `tests/vsh/framework.sh` (test helpers)

**Verification**: `./tests/vsh/run_tests.sh` reports 1000+ pass, <5% fail.

---

### Sprint T-2: Rust Compiler Integration Tests (~1,000 lines)

**Goal**: Comprehensive tests for the cross-compiled and self-hosted Rust toolchain.
**Depends on**: C-6 (self-hosting verified)
**Blocked by**: C-6

- [x] `test_rustc_version.sh` -- `rustc --version` outputs expected string
- [x] `test_hello_world.sh` -- compile and run hello world (both Debug and Release)
- [x] `test_cargo_new.sh` -- `cargo new` + `cargo build` + run
- [x] `test_std_features.sh` -- exercise std::fs, std::net, std::thread, std::process
- [x] `test_self_compile.sh` -- Stage 0 compiles hello; Stage 1 compiles hello; outputs match
- [x] `test_cargo_test.sh` -- `cargo test` runs unit tests on VeridianOS
- [x] `test_macros.sh` -- derive macros, procedural macros
- [x] `test_ffi.sh` -- C FFI via libc crate
- [x] `test_async.sh` -- basic async/await (single-threaded executor)
- [x] `test_memory.sh` -- peak RSS during compilation stays under 8GB

**Key files**:
- CREATE: `tests/rust/` (10 test scripts, ~1,000 lines total)
- CREATE: `tests/rust/run_tests.sh` (test runner)

**Verification**: All 10 tests pass; rustc+cargo fully functional on VeridianOS.

---

### Sprint T-3: Documentation (~3,000 lines)

**Goal**: Comprehensive guides for both the Rust compiler port and the vsh shell.
**Depends on**: T-1, T-2
**Blocked by**: T-1, T-2

- [x] `docs/RUST-COMPILER-PORTING.md` -- complete porting guide
  - [x] Target specification details
  - [x] std::sys::veridian module structure
  - [x] LLVM cross-compilation steps
  - [x] Self-hosting verification process
  - [x] Known limitations and workarounds
- [x] `docs/VSH-SHELL-GUIDE.md` -- user guide
  - [x] Feature comparison with bash 5.3
  - [x] Built-in command reference
  - [x] Configuration and startup files
  - [x] Differences from bash (if any)
  - [x] Scripting examples
- [x] `docs/PHASE6.5-COMPLETION-SUMMARY.md` -- phase summary
  - [x] Architecture decisions and rationale
  - [x] Sprint completion timeline
  - [x] Line count and file count metrics
  - [x] Performance benchmarks
  - [x] Lessons learned

**Key files**:
- CREATE: `docs/RUST-COMPILER-PORTING.md` (~1,200 lines)
- CREATE: `docs/VSH-SHELL-GUIDE.md` (~1,200 lines)
- CREATE: `docs/PHASE6.5-COMPLETION-SUMMARY.md` (~600 lines)

**Verification**: Documentation is accurate, complete, and internally consistent.

---

### Sprint T-4: CI Integration, Version Bump, Release (~500 lines)

**Goal**: Update CI, bump version to v0.7.0, create release.
**Depends on**: T-1..T-3
**Blocked by**: T-1..T-3

- [x] Update `.github/workflows/ci.yml`
  - [x] Add vsh build job (cargo build --target x86_64-unknown-veridian -p vsh)
  - [x] Add vsh test job (basic smoke tests)
  - [x] Add Rust toolchain verification job (cross-compile test)
- [x] Version bump
  - [x] `Cargo.toml`: 0.6.4 -> 0.7.0
  - [x] `kernel/src/services/shell/commands.rs`: uname version
  - [x] `kernel/src/fs/mod.rs`: /etc/os-release version
- [x] Update project documents
  - [x] README.md: Phase 6.5 in roadmap, v0.7.0 release
  - [x] CHANGELOG.md: v0.7.0 entry
  - [x] CLAUDE.md: Phase 6.5 status, latest release
  - [x] CLAUDE.local.md: session summary
- [x] Release pipeline
  - [x] `git tag -a v0.7.0`
  - [x] `gh release create v0.7.0 --notes-file /tmp/VeridianOS/release-notes.md`
  - [x] Tri-arch build verification (zero errors, zero warnings)

**Key files**:
- MODIFY: `.github/workflows/ci.yml`
- MODIFY: `Cargo.toml`
- MODIFY: `kernel/src/services/shell/commands.rs`
- MODIFY: `kernel/src/fs/mod.rs`
- MODIFY: `README.md`
- MODIFY: `CHANGELOG.md`
- MODIFY: `CLAUDE.md`

**Verification**: CI passes; v0.7.0 tag created; GitHub release published.

---

## Items NOT in Scope (Phase 7)

| Item | Rationale |
|------|-----------|
| GPU driver (virtio-gpu DRM) | Phase 7 prerequisite: native Rust compiler for driver dev |
| Advanced Wayland (xdg-decoration, DnD) | Phase 7: build on Phase 6 compositor |
| Multimedia (audio mixer, codecs) | Phase 7: needs GPU + DMA infrastructure |
| Virtualization (KVM-style hypervisor) | Phase 7: independent of compiler/shell |
| Container runtime (OCI) | Phase 7: needs namespace isolation, cgroups |
| AArch64/RISC-V Rust target | Stretch: focus on x86_64 first |
| Package manager integration (vpkg for Rust) | Stretch: manual install first |
| Full AML interpreter | Only needed for power management (Phase 7) |

---

## Progress Tracking

| Sprint | Component | Analysis | Implementation | Testing | Complete |
|--------|-----------|----------|---------------|---------|----------|
| P-1 | Memory Scaling | DONE | DONE | DONE | 100% |
| P-2 | Dynamic Linker | DONE | DONE | DONE | 100% |
| P-3 | libc Networking | DONE | DONE | DONE | 100% |
| P-4 | libc Threading | DONE | DONE | DONE | 100% |
| P-5 | libc stdio/stdlib | DONE | DONE | DONE | 100% |
| P-6 | libc POSIX Gaps | DONE | DONE | DONE | 100% |
| P-7 | Kernel Signals | DONE | DONE | DONE | 100% |
| P-8 | Terminal/PTY | DONE | DONE | DONE | 100% |
| P-9 | Filesystem | DONE | DONE | DONE | 100% |
| P-10 | epoll | DONE | DONE | DONE | 100% |
| P-11 | CMake | DONE | DONE | DONE | 100% |
| P-12 | Prereq Tests | DONE | DONE | DONE | 100% |
| R-1 | Core Types | DONE | DONE | DONE | 100% |
| R-2 | fs/io | DONE | DONE | DONE | 100% |
| R-3 | process/thread | DONE | DONE | DONE | 100% |
| R-4 | net/time | DONE | DONE | DONE | 100% |
| R-5 | env/alloc/OS | DONE | DONE | DONE | 100% |
| R-6 | Target Reg | DONE | DONE | DONE | 100% |
| C-1 | LLVM Cross | DONE | DONE | DONE | 100% |
| C-2 | Rust std Build | DONE | DONE | DONE | 100% |
| C-3 | rustc Stage 0 | DONE | DONE | DONE | 100% |
| C-4 | cargo Build | DONE | DONE | DONE | 100% |
| C-5 | Rootfs Integ | DONE | DONE | DONE | 100% |
| C-6 | Self-Hosting | DONE | DONE | DONE | 100% |
| V-1 | Binary Skeleton | DONE | DONE | DONE | 100% |
| V-2 | Lexer | DONE | DONE | DONE | 100% |
| V-3 | Parser/AST | DONE | DONE | DONE | 100% |
| V-4 | Expansion | DONE | DONE | DONE | 100% |
| V-5 | Execution | DONE | DONE | DONE | 100% |
| V-6 | Variables | DONE | DONE | DONE | 100% |
| V-7 | Control Flow | DONE | DONE | DONE | 100% |
| V-8 | Scripts | DONE | DONE | DONE | 100% |
| V-9 | POSIX Builtins | DONE | DONE | DONE | 100% |
| V-10 | Bash Builtins | DONE | DONE | DONE | 100% |
| V-11 | Job Control | DONE | DONE | DONE | 100% |
| V-12 | Line Editor | DONE | DONE | DONE | 100% |
| V-13 | Prompt/Terminal | DONE | DONE | DONE | 100% |
| V-14 | Startup/Compat | DONE | DONE | DONE | 100% |
| T-1 | Bash Tests | DONE | DONE | DONE | 100% |
| T-2 | Rust Tests | DONE | DONE | DONE | 100% |
| T-3 | Documentation | DONE | DONE | DONE | 100% |
| T-4 | CI/Release | DONE | DONE | DONE | 100% |

## Estimated Line Counts

| Wave | Sprints | New Lines | Modified Lines | Total |
|------|---------|-----------|----------------|-------|
| Wave 0 | P-1..P-12 | ~13,500 | ~1,500 | ~15,000 |
| Wave 1 | R-1..R-6 | ~6,500 | ~200 | ~6,700 |
| Wave 2 | C-1..C-6 | ~2,800 | ~200 | ~3,000 |
| Wave 3 | V-1..V-8 | ~14,500 | ~300 | ~14,800 |
| Wave 4 | V-9..V-14 | ~10,100 | ~0 | ~10,100 |
| Wave 5 | T-1..T-4 | ~7,500 | ~400 | ~7,900 |
| **Total** | **42** | **~54,900** | **~2,600** | **~57,500** |

## Timeline

- **Wave 0** (weeks 1-8): P-1..P-12 -- kernel/libc prerequisites
- **Wave 1** (weeks 9-12): R-1..R-6 -- Rust std platform (parallel with Wave 3)
- **Wave 2** (weeks 13-18): C-1..C-6 -- LLVM + rustc cross-build (parallel with Wave 4)
- **Wave 3** (weeks 9-14): V-1..V-8 -- vsh core engine (parallel with Wave 1)
- **Wave 4** (weeks 15-18): V-9..V-14 -- vsh builtins/job control (parallel with Wave 2)
- **Wave 5** (weeks 19-21): T-1..T-4 -- testing, docs, release

---

**Previous Phase**: [Phase 6 - Advanced Features](PHASE6_TODO.md)
**Next Phase**: [Phase 7 - Production Readiness](PHASE7_TODO.md)

# Linux ABI review, 2026-10-07 (supplement)

When the kernel switched to Linux system call numbers (X1, ADR 0009), every handler that now
serves a Linux number was compared with Linux's semantics by three read-only reviewers (files and
directories; processes, threads, signals, time and scheduling; memory, sockets and event
descriptors). A Linux number promises Linux behaviour, so a mismatch is a bug for every program,
not only for code ported from Linux.

This file is the tracking list for the IDs it adds (N-182 to N-252). Findings that an earlier
audit already records point to that ID instead of getting a new one. **C** = confirmed by tracing
the code. Status changes are appended to the last column. Targets: **X1** = fixed with the
renumbering; **X2** = the Linux syscall completion program (`docs/compat/COMPATIBILITY-PLAN.md`);
**D**, **F** = v0.27 sprints (`to-dos/MASTER_TODO.md`).

## Fixed with the renumbering

| ID | Call | Finding | Status |
|---|---|---|---|
| N-182 | (dispatch) | The musl remap sent Linux numbers to the wrong calls: dup3 to pipe2, epoll_create1 to dup3, epoll_ctl to epoll_create, sigaltstack to setsid, getppid to getpgid, pipe2 to inotify_init1, truncate and ftruncate swapped, execveat/mlock2/copy_file_range to the timerfd calls; the kernel guessed the intended call from argument values. The kernel's own Linux table sent exit (60) to whole-process exit and lacked truncate | **fixed** X1: one table generated from the Linux table, no remap, no guessing |
| N-183 | mmap | musl passes fd in r8 and the offset in r9, but the native path unpacked `fd << 32 \| offset` from r8, so every musl file mapping read fd 0 | **fixed** X1: Linux layout for every caller; the native libc passes six arguments |
| N-184 | pipe2, dup3 | Flags decoded as 0x2000, a value no C library sends: O_CLOEXEC (0x80000) was EINVAL in dup3 and ignored in pipe2, O_NONBLOCK never set, other bits accepted | **fixed** X1: Linux flags, others EINVAL; dup3 errors are EBADF where Linux says so |
| N-185 | dup2, dup3, fcntl F_DUPFD | The fd table grew to whatever target the caller named (kernel memory exhaustion from one call); RLIMIT_NOFILE reported 256 against a real limit of 1024 | **fixed** X1: `MAX_FDS`, EBADF / EINVAL past it, the rlimit reports it |
| N-186 | socket, socketpair, accept4 | SOCK_NONBLOCK and SOCK_CLOEXEC were masked off and lost; accept4's flags were dropped; protocol was ignored; a bad domain was EINVAL | **fixed** X1: flags honoured, unknown bits EINVAL, protocol checked (EPROTONOSUPPORT), EAFNOSUPPORT, socketpair(AF_INET) EOPNOTSUPP |
| N-187 | ptrace | Any process could PEEK and POKE any pid; TRACEME did nothing; PEEK returned the word instead of storing it at `*data` | **fixed** X1: tracer relationship (TRACEME, ATTACH with same uid or root, DETACH, cleared when the tracer exits); PEEK/POKE only on a traced process (ESRCH otherwise); PEEK stores at `*data`. GETREGS/SETREGS/CONT/SINGLESTEP stay ENOSYS until tracees can be stopped (D3) |
| N-188 | rt_sigreturn | The native libc's signal restorer issued the native number 123 by hand; the kernel trampoline likewise | **fixed** X1: both take the number from the generated table |
| N-189 | fsync, flock | Shared one native number, told apart by the second argument | **fixed** X1: Linux 74 and 73 |

## Files and directories

| ID | Sev | Call | Finding | Fix | Target |
|---|---|---|---|---|---|
| N-190 | High C | pread64, pwrite64 | The file's access mode is not checked (pwrite works on an O_RDONLY fd); pipes and sockets are not refused (ESPIPE); the offset is unsigned | check `file.flags`, ESPIPE for streams, EINVAL for a negative offset | v0.27; **fixed**: Linux's order (negative offset EINVAL, EBADF, ESPIPE, then the access mode; EISDIR for a directory read); streams are a `VfsNode::is_stream` property (pipes, sockets, terminals, eventfd/signalfd/timerfd/epoll), which lseek now honours too; pipes report `S_IFIFO` (they were character devices) |
| N-191 | High C | ftruncate, truncate | ftruncate works through any fd including O_RDONLY; truncate checks no permission at all; lengths are unsigned; errors become EINVAL | require write access / `require_open_access`, EISDIR, EINVAL for negative | v0.27; **fixed**: ftruncate is EINVAL unless the fd is a regular file open for writing; truncate needs write permission (EACCES), EISDIR for a directory, EINVAL for other types; negative lengths EINVAL; filesystem errors keep their errno; handlers renamed `sys_ftruncate`/`sys_truncate` after their calls |
| N-192 | High C | mkdir | No write+search check on the parent (mkdirat has one): any user creates directories anywhere | `require_dir_write` | v0.27; **fixed**: mkdir and mkdirat share one path: an existing name is EEXIST first (as Linux; `mkdir -p` relies on it), then write and search permission on the parent |
| N-193 | High C | access, faccessat* | Non-root callers are checked against the "other" bits only | `Permissions::can_*` with owner and group | D; **fixed**: owner/group/other with all of the process's groups, real IDs (effective with AT_EACCESS), AT_SYMLINK_NOFOLLOW, EINVAL for unknown bits; root needs an x bit to execute |
| N-194 | Medium C | select | Ignores the timeout and exceptfds, never blocks, reports every open fd ready | build on the poll machinery | X2 (events) |
| N-195 | Medium C | unlink, rmdir, unlinkat | A path without `/` is EINVAL; no file/directory type checks; AT_REMOVEDIR ignored | resolve through `resolve_at_path(AT_FDCWD)`, EISDIR/ENOTDIR, honour the flag | X2 (files) |
| N-196 | Medium C | newfstatat | AT_SYMLINK_NOFOLLOW and AT_EMPTY_PATH ignored (symlinks always followed) | honour both, EINVAL for others | X2 (files) |
| N-197 | Medium C | read, write, lseek | Wrong access mode is EACCES (Linux EBADF); write to an unopened fd is EINVAL; lseek succeeds on pipes and sockets (no ESPIPE) | EBADF, ESPIPE (now a `SyscallError`) | v0.27; **fixed**: EBADF for the wrong access mode and for writing an unopened fd, EISDIR for reading a directory, ESPIPE from lseek on streams (with N-190); timerfd, signalfd and epoll fds are read-write and the console fds 0-2 one read-write open, as on Linux |
| N-198 | Medium C | ioctl | Terminal ioctls decided by fd number (0-2 always a tty, >2 never); unknown requests EINVAL instead of ENOTTY; FIONBIO/FIONREAD/FIOCLEX missing | decide by node type | X2 (files) |
| N-199 | Medium C | fcntl | Record locks (F_GETLK/SETLK/SETLKW, OFD) EINVAL; bad fd EINVAL; F_SETFL ignores O_APPEND | locks, EBADF, O_APPEND | X2 (files) |
| N-200 | Medium C | readv, writev | One read/write per segment: can block between segments, and writev to a pipe is not atomic | gather into one transfer | X2 (files) |
| N-201 | Medium C | mknod | Always EPERM, so `mkfifo` fails | FIFOs and regular files for any user | X2 (files) |
| N-202 | Medium C | link, linkat, symlink, chown, chmod | Errors collapse to EINVAL (EEXIST lost); linkat follows symlinks without AT_SYMLINK_FOLLOW; a no-op chown by the owner is refused; setuid/setgid bits dropped by chmod; EACCES where Linux says EPERM | `map_kernel_error`, flags, owner rules | X2 (files) |
| N-203 | Medium C | path arguments | Empty path resolves to the cwd (ENOENT), over-long paths truncated (ENAMETOOLONG), non-UTF-8 names EINVAL, trailing `/` on a file ignored (ENOTDIR) | fix in the shared path reader | X2 (files) |
| N-204 | Medium C | *at calls | Bad dirfd EINVAL (EBADF), non-directory dirfd accepted (ENOTDIR), dirfd compared as 64-bit, dirfd path stored as opened (re-resolved against a later cwd) | canonical path at open, `dirfd as i32` | X2 (files); **partly fixed**: EBADF for a bad dirfd, ENOTDIR for a non-directory, dirfd compared as an int, and an fd records its canonical path at open |
| N-205 | Medium C | stat family | st_dev always 1, st_nlink 1, st_rdev 0 (libdrm/libinput identify devices by it), ctime is creation time; lstat errors all ENOENT and a fake socket for missing Wayland paths | dev/rdev/nlink in `Metadata`; real errors | X2 (files) |
| N-206 | Low C | getdents64, poll, open | getdents64 on a file EIO (ENOTDIR); poll caps nfds at 256 and drops RDNORM/WRNORM; O_PATH and O_TMPFILE ignored; NoSpace mapped to ENOMEM | per finding | X2 (files) |
| N-207 | Low C | flock | Blocking locks return EAGAIN instead of waiting | wait queue | D (blocking) |
| N-208 | Low C | statfs, fstatfs, fallocate, statx | ENOSYS (statx is fine: callers fall back) | real `struct statfs`; fallocate mode 0 | X2 (files) |

## Processes, threads, signals, time

| ID | Sev | Call | Finding | Fix | Target |
|---|---|---|---|---|---|
| N-209 | High C | rt_sigaction, kill, tkill, tgkill | Signals 32-64 are EINVAL; musl's `pthread_cancel` uses 33 | 64-entry tables (the sigset is already 64-bit) | v0.27; **fixed**: signals 1-64 in sigaction, kill, tkill, tgkill and delivery (`NSIG`); real-time signals (32-64) queue per process and per thread -- each send delivered once, EAGAIN beyond 1024 queued -- while standard ones merge; pending signals survive exec except ignored ones (they were all dropped); kill/tkill read their int argument's low 32 bits; the native libc has `_NSIG` 65 and `SIGRTMIN`/`SIGRTMAX` |
| N-210 | High C | clone | Only the CLONE_THREAD form is accepted; fork/vfork-style clone (musl `posix_spawn`, so `system`/`popen`) is EINVAL; newsp 0 is EFAULT | route non-thread clones to fork/vfork | v0.27; **fixed**: clone without CLONE_THREAD makes a process as fork (CLONE_VM: the parent's frames shared writable, its copy-on-write pages first made its own; CLONE_VFORK: the parent waits until the child execs or exits), with the exit signal from the low byte, a new stack and TLS, and the three TID flags (CHILD_SETTID written to the child's memory only); vfork (58) added, and the native libc's vfork is real (assembly; it was fork); CLONE_FILES/CLONE_SIGHAND across processes EINVAL. Runtime tests `vfork_and_process_clone`, `musl_posix_spawn`, `musl_posix_spawn_reports_enoent`, `musl_system` |
| N-211 | High C | execve | A NULL or empty envp inherits the parent's environment (`env -i` leaks it) | empty means empty | v0.27; **fixed**: a NULL or empty envp is an empty environment (runtime tests `execve_empty_envp_is_empty`, `execve_null_envp_is_empty`) |
| N-212 | Medium C | int arguments | pid_t/int arguments use the full 64-bit register (a zero-extended -1 is pid 4294967295) in wait4, kill, waitid, setpgid, getpgid, getsid, tkill, tgkill | `as i32` first | X2 (process) |
| N-213 | Medium C | wait4, waitid | Non-child pid EINVAL (ECHILD); rusage ignored; status written under WNOHANG with nothing to report; WNOWAIT EINVAL | per finding | X2 (process) |
| N-214 | Medium C | getcwd | Returns strlen, not strlen+1; too-small buffer EINVAL, not ERANGE | `len + 1`, ERANGE (now a `SyscallError`) | X2 (files) |
| N-215 | Medium C | chdir, setuid, setgid, setsid, arch_prctl | ENOTDIR reported as EINVAL; EPERM reported as EACCES | correct errnos | X2 (process) |
| N-216 | Medium C | setpgid | Non-child EACCES (ESRCH), negative pgid stored, no session checks | Linux rules | X2 (process) |
| N-217 | Medium C | gettid | The main thread's tid differs from the pid, and tids collide with other processes' pids | tid = pid for the first thread, one id allocator | D (threads) |
| N-218 | Medium C | clock_gettime | Only clocks 0 and 1; musl `clock()` (CLOCK_PROCESS_CPUTIME_ID) fails | aliases and CPU-time clocks | X2 (time) |
| N-219 | Medium C | clock_gettime, gettimeofday | CLOCK_REALTIME counts from boot, not 1970 | RTC epoch offset | X2 (time) |
| N-220 | Low C | gettimeofday | NULL tv is EFAULT; tz never written | allow NULL, zero tz | X2 (time) |
| N-221 | Medium C | sched_* | Stubs: set calls succeed without effect, get calls return 0, getaffinity writes nothing; the native affinity calls take (tid, ptr, size), not Linux's (pid, size, ptr) | policy table, affinity through the native handlers | D (scheduling syscalls) |
| N-222 | Medium C | sigaltstack | Succeeds without doing anything, never writes `old_ss` | per-thread alternate stack, SA_ONSTACK | D3 |
| N-223 | Medium C | sysinfo, getrusage | ENOSYS; musl `sysconf(_SC_PHYS_PAGES)` reads an uninitialised struct | implement | X2 (system) |
| N-224 | Medium C | getrlimit, setrlimit, prlimit64 | Most resources EINVAL; setrlimit has no effect; prlimit64 ignores the pid | per-process limits | X2 (process) |
| N-225 | Medium C | set_robust_list | Stored per process, never walked at thread death | per thread, walked in exit | D (threads) |
| N-226 | Low C | futex | WAKE_BITSET and the PI operations EINVAL | WAKE_BITSET; ENOSYS for PI | X2 (ipc) |
| N-227 | Low C | prctl | PR_SET_NAME does not store the name | store in `thread.name` | X2 (process) |
| N-228 | Low C | getpriority, setpriority | Native semantics (0-39 RealTime); kept on private numbers until nice values exist | nice-based translation | X2 (process) |

## Memory, sockets, event descriptors

| ID | Sev | Call | Finding | Fix | Target |
|---|---|---|---|---|---|
| N-229 | High C | memfd_create | Returns an eventfd registry id, not an installed fd | ENOSYS until a tmpfs-backed node exists | v0.27; **fixed** (implemented rather than ENOSYS): an installed read-write fd on an anonymous file in no directory (owned by the caller, mode 0777, or 0666 with MFD_NOEXEC_SEAL), Linux's flags and name limit, and file seals (F_ADD_SEALS/F_GET_SEALS; SHRINK, GROW, WRITE, FUTURE_WRITE, EXEC, SEAL enforced with EPERM); the native libc declares it in `<sys/mman.h>` and the seals in `<fcntl.h>`. Still open: a MAP_SHARED mapping of it is a copy (N-230) |
| N-230 | High C | mmap | A non-anonymous mapping with a closed fd silently becomes anonymous (EBADF); unaligned offset not EINVAL; MAP_SHARED file mappings are copies | per finding | X2 (memory) |
| N-231 | High C | bind, connect (AF_INET) | see **N-64** | | F1 |
| N-232 | High C | eventfd2, timerfd_create, signalfd4 | NONBLOCK flag stored in the object but not on the file, so read blocks | set `file.nonblock` | X2 (events) |
| N-233 | High C | signalfd4 | Mask read as the pointer value; sizemask taken as flags; the fd number used as the internal id; `deliver_signal` never called, so never readable | per finding | X2 (events) |
| N-234 | Medium C | epoll_ctl, epoll_pwait | Errors all EINVAL (EEXIST/ENOENT/EBADF/EPERM); registrations keyed by fd number and never removed on close; epoll_pwait ignores the sigmask | per finding | X2 (events) |
| N-235 | Medium C | recvfrom, sendto, accept, sendmsg, recvmsg | Sixth argument (addrlen) unread; flags ignored (MSG_PEEK consumes); never block on a blocking socket; INET errors EIO; msg_name unwritten; MSG_CMSG_CLOEXEC ignored | wait loop, flags, `copy_sockaddr_out` | F1 |
| N-236 | Medium C | getsockopt, getsockname, getpeername | INET getsockopt writes nothing but sets optlen; SO_PEERCRED missing (D-Bus EXTERNAL auth); addresses ignore `*addrlen`; getpeername never ENOTCONN | per finding | F1 |
| N-237 | Medium C | bind, connect (AF_UNIX) | Abstract namespace and autobind EINVAL (D-Bus, X11 use them) | implement | F1 |
| N-238 | Medium C | socket errors | Connection errnos all EINVAL/ENOENT | the new `SyscallError` variants (EADDRINUSE, ECONNREFUSED, ...) | F1 |
| N-239 | Medium C | timerfd | CLOCK_REALTIME timed on the monotonic clock; CLOCK_BOOTTIME EINVAL; bad itimerspec accepted; old_value is the spec, not the remaining time | per finding | X2 (events) |
| N-240 | Medium C | munmap, mprotect | Lengths over 256 MiB EINVAL; kernel range EACCES (EINVAL/ENOMEM); `munmap(0, len)` and `mprotect(addr, 0)` EINVAL (Linux succeeds) | `is_user_range` | X2 (memory) |
| N-241 | Low C | mmap, mprotect | W^X refuses PROT_WRITE\|PROT_EXEC; MAP_SHARED_VALIDATE rejected | keep W^X (document); accept MAP_SHARED_VALIDATE | X2 (memory) |
| N-242 | Low C | brk | Shrinking is ignored | lower the break | X2 (memory) |
| N-243 | Low C | madvise | Always succeeds without effect; MADV_DONTNEED does not zero | DONTNEED, EINVAL checks | X2 (memory) |
| N-244 | Low C | getrandom | Caps each call at 256 bytes; flags unchecked | full count, flag validation | X2 (system) |
| N-245 | Low C | epoll_create1, eventfd2, epoll | Unknown flags accepted; EPOLLET treated as level-triggered; epoll_wait on a non-epoll fd EBADF (EINVAL) | per finding | X2 (events) |
| N-246 | Low C | mremap | ENOMEM for every request, even a shrink | shrink in place, EINVAL checks | X2 (memory) |
| N-247 | Low C | inotify_* | ENOSYS (callers fall back to polling) | implement | X2 (files) |

## Credentials and paths (added at the user's request, implemented next)

| ID | Sev | Call | Finding | Fix | Target |
|---|---|---|---|---|---|
| N-248 | High C | seteuid, setegid, initgroups (native libc) | Reported success without changing anything: a program that drops privileges believes it did | setresuid/setresgid/setgroups and their getters in the kernel (real, effective, saved IDs and supplementary groups; permission checks on effective IDs) | v0.27 (pulled forward from X2); **fixed**: real/effective/saved IDs and up to 64 supplementary groups in the kernel (setresuid/setresgid, setreuid/setregid, setuid/setgid by Linux rules, getres*id, get/setgroups); file, search, exec, kill and ptrace checks use the effective IDs and groups; native libc and musl use them, and getgrouplist/initgroups read /etc/group |
| N-249 | Medium C | utimes, utimensat (native libc) | utimes did nothing | utimensat in the kernel | v0.27; **fixed**: utimensat (UTIME_NOW/UTIME_OMIT, AT_SYMLINK_NOFOLLOW, futimens form, owner/write rules) on ramfs, tmpfs and BlockFS; utimes/utimensat/futimens in the native libc |
| N-250 | Medium C | fchdir, chroot (native libc) | ENOSYS | fchdir; chroot with a per-process root applied by the path resolver | v0.27; **fixed**: fchdir; chroot with a per-process root applied by the path resolver (absolute paths, ".." and absolute symlinks stay inside); chdir resolves to the canonical directory and checks search permission |
| N-251 | Medium S | path lookups in kernel subsystems | Every VFS lookup used the current thread's root, working directory and credentials, so kernel code running during a system call (the audit log, the package manager) resolved paths in a chrooted caller's tree -- a chrooted process could have the audit log written to a file it owns -- and with the caller's search permissions and MAC identity (security review of the N-250 change) | Kernel lookups use the kernel's view; paths a program supplies go through an explicit caller view | v0.27; **fixed**: `PathContext` (cwd, root, credentials, MAC pid); `Vfs` methods resolve in the kernel's view, and `Vfs::as_caller()` gives a `CallerView` used by system calls, exec, the PATH search and the program loader; `CallerView` does not dereference to `Vfs`, so an operation it lacks is a compile error rather than a lookup outside the chroot; mount and umount targets are resolved in the caller's view and must be directories |
| N-252 | High S | ptrace POKETEXT/POKEDATA | The word was written into the frame recorded in the tracee's mapping, which after a fork is shared with other processes (copy-on-write, or read-only like code): a breakpoint set in a child also landed in its parent (found while implementing N-210) | Write into the tracee only | v0.27; **fixed**: `VirtualAddressSpace::write_bytes_private` copies a shared page first (MAP_SHARED pages stay shared); EIO for an unmapped address |

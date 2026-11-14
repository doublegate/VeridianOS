# ADR 0027: the Linux system calls still missing after group 3

- Status: proposed
- Date: 2026-11-11
- Tracking: v0.28.0 order of work, item 5 (`to-dos/MASTER_TODO.md`). One branch and pull
  request, a commit per family; each family adds its decisions here before this ADR is
  accepted.

## Context

After group 3 and ADR 0024, 108 of Linux 7.2's 385 x86_64 system calls are not routed to an
implementation (`docs/compat/LINUX-SYSCALL-COVERAGE.md`). Item 5 asks for each to be either
implemented with Linux's semantics or to answer what Linux built without the feature answers
(ENOSYS, EPERM or EOPNOTSUPP, as `kernel/sys_ni.c` or the feature's stub says), with the
`abi/kconfig.map` option left unset, and for every such choice to be recorded rather than made
silently. ADR 0024 also asks new compatibility code to keep Linux's semantics behind module
interfaces, so it can move to the user-space Linux runtime later, and every Linux privilege check
to need an object capability as well.

The reference for every behaviour below is the Linux 7.3 source (kernel.org tarball,
`ipc/`, `mm/`, `fs/`, `kernel/`), and LTP's tests of each call.

## Decision

### System V IPC (shmget, shmat, shmdt, shmctl, semget, semop, semtimedop, semctl, msgget, msgsnd, msgrcv, msgctl)

1. Implemented with Linux's semantics, each IPC namespace with its own objects and limits
   (ADR 0022 H1 made the namespace; it now holds them), in `kernel/src/syscall/sysvipc/`:
   identifiers with a sequence number as `ipc_ids` makes them, `ipcperms`, the `ipcget` order of
   checks, `/proc/sysvipc/{shm,sem,msg}`, and the `/proc/sys/kernel` limits `shmmax`, `shmall`,
   `shmmni`, `shm_rmid_forced`, `sem`, `msgmax`, `msgmnb` and `msgmni`, per namespace, with
   Linux's defaults. `CONFIG_SYSVIPC` is set.
2. One security model (ADR 0024): every System V and POSIX message-queue access also needs the
   caller's rights over the IPC namespace, split by intent (the maintainer's choice, 2026-11-11,
   over READ/WRITE alone, over CREATE without MODIFY, and over no check):
   - READ for any access: IPC_STAT and the other reads, opening a queue, waiting for a
     semaphore to reach zero, a read-only attachment;
   - WRITE to change an object's contents: sending and receiving a message (receiving takes it
     out), altering a semaphore, a writable attachment, registering for mq_notify;
   - CREATE to make an object (shmget, semget, msgget with a new key, mq_open with O_CREAT;
     EACCES without);
   - MODIFY to control one or the namespace: IPC_SET, IPC_RMID, SHM_LOCK and SHM_UNLOCK,
     mq_unlink, and writing the namespace's limits (EPERM without; EACCES for the limits, as
     sysctl_perm). The initial IPC namespace's limits are the whole system's, shared by every
     process not unshared from it, so writing them needs MODIFY on the `System` object as
     well, as every other sysctl does (ADR 0024, step 7); a new IPC namespace's root may tune
     its own namespace without the `System` object (the maintainer's choice, 2026-11-11, over
     MODIFY on the namespace alone and over the `System` object for every IPC namespace).
   A launcher can then let a program use an existing channel but not make new ones, or use
   one without deleting it or changing its owner. CAP_IPC_OWNER, CAP_SYS_ADMIN, CAP_IPC_LOCK and
   CAP_SYS_RESOURCE count over the namespace's user namespace, as `ns_capable(ns->user_ns,
   ...)`. ADR 0024's table gets the rows (its decision 5 says calls added later add theirs).
3. A segment's pages are a memfd's (`fs::shmem` frames), so an attachment is a MAP_SHARED mapping
   of them. Each piece of an attachment holds a hook on it (`mm::vas::MappingHook`, Linux's
   vm_ops open and close), so splits, a fork's copy, munmap, exit and exec keep `shm_nattch`,
   and such a mapping is never merged with its neighbour.
4. Differences from Linux, each recorded in `docs/KNOWN-LIMITATIONS.md`:
   - A segment's pages are allocated when it is made, not when first touched (memfd's pages
     are too), so shmget of more memory than is free fails with ENOMEM rather than later.
   - SHM_HUGETLB is EINVAL (CONFIG_HUGETLBFS is unset: no huge page size exists, as newseg
     answers for an unknown one). SHM_LOCK and SHM_UNLOCK only record the flag: no page is
     ever swapped out (CONFIG_SWAP is unset), so locking changes nothing else.
   - MSG_COPY is ENOSYS after its EINVAL checks, and there are no `*_next_id` sysctls, as on
     Linux without CONFIG_CHECKPOINT_RESTORE.
   - The semaphore undo list belongs to the process, shared by its threads and by a
     CLONE_SYSVSEM clone, rather than to each thread: a thread's unshare(CLONE_SYSVSEM) or its
     setns into another IPC namespace applies and leaves the whole process's list.
   - A semop waiter tries again after every change to the namespace's semaphores rather than
     being woken in Linux's queue order; which of several waiters goes first is not specified
     by POSIX either.

### POSIX message queues (mq_open, mq_unlink, mq_timedsend, mq_timedreceive, mq_notify, mq_getsetattr)

5. Implemented with Linux's semantics (`ipc/mqueue.c`), each IPC namespace with its own queues
   and `/proc/sys/fs/mqueue` limits (`queues_max`, `msg_max`, `msgsize_max`, `msg_default`,
   `msgsize_default`, Linux's defaults and ranges), in `kernel/src/syscall/mqueue.rs`: a queue
   is a file, so a descriptor of it reads Linux's status line, polls for a message or room, and
   holds the queue after mq_unlink until its last close; the queue's bytes count against
   RLIMIT_MSGQUEUE per real user as Linux counts them (EMFILE past it). Access needs the
   caller's rights over the IPC namespace as for System V (READ, and WRITE to change).
   `CONFIG_POSIX_MQUEUE` is set.
6. Differences from Linux:
   - mq_notify's SIGEV_THREAD needs a netlink socket (the C library makes one and the kernel
     writes the notification to it), and there are no netlink sockets yet (they come with the
     network family's netlink route work): musl's mq_notify with SIGEV_THREAD fails where it
     makes the socket (EAFNOSUPPORT), and the raw call answers what Linux answers for a
     descriptor that is no netlink socket (ENOTSOCK, or ECONNREFUSED for another socket).
     SIGEV_NONE and SIGEV_SIGNAL are Linux's.
   - The mqueue filesystem is mountable (`mount -t mqueue none /dev/mqueue`, the maintainer's
     choice for item 5): its sticky, world-writable root lists the mounting namespace's queues,
     reading one gives its status line, unlink is mq_unlink and open(O_CREAT) makes a queue
     with the default attributes; mkdir there is EPERM, as on Linux.

### futex2 (futex_waitv, futex_wake, futex_wait, futex_requeue)

7. Implemented with Linux's semantics (`kernel/futex/syscalls.c`) on the futex table the futex(2)
   operations use (`kernel/src/syscall/futex2.rs`). Only 32-bit futexes exist, as on Linux
   (other sizes EINVAL). FUTEX2_PRIVATE is a hint, as there: the key comes from the mapping.
   FUTEX2_NUMA doubles the word, its second half the node; with one node, FUTEX_NO_NODE (-1)
   becomes 0 and any node but 0 is EINVAL, as `get_futex_key` answers on a one-node system.
   futex_waitv queues one table entry per futex sharing the waiter's wake flag and returns the
   index of the entry a waker took; an entry of a waiter another entry already woke is removed
   without counting as a second task woken.
8. The VeridianOS-private calls 1225 and 1226, whose kernel names were FutexWait and
   FutexWake, are renamed NativeFutexWait and NativeFutexWake (`SYS_NATIVE_FUTEX_WAIT`,
   `SYS_NATIVE_FUTEX_WAKE`), as Linux's futex_wait and futex_wake take the names; their numbers
   are unchanged.

### Memory (mlock, mlock2, munlock, mlockall, munlockall, mseal, msync, mincore, remap_file_pages, membarrier, process_madvise, process_mrelease, memfd_secret, pkey_*)

9. Implemented with Linux 7.3's argument checks in Linux's order (`kernel/src/syscall/memctl.rs`,
   `membarrier.rs`, `memory.rs`). A mapping records its mlock state (VM_LOCKED, VM_LOCKONFAULT),
   whether it is sealed and its protection key; the address space records mlockall(MCL_FUTURE)'s
   default, the membarrier registrations and the allocated keys. A fork child keeps seals, keys
   and registrations but not locks or MCL_FUTURE; exec clears all of it. Pages here are never
   paged out, so a lock changes what is counted (VmLck in `/proc/<pid>/status`, Locked and the
   `lo`/`lf` VmFlags in `/proc/self/smaps`, RLIMIT_MEMLOCK), what MAP_LOCKED, MCL_FUTURE
   (later mmap and brk), the growth of a locked mapping by mremap (EAGAIN past the limit each) and
   msync's MS_INVALIDATE (EBUSY) do.
10. Locking past RLIMIT_MEMLOCK, or at all while it is 0, needs CAP_IPC_LOCK and, as ADR 0024's
    RLIMIT_NPROC override, the `System` object with MODIFY (the maintainer's choice, 2026-11-12,
    over CAP_IPC_LOCK alone, which would let a sandboxed root pin unbounded memory, and over no
    exemption at all). ADR 0024's table gains the row.
11. mseal refuses, with EPERM, munmap, a MAP_FIXED mapping over a sealed range, mremap of or onto
    one, mprotect and pkey_mprotect, and the madvise advice that discards contents on sealed
    anonymous memory that is not writable, as Linux; a range with a hole is ENOMEM with nothing
    sealed.
12. msync: a MAP_SHARED mapping of a regular file holds a copy of it until the v0.29 page cache,
    so MS_SYNC and MS_ASYNC both write that copy back within the file's size (the maintainer's
    choice); a memfd's or tmpfs file's mapping maps its pages and needs nothing. mincore reports
    each page that has a frame, which every page of an anonymous mapping has (memory is not
    lazy yet: LTP's mincore03 expects untouched pages to be absent).
13. membarrier: every expedited command interrupts each other online CPU and waits until it has
    taken the interrupt, whose handler runs a full barrier, and whose return serialises the
    core (iret, eret; fence.i on RISC-V), which also serves SYNC_CORE. The wait never ends early,
    as on Linux: a CPU slow to answer is sent the interrupt again and the caller yields its CPU
    meanwhile, warning once (the maintainer's choice over a time limit with an error, which
    Linux never returns); only a process being killed stops waiting. MEMBARRIER_CMD_GLOBAL is
    the same barrier. QUERY reports 639, as Linux built without CONFIG_RSEQ: rseq(2) is not
    implemented, so the RSEQ commands are EINVAL.
14. process_madvise on another process needs PTRACE_MODE_READ access and CAP_SYS_NICE, with ADR
    0024's ptrace/scheduling row (MODIFY over the target; the maintainer's choice, 2026-11-12),
    and takes only the advice Linux allows remotely (COLD, PAGEOUT, WILLNEED), which are hints
    here. process_mrelease accepts a dying process and returns 0: its memory goes as it exits.
15. remap_file_pages is Linux's emulation, a MAP_FIXED remapping of the same shared file at
    another offset; a System V attachment is refused (EINVAL) rather than losing its count.
16. Without hardware or support, as Linux built without the option: the NUMA calls (mbind,
    set_mempolicy, get_mempolicy, migrate_pages, move_pages, set_mempolicy_home_node),
    userfaultfd and map_shadow_stack are ENOSYS.
17. memfd_secret is ENOSYS for now, as Linux answers where the architecture cannot take pages
    out of its direct map (`can_set_direct_map`). Nothing here can split the physical map yet
    (x86_64 maps it with 1 GiB pages in tables every address space shares); a following pull
    request builds that on all three architectures, guards every kernel path that reaches user
    frames through the physical map, and implements memfd_secret on it (the maintainer's
    choice, 2026-11-12, over doing it inside this group and over leaving it for v0.29).
18. Protection keys answer as Linux does without the hardware (x86 without OSPKE, arm64 without
    POE), on every architecture: pkey_alloc checks its flags and access bits (EINVAL) and has no
    key to give (ENOSPC), pkey_free is EINVAL, pkey_mprotect makes mprotect's checks and takes
    only keys -1 and 0. LTP's mseal01, which calls pkey_mprotect, runs, and pkey01 ends in a
    clean TCONF. Real keys (x86_64 PKU: key bits in user PTEs, CR4.PKE, PKRU per thread in the
    XSAVE state; AArch64 POE) follow in their own pull request (the maintainer's choice,
    2026-11-12).

### Files and scheduling (ioprio_set, ioprio_get, file_getattr, file_setattr, cachestat)

19. ioprio_set and ioprio_get have Linux's semantics (`block/ioprio.c`,
    `kernel/src/syscall/ioprio.rs`). Each thread has an I/O context (`IoContext`), shared by
    CLONE_IO and copied by any other clone or fork. Its value is 0 (class NONE) until set; such a
    thread's I/O runs at the priority its scheduling gives (SCHED_IDLE IDLE, real-time and deadline
    RT, otherwise BE at level (nice + 20) / 5). The RT class needs CAP_SYS_ADMIN or CAP_SYS_NICE;
    another thread's needs its owner's UID to be the caller's real or effective one, or
    CAP_SYS_NICE, and MODIFY over its process, as setpriority (ADR 0024).
20. The priority reaches the devices through an arbiter (`drivers::block_arbiter`, the
    maintainer's choice over asynchronous queues, which come with v0.29's completion path). The
    virtio-blk and NVMe drivers run one request at a time behind the device's lock, so callers
    wait here and each turn goes to the best priority waiting: RT before BE before IDLE, by level
    within a class, the earliest first between equals. A waiter passed over for a second goes
    next, whatever its class, and IDLE goes only when no other class waits.
21. Inode attribute flags (`kernel/src/fs/attr.rs`, `kernel/src/syscall/fileattr.rs`):
    - **The set:** Linux tmpfs's, the maintainer's choice: immutable, append-only, nodump and
      noatime, through FS_IOC_GETFLAGS/SETFLAGS, FS_IOC_FSGETXATTR/FSSETXATTR and
      file_getattr/file_setattr. Other flags, any xflag beyond the common ones, an extent size and
      a project ID are EOPNOTSUPP, as tmpfs answers; file_setattr's unknown xflags are EINVAL.
    - **Storage:** BlockFS keeps them in the on-disk inode's `flags` field, at ext4's i_flags
      offset and unused before (no format change: older images read 0). tmpfs, ramfs and memfd
      keep them in memory. ext4, read-only here, reports its i_flags and refuses a change (EROFS).
      Other filesystems have none (ENOTTY from the ioctls, EOPNOTSUPP from file_getattr).
    - **Setting:** the owner or CAP_FOWNER over the file, with WRITE on the mount namespace (as
      chmod); immutable and append-only need CAP_LINUX_IMMUTABLE. A new node takes nodump and
      noatime from its directory.
    - **Immutable, ext4's semantics on every filesystem** (the maintainer's choice over tmpfs's,
      which checks only at open):
      - No write by anyone, through a descriptor opened before the flag was set too: write and
        its variants, truncate, fallocate, copy_file_range and splice into it.
      - No open for writing, no entry made or removed in an immutable directory, no unlink,
        rename or link of the file itself, no chmod, chown or setting of its times, no xattr
        change.
      - Shared mappings: setting the flag first write-protects every writable shared mapping of
        the file in every process and writes their contents back. A later write through one is
        SIGBUS (BUS_ADRERR), as ext4's page_mkwrite refuses it, and so is a write through one
        made or made writable later (mmap, mprotect, mremap).
    - **Append-only:** opens for writing only with O_APPEND and never with O_TRUNC; no truncate,
      no fallocate except FALLOC_FL_KEEP_SIZE, no clearing O_APPEND with fcntl, no MAP_SHARED mapping
      through a descriptor open for writing (EACCES, as do_mmap), no unlink, rename or link,
      nothing removed from an append-only directory.
    - **nodump** is only recorded.
22. cachestat (`mm/filemap.c`) reports the real caches, the maintainer's choice:
    - **tmpfs and memfd:** every page of the range the file holds (they are always resident, and
      none is dirty).
    - **ramfs:** every page within the file's size.
    - **BlockFS:** a page is cached when its block is in the block cache, dirty as the cache
      says, or in a mapping's page cache. A hole is not cached.
    - **Always 0:** nr_writeback (I/O is synchronous) and nr_evicted/nr_recently_evicted (no
      eviction history is kept).
    - **Permission:** as Linux's can_do_cachestat (open for writing, the owner, or write
      permission; EPERM otherwise).

### fchroot and failfs (Linux 7.3)

23. fchroot (472, new in Linux 7.3; the vendored tables move to v7.3 with it, `abi/linux/VERSION`)
    with Linux's semantics (`fs/open.c`, `fs/failfs.c`): `fchroot(fd, 0)` is chroot to the
    directory `fd` names, with chroot's checks (search permission, CAP_SYS_CHROOT, and MODIFY on
    the mount namespace by ADR 0024's chroot row). `FD_FAILFS_ROOT` (-10004), for fchroot and
    fchdir, is failfs: from its root no absolute path or absolute symlink resolves, and from it
    as working directory no relative one (EOPNOTSUPP), while lookups anchored at real
    directories go on and `..` stops at the top of the tree. An unprivileged caller may enter it
    with no_new_privs, unshared filesystem state and no earlier chroot (EPERM, EINVAL, EPERM),
    as on Linux; entering it from the top of the tree takes authority away, so it needs no
    object capability, but a privileged caller entering it from a chroot leaves the jail (its
    relative lookups walk the whole tree) and needs MODIFY on the mount namespace, as chroot
    does (EPERM; Linux asks only for CAP_SYS_CHROOT there). Kept in the
    path-string model as a sentinel root no user path can hold (`fs::FAILFS_ROOT`). Not yet:
    `/proc/self/root` and `/proc/self/cwd`, which do not exist.

## Consequences

- LTP's ipc tests (shmget*, shmat*, shmctl*, semget*, semop*, semctl*, msgget*, msgsnd*,
  msgrcv*, msgctl*) and programs using System V IPC (PostgreSQL's shared buffers, X11's MIT-SHM)
  can run; the expected lists are refreshed from full runs.
- `/proc/sys/kernel` and `/proc/sysvipc` gain files per IPC namespace.
- The System V code is Linux compatibility code: it lives with the system calls and moves to the
  user-space Linux runtime with them (ADR 0024).

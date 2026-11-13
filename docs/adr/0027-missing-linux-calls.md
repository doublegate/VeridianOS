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

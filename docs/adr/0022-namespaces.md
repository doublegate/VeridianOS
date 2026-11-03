# ADR 0022: Linux namespaces

- Status: accepted
- Date: 2026-11-02
- Tracking: v0.28.0 order of work, item 3 (syscall groups), step H. The user chose "all namespace
  types" for v0.28 on 2026-10-18 (MASTER_TODO, decisions carried from v0.27).

## Context

Linux programs create and join namespaces through clone(CLONE_NEW*), unshare(2) and setns(2),
and inspect them through `/proc/<pid>/ns/*`. Container runtimes (runc, crun, podman, bubblewrap),
systemd's sandboxing, `unshare(1)`, `nsenter(1)`, and much of LTP (clone301/304, setns, unshare,
the `*_ns` tests, every test that calls `tst_reinit` in a new PID namespace) depend on them.
VeridianOS has none: every table is global (survey of 2026-11-02, kept with this ADR's step
notes), and `kernel/src/virt/namespace.rs` is an unwired model that no system call reaches.

Linux 7.3 has eight types: UTS, IPC, mount, PID, network, user, cgroup and time. Each task holds
them through an `nsproxy` (all but user, which lives in the credentials); `pid_for_children` and
`time_for_children` take effect only for children. Each namespace has an inode number on nsfs
(the eight initial ones fixed at 0xEFFFFFF8 to 0xEFFFFFFF, new ones from 0xF0000000) and, since
6.18, a 64-bit id (1 to 8 for the initial ones).

## Decision

1. **One namespace set per thread, as Linux's nsproxy** (`process::ns::Namespaces`, an `Arc` on
   `Thread`): uts, ipc, mnt, pid_for_children, net, cgroup, time, time_for_children. A thread
   created with CLONE_THREAD shares its creator's set; unshare and setns give the calling thread a
   new set, as on Linux. The user namespace belongs to the credentials (`Credentials.user_ns`),
   which are per process here; unshare(CLONE_NEWUSER) needs a single-threaded caller, as on
   Linux, so per-process credentials lose nothing.

2. **Each namespace is a kernel object with Linux's identity**: its type, its owning user
   namespace, its parent (PID and user), an inode number (the initial ones Linux's fixed values,
   new ones from 0xF0000000) and a 64-bit id (the initial ones 1 to 8, new ones counting up from
   9), so `ls -l /proc/self/ns`, `stat` and `NS_GET_ID` agree with Linux's format.

3. **nsfs and `/proc/<pid>/ns`**: `/proc/<pid>/ns/{cgroup,ipc,mnt,net,pid,pid_for_children,time,
   time_for_children,user,uts}` are magic links (readlink `uts:[4026531838]`) that open an nsfs
   file for the namespace; fstat gives its inode number on nsfs (NSFS_MAGIC). The nsfs ioctls
   NS_GET_NSTYPE, NS_GET_USERNS, NS_GET_PARENT, NS_GET_OWNER_UID, NS_GET_ID and the
   NS_GET_*_PIDNS translations are implemented; NS_MNT_GET_* and listns/statns come later.
   Reading another process's links needs ptrace read access, as Linux.

4. **The system calls follow Linux 7.3's checks in order**: unshare (`ksys_unshare`'s flag
   implications and `check_unshare_flags`), setns (an nsfs file with a type or 0, or a pidfd with
   a set of CLONE_NEW* flags; `validate_nsset` then each type's install rules), and clone/clone3
   (`copy_namespaces`). Creating any type but user needs CAP_SYS_ADMIN in the caller's user
   namespace; a user namespace can be created without privilege, as on Linux, bounded by
   `/proc/sys/user/max_*_namespaces`.

5. **Every type does what Linux's does**, delivered in this order, one gated commit each:
   - **H1, the framework, UTS, IPC and cgroup.** The set, nsfs, `/proc/<pid>/ns`, unshare, setns,
     clone. UTS: hostname and domainname per namespace, sethostname and setdomainname,
     `/proc/sys/kernel/{hostname,domainname}`. IPC and cgroup namespaces are created, joined and
     reported, but hold nothing yet: there is no System V IPC, no message queue and no cgroup
     hierarchy (POSIX shared memory lives in `/dev/shm`, which follows the mount namespace, as on
     Linux).
   - **H2, time.** CLOCK_MONOTONIC and CLOCK_BOOTTIME become separate clocks (boot time counts
     no suspend, so they agree until suspend exists), and a time namespace holds an offset for
     each, set through `/proc/<pid>/timens_offsets` before its first process. Every read and every
     absolute deadline (clock_nanosleep, timerfd, POSIX timers, /proc/uptime, sysinfo) goes
     through the reader's offset.
   - **H3, mount.** Each mount namespace has its own `MountTable`; unshare copies the caller's,
     and mounts and unmounts then affect only that namespace. Every mount is private (no shared
     or slave propagation), recorded as a limitation. Linux mount(2), umount2 and pivot_root come
     with it, and `/proc/<pid>/mounts` and `mountinfo`. As on Linux, the copy's mounts get new
     ids from the pool the original's came from (one IDA), and a file's device number belongs to
     its filesystem, so it is the same in every namespace.
   - **H4, user.** uid_map, gid_map and setgroups (Linux's write rules), ids translated at every
     boundary (stat, chown, credentials calls, SO_PEERCRED, siginfo si_uid, `/proc/<pid>/status`),
     and `capable()` asking about the user namespace that owns the object (Linux's ns_capable)
     at each of the 45 call sites.
   - **H5, PID.** A process has an id at each level of its PID namespace, from its own up to the
     initial one; every interface that takes or returns a pid or tid translates through the
     caller's namespace (getpid, wait, kill, siginfo, signalfd, sched calls, ptrace, pidfd,
     SO_PEERCRED, futex and robust-list words, `/proc`). A namespace's first process is its init:
     orphans are reparented to it, and its exit kills the rest of the namespace. procfs shows
     the PID namespace it was mounted in.
   - **H6, network.** The network stack's global tables move into a `NetNs`; the existing stack
     becomes the initial namespace, and a new namespace starts with only its own loopback, as on
     Linux. Sockets belong to the namespace they were created in; moving devices between
     namespaces comes with veth, later.

6. **`kernel/src/virt/namespace.rs` stays an unwired model** for the kernel shell's container
   commands, which move onto the real namespaces with C6 (and the dead container hook in fork
   with them); no system call reaches it.

## Consequences

- Unmodified container tools can run once H1 to H6 are in, within the limitations below.
- The cost is concentrated in H4 (every privilege check names a namespace), H5 (every pid
  crossing the user boundary is translated) and H6 (the network stack is restructured); each is
  its own commit with its own tests, so a regression points at one of them.
- Limitations recorded in KNOWN-LIMITATIONS as each lands: IPC and cgroup namespaces with
  nothing in them; no mount propagation; no device moves between network namespaces; no
  listns/statns or NS_MNT_GET_*; the initial namespaces' fixed numbers are Linux's, so a program
  that compares them with a Linux host's sees the same values.
- Tests: host tests for each rule (flag checks, map writes, translation), runtime tests on all
  three architectures for each type, and LTP's namespace tests in the expected lists.

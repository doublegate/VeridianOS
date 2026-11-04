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
     or slave propagation), recorded as a limitation: MS_PRIVATE and MS_SLAVE are accepted (a
     slave of nothing is private), MS_UNBINDABLE is kept, and MS_SHARED is EINVAL rather than
     accepted and silently not propagated. Linux mount(2), umount2 and pivot_root come
     with it, and `/proc/<pid>/mounts` and `mountinfo`. As on Linux, the copy's mounts get new
     ids from the pool the original's came from (one IDA), and a file's device number belongs to
     its filesystem, so it is the same in every namespace. Mount flags are enforced where Linux
     checks them (mnt_want_write and the like), on the mount that serves the path a call
     resolved; for a call on a descriptor, the mount serving the path it was opened at, since
     the VFS does not keep the mount in the open file. pivot_root follows Linux's
     `path_pivot_root` (fs/namespace.c), checks in its order: the new root, then put_old, looked
     up as directories; EPERM without CAP_SYS_ADMIN; EBUSY when either is the caller's root
     mount; EINVAL when the root is not a mount point, the new root is not one, put_old is not
     under the new root or the new root is not under the root. The new root's mount takes the
     root's place and the old root goes to put_old; the namespace root can be pivoted, as with
     Linux's nullfs. Linux moves only the roots and working directories that were at the old
     root (chroot_fs_refs) and the rest go with their mounts; here they are paths, so each one
     in the namespace moves to where its mount went. `pivot_root(".", ".")` puts the old root
     on top of the new one where no path reaches it (the table keeps it as a parked tree, shown
     in mountinfo), until `umount2(".", MNT_DETACH)` takes it off.
   - **H4, user.** uid_map, gid_map and setgroups (Linux's write rules), ids translated at every
     boundary (stat, chown, credentials calls, SO_PEERCRED, siginfo si_uid, `/proc/<pid>/status`),
     and `capable()` asking about the user namespace that owns the object (Linux's ns_capable)
     at each of the 45 call sites. Three steps. H4a, the namespace and its creation (Linux 7.3's
     kernel/user_namespace.c): a `UserNs` per process (threads share it, as their credentials'
     user_ns), created by clone and unshare with CLONE_NEWUSER as create_user_ns does (deeper
     than 32 levels or over `/proc/sys/user/max_user_namespaces` ENOSPC, a chrooted caller or
     an owner or group the parent does not map EPERM), the creator getting every capability in
     it and none in its parent; the maps written once through `/proc/<pid>/uid_map`, `gid_map`
     and `setgroups` by map_write's rules (one write under a page at offset 0, at most 340
     extents, no overlap or wrap, new_idmap_permitted and verify_root_map, the lower ids taken
     through the parent's map); setns into one by userns_install's rules; the nsfs ioctls
     NS_GET_USERNS, NS_GET_PARENT and NS_GET_OWNER_UID by ns_get_owner's. Every other namespace
     records the user namespace that owns it. cap_capable's walk decides ns_capable: in the
     credentials' own namespace their effective set, over a namespace below it all
     capabilities when they own it. `capable()` keeps meaning the initial namespace, so a
     process's capabilities in a namespace of its own count for nothing outside it until a
     check moves to ns_capable on purpose (H4c). The maintainer's policy (2026-11-03): creating
     a user namespace needs CAP_SYS_ADMIN in the initial one, unless root sets
     `/proc/sys/kernel/unprivileged_userns_clone` to 1 (it is 0 by default), because a user
     namespace hands an ordinary user root-only kernel interfaces. H4b translates ids at the
     boundaries: credentials stay kernel ids, and an id a caller passes is its namespace's,
     made a kernel id on the way in (make_kuid: unmapped EINVAL for the set*id calls, chown,
     setgroups and tmpfs's `uid=`/`gid=`), while an id returned is shown as the receiver's
     namespace sees it (from_kuid_munged: unmapped reads as 65534) in the get*id calls,
     getgroups, stat and statx, SO_PEERCRED, waitid, auxv, `/proc/<pid>/status` (as the
     opener's namespace sees it, seq_user_ns, like the map files) and siginfo, whose si_uid is
     the sender's namespace's until the signal is queued and the receiver's after
     (userns_fixup_signal_uid; SIGCHLD carries the child's uid as its parent sees it). With
     ids translated, the set*id calls and exec take CAP_SETUID and CAP_SETGID in the caller's
     own namespace (ns_capable_setid), and setgroups also needs its namespace to allow it
     (userns_may_setgroups). Root, for the capability rules (the setuid fixup, exec's root
     emulation, access), is the namespace's uid 0, not kernel uid 0; a file capability counts,
     and reads back, by its root id as rootid_owns_currentns and cap_inode_getsecurity decide.
     H4c moves each privilege check to the namespace Linux names: CAP_SYS_ADMIN over the user
     namespace that will own a new namespace (clone, unshare: the new one's creator is its
     root), over a target's owner and in the caller's own for setns (CAP_SYS_CHROOT too for a
     mount namespace), over the UTS namespace's owner for sethostname and setdomainname, over
     the time namespace's owner, as the opener holds it, for `timens_offsets`; in the caller's
     own namespace for chroot and CLONE_PIDFD_AUTOKILL; over the target's for kill
     (CAP_KILL), prlimit (CAP_SYS_RESOURCE), setpriority and sched_setaffinity
     (CAP_SYS_NICE) and ptrace (CAP_SYS_PTRACE, and for a non-dumpable target over the
     namespace its memory belongs to, mm->user_ns, the credentials' at its last exec; exec's
     ptracer_capable asks it of the tracer's credentials from the attach); over the opener's
     for linkat(AT_EMPTY_PATH); over the child's PID namespace's owner for set_tid; and for
     files capable_wrt_inode_uidgid, the capability in the caller's own namespace when it
     maps the file's owner and group: the DAC overrides, CAP_FOWNER (chmod, utimes,
     O_NOATIME, the sticky bit, a sticky directory's user attributes), CAP_FSETID for keeping
     a set-group-ID bit, CAP_CHOWN and CAP_SETFCAP. A file capability written in a namespace
     is stored as v3 naming that namespace's root (cap_convert_nscap; v2 as given only from
     CAP_SETFCAP in the initial namespace, the filesystems'), so it never counts for the
     initial root. `capable()` stays where Linux keeps it: `trusted.*` and the other
     `security.*` attributes, raising a hard limit, RLIMIT_NPROC, setting the clock,
     CAP_WAKE_ALARM, real-time priority and nice, mknod, open_by_handle_at, keeping a
     set-user-ID bit, the userns sysctl; and mount, umount2 and pivot_root until ADR 0024
     gives them object capabilities (MNT_LOCKED has to come with that move, so mounts a less
     privileged namespace inherits cannot be undone to uncover what they hide).
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

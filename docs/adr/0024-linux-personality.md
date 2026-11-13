# ADR 0024: the Linux ABI as a personality, under one security model

- Status: accepted; decision 7's "all rights" superseded by ADR 0025 (2026-11-05), and its
  "Dropping" point by ADR 0026 (2026-11-08), which drops by rights
- Date: 2026-11-05
- Tracking: v0.28.0 order of work, item 4. The maintainer chose option (b) on 2026-11-03: the Linux
  system calls are to be served from user space on native IPC and capabilities, rather than kept
  in the kernel for good. On the same day they asked for one security model, and for this ADR
  after group 3 and before the remaining calls (item 5), so those calls are built for the move.
  The maintainer reviewed and accepted the details of this ADR as written on 2026-11-05.
- Superseded in part 2026-11-05 (step 2): ADR 0025 replaces the "all rights" of decision 7 with
  every right but GRANT, REVOKE and DELETE.
- Superseded in part 2026-11-08 (before step 6): ADR 0026 makes `CapabilityDrop` take a rights
  mask, narrowing a capability instead of only removing it.

## Context

VeridianOS is meant to be a microkernel: the kernel keeps memory, scheduling, IPC, object
capabilities and the hardware abstraction, and everything else runs as user-space programs
(critique C6). In fact everything runs in ring 0, and the Linux compatibility work of v0.27 and
v0.28 has made that surface larger:
- the Linux system calls (`kernel/src/syscall`);
- the Linux parts of `fs` and `process`;
- the namespaces of ADR 0022;
- the capability model of ADR 0020.

Left alone, the project becomes a Linux clone written in Rust.

Two systems serve a foreign ABI from user space on a capability kernel:
- **Fuchsia's Starnix.** The runtime shares the Linux program's address space, and the Linux code
  runs in Zircon's restricted mode. A system call or exception from restricted mode returns
  control to the runtime in normal mode, on the same thread, with no switch to another process.
- **gVisor's Sentry.** It does the same from a separate process, catching the calls through
  seccomp traps, ptrace or KVM. Each call costs a switch between processes, or a VM exit.

There are also two security models, and they meet in one place only:
- **The native model.** Unforgeable object capabilities (`kernel/src/cap`) name a kernel object
  and the rights over it.
- **The Linux model.** POSIX capabilities, uids and mode bits (ADR 0020) decide whether the caller
  is privileged enough. They do not name the object acted on.
- **Where they meet.** The private `mount` call checks CAP_SYS_ADMIN and also an object capability
  (ADR 0020, decision 9), but that check is weak: any Memory capability with WRITE and CREATE
  passes.

A survey of the tree on 2026-11-05 found the following:
- **About 60 Linux privilege checks.** The sites, by subsystem, are listed in Decision 5.
- **None of them consults an object capability.**
- **No object kind exists for a namespace, a mount, a clock or the system as a whole.** The kinds
  in `cap::ObjectRef` are Memory, Process, Thread, Endpoint, Interrupt, IoPort, PageTable,
  CapabilitySpace and Device.
- **`cap::manager::check_capability` checks rights and revocation, not the object's type.** Typed
  checks are written out by hand.
- **No user process can hold the capability that `mount` asks for:**
  - nothing grants capabilities at launch (`cap::inheritance::create_initial_capabilities` is
    never called);
  - exec empties the space, since nothing sets PRESERVE_EXEC;
  - the Memory capabilities a process can make itself never carry CREATE.
- **The private `umount` checks no POSIX capability.**
- **The package calls accept a capability of any type.**
- **Fork copies the space twice:** `fork.rs` clones it and `fork_inherit_capabilities` copies it
  again.

## Decision

### The personality

1. **Linux is a personality served by a user-space runtime.** A Linux runtime, `linuxd`, serves
   the Linux system calls of the programs it runs:
   - it runs on native IPC to the services, and holds capabilities from them;
   - the services are the filesystem, network, device and process servers that C6 moves out of
     ring 0;
   - it uses the native calls for memory and threads.

   The kernel keeps memory, scheduling, IPC, object capabilities, interrupts and the hardware
   abstraction, and knows nothing about Linux beyond the entry described next.
2. **A Linux call reaches the runtime through a personality entry, on the model of Zircon's
   restricted mode.**
   - **The two contexts.** A thread with the Linux personality has two contexts: the Linux
     program's, and the runtime's, which is mapped in the same address space at addresses the
     Linux program cannot map (the top of the user range, kept out of its `mmap` range as Starnix
     does).
   - **The entry.** A `syscall`/`svc`/`ecall`, or a fault, in the Linux context saves the Linux
     registers in a per-thread block that the runtime can read and write, and resumes the runtime
     context at its entry point. The runtime handles the call and returns through a native call
     that loads the Linux registers back. Signals are delivered by the runtime, which edits that
     block.
   - **Cost.** This is one kernel entry and exit per call, the same as today, with no switch
     between processes and no message.

   Rejected alternatives:
   - **One message per call to a separate runtime process, as gVisor's Sentry does.** It costs two
     switches between processes per call.
   - **Redirecting the calls in libc.** It does not cover static binaries or programs that make
     the calls themselves.
   - **Seccomp-style trapping.** It needs the Linux syscall path in the kernel anyway.
3. **The move goes one family at a time, behind a routing table.**
   - **The routing table.** Each personality has a table that says, per Linux system call number,
     whether the call goes to the in-kernel handler (today's code, kept as the reference
     implementation) or to the runtime.
   - **The first commit.** The entry and the table come first, with every number routed to the
     kernel; nothing changes for programs.
   - **A move.** Each later move routes one family to the runtime and deletes its kernel copy,
     once the LTP, Open POSIX and libc-test expected lists show no regression.
   - **Order.** The order follows what each family needs:
     1. calls that need only the runtime's own state: uname, personality, most of prctl, the
        identity and capability calls;
     2. signals, timers and process state, which need the native thread calls and the IPC
        redesign;
     3. files, after the filesystem servers (C6 d);
     4. sockets, after the network server (C6 b);
     5. namespaces last, since the runtime then implements them over its own tables.
4. **The moves wait for the IPC redesign** (N-79 to N-91, v0.29 item 16).
   - **Why.** The runtime needs typed endpoint capabilities, capability transfer and reply
     capabilities to talk to the servers, and none of these exists yet.
   - **Until then (v0.28 and v0.29 until item 16).** The compatibility code stays in the kernel,
     with these rules:
     - new code keeps Linux semantics behind module interfaces, with no new reach-ins across
       subsystems, so that each family can move whole;
     - README and the design documents describe the current state (everything in ring 0) and this
       target.

### One security model

5. **Every Linux privilege decision also needs an object capability for what it acts on.**
   - **The rule.** A POSIX capability (ADR 0020) or a uid or mode-bit check decides whether a
     caller is privileged enough. The object capability decides whether it may act on that object
     at all. Both must hold.
   - **What this gives.** A process confined by its object capabilities stays confined even as
     root with every POSIX capability, so the native model governs Linux programs rather than
     running beside them.
   - **Errors.** A missing object capability gives the error Linux gives for missing privilege at
     that point: EPERM, or EACCES where Linux's check is a permission check (inode access, search
     permission).

   New object kinds are added to `cap::ObjectRef`:
   - **`Namespace { kind, id }`:** one for each namespace of ADR 0022, user namespaces included,
     with the 64-bit namespace id.
   - **`Clock { id }`:** the system clock, and separately the alarm clocks.
   - **`System`:** the machine as a whole, covering sysctls, packages, reboot, swap, modules and
     the kernel log.

   The rights keep their existing bits, read as follows: READ to inspect, WRITE to act on, MODIFY
   to change configuration or topology, CREATE to create children, and EXECUTE to enter (setns).

   | Check (sites as of 2026-11-05) | Linux privilege | Object capability |
   |---|---|---|
   | Inode access: `fs::permission`, the xattr rules, O_NOATIME, utimes, chmod, chown, sticky removal, linkat AT_EMPTY_PATH, open_by_handle_at | DAC_*, FOWNER, FSETID, CHOWN, SETFCAP, SYS_ADMIN (trusted.*) | the mount namespace the path resolves through: READ for lookups and reads, WRITE for any change |
   | mknod of a device | MKNOD | mount namespace, CREATE |
   | mount, umount2, pivot_root, chroot; private mount and umount; the new mount API (item 5) | SYS_ADMIN, SYS_CHROOT | mount namespace, MODIFY |
   | kill and every signal sent to another process | KILL, uid match | the target's PID namespace, WRITE; or a `Process { pid }` capability with WRITE |
   | ptrace, `/proc/<pid>` access, scheduling, setpriority, prlimit | SYS_PTRACE, SYS_NICE, SYS_RESOURCE, uid match | the target's PID namespace, MODIFY; or `Process { pid }` with MODIFY (as setpriority checks today) |
   | clone3 set_tid | CHECKPOINT_RESTORE, SYS_ADMIN | the PID namespace, MODIFY |
   | clock_settime, settimeofday, adjtimex, clock_adjtime | SYS_TIME | `Clock` (the system clock), MODIFY |
   | timer_create on an *_ALARM clock | WAKE_ALARM | `Clock` (the alarm clock), WRITE |
   | timens_offsets | SYS_TIME | the time namespace, MODIFY |
   | sethostname, setdomainname | SYS_ADMIN | the UTS namespace, MODIFY |
   | unshare and clone with CLONE_NEW* | SYS_ADMIN (none for a user namespace) | the caller's namespace of that kind, CREATE; the caller's user namespace, CREATE, for CLONE_NEWUSER |
   | setns | SYS_ADMIN over the target's owner, SYS_CHROOT | the target namespace, EXECUTE |
   | uid_map, gid_map, setgroups | SYS_ADMIN, SETFCAP | the user namespace, MODIFY |
   | sysctls: unprivileged_userns_clone, pid_max, other writes (the initial IPC namespace's too; a new IPC namespace's, below) | SYS_ADMIN | `System`, MODIFY |
   | the package calls | (native) | `System`, MODIFY (replacing the check that accepts any type) |
   | RLIMIT_NPROC override | SYS_RESOURCE, SYS_ADMIN | `System`, MODIFY |
   | RLIMIT_MEMLOCK override: mlock, mlock2, mlockall, MAP_LOCKED and MCL_FUTURE growth past the limit, or with it 0 (item 5) | IPC_LOCK | `System`, MODIFY |
   | process_madvise on another process (item 5) | SYS_NICE, PTRACE_MODE_READ | the target's PID namespace, MODIFY; or `Process { pid }` with MODIFY (the ptrace row) |
   | native account calls (`veridian_auth`) on another user | DAC_OVERRIDE | the mount namespace, WRITE (the account store is a file) |
   | System V IPC and POSIX message queues: reading, waiting for zero, a read-only attachment, opening a queue (item 5) | the mode bits, IPC_OWNER | the IPC namespace, READ |
   | the same: sending, receiving, altering a semaphore, a writable attachment, mq_notify (item 5) | the mode bits, IPC_OWNER | the IPC namespace, WRITE |
   | the same: making an object (shmget, semget, msgget, mq_open with O_CREAT; item 5) | (none) | the IPC namespace, CREATE |
   | the same: IPC_SET, IPC_RMID, SHM_LOCK, mq_unlink, the IPC limits in /proc/sys (item 5) | owner, SYS_ADMIN, IPC_LOCK, SYS_RESOURCE | the IPC namespace, MODIFY (and `System`, MODIFY, for the initial IPC namespace's limits) |
   | set*id, setgroups, capset, the capability prctls, securebits | SETUID, SETGID, SETPCAP | none: these act on the caller's own credentials |

   Calls added later, in item 5 and beyond, take their row from this table, or add one to it in
   the same change.
6. **One typed check.** `cap::authority::holds(space, object, rights)` matches the object's kind
   and identity as well as the rights and revocation; `current_holds` applies it to the calling
   process (kernel context, with no current process, acts for itself and passes). It replaces the
   hand-written `iter_capabilities` scans as each subsystem converts. A process holds about a dozen
   of these capabilities, so the check is a scan of a small table; an index by kind is added only
   if a measurement of the inode path asks for one.
7. **How a process gets its capabilities.**
   - **At launch.** Each program the kernel starts (`process::creation::create_process_with_options`,
     which `boot_run_program` and the shell's launcher use) receives capabilities with all rights
     for the namespaces it is in, both clocks and `System` (`cap::authority::grant_initial`).
     `cap::inheritance::create_initial_capabilities`, never called, is left alone: its stack and
     heap capabilities name no real memory, and the IPC redesign decides what a process's native
     capabilities are.
   - **Fork and clone.** The child copies the parent's space, once; the second copy is removed.
   - **Exec.** Namespace, clock and `System` capabilities are kept across exec (PRESERVE_EXEC), as
     a Linux exec keeps its namespaces. The POSIX capabilities still decide privilege after exec
     (ADR 0020, decision 7).
   - **Authority through the owner.** A capability on a user namespace with some rights stands for
     those rights over every namespace it owns, and over its descendants, as Linux's
     `ns_capable(ns->owner, ...)` reaches every namespace its user namespace owns. A check on a
     namespace therefore passes with a capability on the namespace itself, or on its owner or one
     of the owner's ancestors (`cap::authority::holds_namespace`). This is what makes the Linux
     sequences work without a grant per namespace: `unshare(CLONE_NEWNS)` then mount (the new
     mount namespace belongs to the caller's user namespace), and `nsenter -m` then mount.
   - **Creating a user namespace.** The process that creates one (unshare or clone with
     CLONE_NEWUSER) receives a capability with all rights over it, so it has authority over the
     namespaces created inside it, as the user namespace's owner does on Linux. Other namespaces
     need no grant: they belong to a user namespace the creator already has authority over.
   - **Dropping.** A process can delete a capability from its own space with a new private call,
     `CapabilityDrop(kind, id)` (1056), which names the object (the token's type code: 9 a
     namespace by its id, 10 a clock, 0 the system clock and 1 the alarm clocks, 11 the system)
     rather than a token, since a process does not know its tokens. Revocation by token
     (`CapabilityRevoke`) is not suitable for this, because it revokes the token in every space
     that holds a copy. A launcher confines a program by dropping what it should not have before
     exec: for example the system clock or `System`, or a mount namespace together with the user
     namespaces above it, which would otherwise stand for it.
8. **The checks change one subsystem per commit, as in ADR 0020 step 6.**
   - **The first commit** gives mount(2), umount2 and pivot_root, and the private mount and umount
     calls, the same rule: a capability on the mount namespace with MODIFY.
     - It replaces the Memory WRITE+CREATE check.
     - The private umount also gains CAP_SYS_ADMIN.
   - **The other subsystems follow** in this order: namespaces, time, process and signals,
     filesystem inode checks, system.

## Consequences

- **Behaviour.**
  - Programs launched as today hold every capability, so the existing suites and expected lists do
    not change.
  - Confinement becomes possible that Linux itself cannot express. A launcher can run a root
    program that may not set the clock, mount, or signal outside its own PID namespace, whatever
    its POSIX capabilities.
- **The personality entry is new work in all three architectures' entry layers (ADR 0008, 0014,
  0016).** The kernel keeps the Linux syscall path as the routed default until each family moves,
  so for a while there are two paths. The routing table keeps both in one place.
- **The order of work.**
  - Moving the compatibility code out depends on the IPC redesign and C6, so no family moves in
    v0.28.
  - The capability side of this ADR can and does land in v0.28. It also gives the runtime the
    model it will enforce: when a family moves, the runtime makes the same checks against the
    capabilities it holds for its programs.
- **Cost.** Each privileged operation does one more lookup, a scan of the caller's small
  capability table. The inode path, the most frequent, checks once per path walk, not once per
  component.
- **Error mapping.** The private mount calls change their error for a missing object capability
  from EACCES to EPERM, as Linux's mount does.
- **Stale documentation corrected with the first commit.**
  - KNOWN-LIMITATIONS N-129 ("mount is allowed to any process holding a memory capability")
    describes the old rule.
  - ARCHITECTURE-OVERVIEW's "user programs cannot reach IPC yet (N-33)" predates v0.27's endpoint
    calls.

## Implementation order

1. This ADR; the new object kinds; `cap::authority`; the grants at launch, PRESERVE_EXEC, the
   single copy at fork, and `CapabilityDrop`. Host tests for each; behaviour unchanged.
2. The mount namespace rule for mount, umount2, pivot_root and the private mount and umount calls,
   with CAP_SYS_ADMIN on the private umount; `holds_namespace` (authority through the owner) and
   the grant to a new user namespace's creator, which the rule needs to keep mounting in new
   namespaces working. A runtime test checks that a root process that has
   dropped its mount namespace capability gets EPERM from mount.
3. The namespaces calls (unshare, clone, setns, the user namespace maps, sethostname); a runtime
   test for setns without the target's capability.
4. Time (the clocks, the alarm clocks, timens_offsets); a runtime test that a root process without
   the system clock capability cannot set it.
5. Process and signals (kill, ptrace and `/proc`, scheduling, prlimit, set_tid).
6. The filesystem inode checks.
7. System (sysctls, packages, RLIMIT_NPROC, the account calls); README, the design documents,
   PORTING-GUIDE, KNOWN-LIMITATIONS, CHANGELOG.
8. With the IPC redesign (v0.29): the personality entry and the routing table, every number routed
   to the kernel; then the moves of Decision 3, each its own pull request.

## References

- Fuchsia: Starnix and Zircon restricted mode (`zx_restricted_enter`, `zx_restricted_bind_state`)
- gVisor: the Sentry and its platforms (systrap, ptrace, KVM)
- ADR 0008, 0014, 0016 (entry layers); ADR 0020 (POSIX capabilities); ADR 0022 (namespaces)
- Linux 7.3: `kernel/capability.c` (`ns_capable`), `fs/namespace.c` (`may_mount`)

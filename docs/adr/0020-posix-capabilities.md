# ADR 0020: POSIX (Linux) capabilities

- Status: accepted
- Date: 2026-10-23 (drafted 2026-10-21)
- Tracking: v0.28.0 system call program (process group: capget, capset, prctl); N-131. The user
  chose the full Linux capability model on 2026-10-18, with an ADR first.

## Context

VeridianOS decides privilege the way Unix did before Linux 2.2: `Credentials::privileged()` is
`euid == 0` and `fs_privileged()` is `fsuid == 0` (`kernel/src/process/creds.rs`). About twenty-five
decision sites in eight files use them: the set*id calls, file permission checks,
chmod/chown/utimensat, sticky-bit removal, exec permission, kill, ptrace, scheduling priority,
rlimits, settimeofday, chroot, mknod and the native account calls. In one place, `VfsContext`, an
fs-privileged caller skips every VFS-layer check at once.

capget and capset are not implemented (ENOSYS), and prctl rejects every capability option
(`PR_CAPBSET_*`, `PR_CAP_AMBIENT`, `PR_*_SECUREBITS`, `PR_*_KEEPCAPS`). Programs that use
libcap, drop privileges with capabilities (systemd services, ping, Wireshark's dumpcap, container
runtimes, BusyBox's `setpriv`), or carry file capabilities therefore either fail or run with all
of root's power. LTP's capability tests (capget01-02, capset01-04, the prctl and cap_bounds tests,
kernel/security/filecaps) cannot pass.

Linux's model, as implemented in `kernel/capability.c` and `security/commoncap.c` (Linux 7.3):

- Each **thread** has five 64-bit sets: effective (pE), permitted (pP), inheritable (pI),
  bounding, and ambient (pA), plus the securebits and the keep-caps flag. Capabilities 0 to 40
  exist (`CAP_LAST_CAP` = `CAP_CHECKPOINT_RESTORE`).
- **capget/capset** use a header with a version (V1 32-bit; V2/V3 64-bit in two words) and a
  pid. capset changes only the calling thread, under four subset rules (EPERM otherwise).
- **execve** computes the new sets from the old ones and the file's capabilities:
  `pP' = (pI & fI) | (fP & bounding) | pA'`, `pE' = fE ? pP' : pA'`, with the ambient set cleared
  for privileged files. Root is emulated: for ruid or euid 0, fP and fI count as all ones (unless
  SECBIT_NOROOT). A "capability-dumb" binary (fE set) that would lose a capability it asks for
  fails with EPERM. no_new_privs, an unprivileged tracer or a shared fs cap the gain.
- **UID changes** fix the sets up: losing all root UIDs clears pP and pE (unless KEEP_CAPS),
  euid changes move pE, and fsuid changes move the filesystem capabilities (`CAP_FS_MASK`).
- **File capabilities** live in the `security.capability` extended attribute (v1, v2, or v3
  with a namespace root id).

VeridianOS also has an unrelated **object capability** system (`kernel/src/cap/`): unforgeable
tokens that name and authorise kernel objects such as IPC endpoints and memory. The two must not
be confused, in code or in documentation.

## Decision

1. **Implement Linux's model exactly, for the initial user namespace**, with Linux's numbering,
   ABI, errors and transformation rules. Where this ADR is silent, Linux 7.3 is the
   specification.
2. **Capability state is per thread.** pE, pP, pI, the bounding set, pA, the securebits and
   keep-caps live in the thread's credentials. capset and the prctl options change the calling
   thread only, as on Linux; libc broadcasts set*id and capset to all threads where POSIX needs
   it (musl does for set*id). The user and group IDs stay per process.
   - A child created by fork or clone copies the creator's state.
   - The user and group IDs being per process, a set*id call changes them for every thread at
     once, so the UID-change fixups (item 5) are applied to every thread's sets in the same step,
     each under its own securebits. On Linux, musl's set*id broadcast reaches the same state, one
     thread at a time.
   - The set*id rules take their privilege from the calling thread: CAP_SETUID for the user IDs
     and setfsuid, CAP_SETGID for the group IDs, setfsgid and setgroups.
3. **One representation and one check.** A `LinuxCaps(u64)` bitset masked by
   `CAP_VALID_MASK`, and one check: `Caller::capable(cap)` (the calling thread's pE, from
   `Caller::current()`), with `CapSets::capable(cap)` for another thread's sets (Linux's
   `has_capability`).
   Every uid-0 decision site is converted to the capability Linux checks there,
   and `privileged()` / `fs_privileged()` are removed. The VfsContext bypass becomes the
   individual checks: CAP_DAC_OVERRIDE, CAP_DAC_READ_SEARCH and CAP_FOWNER.
   - Every check is in the initial user namespace, and the checks take no namespace argument
     yet. (The draft gave them one, `ns_capable(ns, cap)`; it was left out because nothing could
     pass any other value. Corrected 2026-10-27 in review.) The namespace work adds it to these
     two methods: about 33 call sites, all through them, so it is a signature change rather than
     a second conversion.
4. **Root keeps working.** A process starting as root gets all capabilities, and the exec and
   UID-change rules emulate root as Linux does, including SECBIT_NOROOT, SECBIT_NO_SETUID_FIXUP
   and SECBIT_KEEP_CAPS. Unmodified login, su, BusyBox and service managers behave as on Linux.
5. **The UID-change rules become Linux's** (`cap_task_fix_setuid`), unless SECBIT_NO_SETUID_FIXUP:
   - losing every root user ID (real, effective and saved) clears pP and pE unless SECBIT_KEEP_CAPS,
     and clears pA in any case; an effective uid leaving 0 clears pE, one becoming 0 sets pE to pP;
   - an fsuid leaving 0 drops `CAP_FS_SET` (CHOWN, DAC_OVERRIDE, DAC_READ_SEARCH, FOWNER, FSETID,
     MKNOD, MAC_OVERRIDE and LINUX_IMMUTABLE) from pE, one becoming 0 raises it from pP. This
     replaces the `fs_privileged()` approximation.
6. **File capabilities need persistent extended attributes, so those come first.**
   - BlockFS gains on-disk extended attributes (a format version bump with a migration path in
     `mkfs-blockfs`); tmpfs and ramfs keep extended attributes in their nodes. The in-memory store
     keyed by inode number is retired.
   - The `security.`, `trusted.`, `user.` and `system.` namespaces follow Linux's permission
     rules, and the setxattr/getxattr/listxattr/removexattr families are implemented.
   - `security.capability`: read v1, v2 and v3 at their exact sizes; write v2 (or v3 with root id
     0) and only with CAP_SETFCAP; a v3 with a non-zero root id means no capabilities until user
     namespaces exist. A write, truncate or chown of the file removes it (Linux's killpriv).
7. **Exec implements `cap_bprm_creds_from_file`**, including the unsafe-exec cap, the ambient
   clear, the capability-dumb EPERM, the KEEP_CAPS clear, AT_SECURE from Linux's condition, and
   non-dumpable when pP grows.
8. **All six securebits and their locks** are implemented; SECBIT_EXEC_RESTRICT_FILE and
   SECBIT_EXEC_DENY_INTERACTIVE are stored and reported (they are advice to interpreters).
9. **Native calls keep their own authority.** Object capabilities stay the authority for native
   IPC and memory. Where a native call stands for a POSIX privilege, it checks the POSIX
   capability that matches: `mount` checks CAP_SYS_ADMIN as Linux does, in addition to its
   object-capability gate (both must hold); the native account calls (`veridian_auth`) that act on
   another user check CAP_DAC_OVERRIDE, the power they stand for (writing `/etc/shadow`).
10. **Naming.** The module is `process::linux_caps` and its types are `LinuxCaps`, `CapSets` and
    `Securebits`. Nothing in it is called `Capability*`. Documentation says "Linux capabilities"
    for this model and "object capabilities" for `kernel/src/cap/`.
11. **Interfaces:** capget/capset (x86_64 125/126, generic 90/91), the prctl options,
    `/proc/sys/kernel/cap_last_cap`, and the `CapInh/CapPrm/CapEff/CapBnd/CapAmb` lines in
    `/proc/<pid>/status`.

## Consequences

- Programs that manage privilege with capabilities work unmodified, and a service can run with
  exactly the privileges it needs instead of full root.
- Every privileged path changes. The conversion is done one subsystem at a time, each with host
  unit tests (a non-root thread holding exactly the capability passes; root without it fails) and
  the matching LTP tests, so a regression points at one subsystem.
- Per-thread state means the credentials a check reads are the current thread's, not the
  process's; code that passes a process where Linux passes a task must change.
- BlockFS's on-disk format changes for extended attributes. Existing images are migrated by
  `mkfs-blockfs` (or rebuilt by the rootfs scripts), and the format version guards the mount.
- User namespaces remain future work; the single check (`Caller::capable`, `CapSets::capable`)
  and the v3 root-id rule are the hooks for them.
- The kernel console's root shell (N-131) keeps all capabilities; logging in there is a separate
  item.

## Implementation order

1. This ADR; `LinuxCaps`, `CapSets` and pure transition functions (capset rules, exec, UID and
   fsuid fixups, prctl), each with host unit tests transcribed from `commoncap.c`.
2. Per-thread credential plumbing, root starting with all capabilities; behaviour unchanged
   (every existing suite stays green).
3. capget/capset and `cap_last_cap`; LTP capget01-02, capset01-04.
4. The prctl options and the `/proc/<pid>/status` lines; LTP prctl and cap_bounds tests,
   `capsh --print`.
5. UID-change fixups; `capsh --keep=1 --user=nobody`.
6. The uid-0 check sites, one subsystem per change, each with its LTP group.
7. Exec without file capabilities: root rules, ambient set, AT_SECURE, dumpable.
8. Persistent extended attributes and the xattr system calls; LTP setxattr, getxattr, listxattr,
   removexattr.
9. `security.capability` at exec and on write; LTP filecaps, libcap's tests, setcap/getcap.
10. N-131's capability part closed in KNOWN-LIMITATIONS; PORTING-GUIDE; CHANGELOG.

## References

- capabilities(7), capget(2), prctl(2), xattr(7) (man7.org)
- Linux 7.3: `include/uapi/linux/capability.h`, `include/linux/capability.h`,
  `kernel/capability.c`, `security/commoncap.c`, `include/uapi/linux/securebits.h`,
  `include/uapi/linux/prctl.h`, `kernel/cred.c`

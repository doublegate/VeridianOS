# ADR 0029: Landlock through the authority entry

- Status: proposed
- Date: 2026-11-14

## Context

Linux 7.3's Landlock (`security/landlock/`, ABI 11) lets an unprivileged process restrict itself
and everything it starts afterwards. It does this with stacked rulesets: filesystem access by
hierarchy, TCP and UDP ports, and scopes for abstract UNIX sockets and signals. Programs use it
by default: systemd services, the GNU coreutils sandbox, Chromium, and sandboxing tools such as
`landrun` and Firejail. The v0.28 plan (item 5, system tier) implements it in full.

The maintainer fixed two constraints on 2026-11-13:

1. Landlock is enforced through one authority entry, `authority::holds_path` and
   `authority::holds_net`.
2. The directory and network capabilities of v0.29 (Track S) plug into the same entry, and a
   Landlock ruleset later becomes a capability drop (one model). The path-blind
   `mac::check_file_access` it replaces is retired with the MAC module (ADR to come).

Two facts shape the design:

- **Landlock rules name objects, not paths.** A rule's anchor is the inode of the directory or
  file it was added on (`landlock_object`). An access is allowed in a layer if some ancestor of
  the target, walking up through mount points to the real root, carries a rule granting it. A
  rename keeps the rule with the inode, and a bind mount reaches the same inode under another
  path.
- **The VFS resolves path strings.** `Mounts::resolve_in` walks the canonical path top-down,
  one component at a time; there is no dentry cache with parent pointers yet (Track P2).

## Decision

### Model

- **Ruleset:** a `Ruleset` is the result of `landlock_create_ruleset`. It holds a handled-access
  set (fs, net, scope) and rules. Filesystem rules map an object key (`fs::inode_key` of the
  base node: idmapped views are transparent) to an allowed mask. Network rules map
  (protocol, port) to an allowed mask. A ruleset descriptor is an anonymous file, as on Linux.
- **Domain:** `landlock_restrict_self` adds the ruleset as the top layer of the caller's domain.
  The domain is an immutable `Arc` chain of at most 16 layers. Threads share it, fork, clone and
  exec keep it, and a new layer makes a new chain. TSYNC (`LANDLOCK_RESTRICT_SELF_TSYNC`) gives
  it to every thread, under the thread map, as seccomp's TSYNC does (ADR 0027 decision 27).
- **Conditions:** restricting needs no_new_privs or CAP_SYS_ADMIN in the caller's user namespace.
  `LANDLOCK_RESTRICT_SELF_NO_NEW_PRIVS` sets no_new_privs first. The log flags are accepted and
  log nothing, since there is no audit subsystem (DECISIONS 2026-11-11).

### Ancestors from the canonical path

- Every check names its object by the canonical path the lookup produced (no `.`, `..` or
  followed symlink, in the whole tree). For a restricted caller, the check walks that path
  once more from the root and records the object key of every directory it passes. A mount
  point contributes only the top mount's root: Linux's walk up follows a mount root to the
  point it covers and goes straight on to that point's parent ("ignores hidden mount
  points"), so neither the covered directory nor the roots of mounts stacked under the top
  one carry a rule that counts. The key is `fs::inode_key` of the base node, so idmapped
  views are transparent.
- The walk is bound to the object the lookup resolved by a final identity comparison: when
  the caller holds that object and the walk does not end at it, the ancestors are not the
  object's, so the check keeps only the object's own key and grants what rules on the object
  itself allow. The walk ends elsewhere after a rename between the lookup and the check, and
  stops short when the path no longer leads anywhere: a file moved or removed since a
  descriptor recorded its path (`O_PATH` with `AT_EMPTY_PATH`). Like a part of the path that
  has gone, this can only grant less. Linux walks the object's own parents (`d_parent`): it
  has no rename window, finds a moved file's new directory, and for a removed file still
  walks the old one, so a check here can be refused where Linux's would pass in those cases.
  linkat of a descriptor whose path no longer leads to its file is EACCES for the same
  reason: the directory the file is in is not known.
- An O_TMPFILE file has no name; its check is its own key then its directory's chain, which
  the open passes explicitly, as Linux's walk starts at the file's `d_parent`.
- The check then reads that list once from the target up, which is O(depth) with no lookup
  per ancestor. The walk happens only when the caller's domain is not empty, so unrestricted
  processes pay one null test per hook.
- A `/proc/<pid>/fd` link is an ordinary symlink to the file's path, so the walk reaches the
  real file. nsfs, reached through a magic link, is internal and always allowed
  (`MNT_INTERNAL`).
- When the dentry cache arrives (Track P2), its parent pointers replace the second walk. The
  authority entry's signature does not change.
- **Rule anchors stay alive:** a domain holds the nodes its rules name, as Linux's
  `landlock_object` holds its inode. A key made from a node's address therefore cannot pass
  to another object while the domain lives.

### The authority entry

- **Filesystem:** `cap::authority::holds_path(domain, &Ancestry, access)` answers for an access
  right set. The VFS and the syscall layer call it where Linux's hooks are:
  - `file_open`: READ_FILE, WRITE_FILE, READ_DIR, EXECUTE (at exec);
  - `path_mknod`, `path_mkdir`, `path_symlink`: the MAKE_* rights;
  - `path_unlink`, `path_rmdir`: REMOVE_*;
  - `path_link`, `path_rename`: REFER and its rules, with the cross-directory check of
    `current_check_refer_path`;
  - `path_truncate`;
  - `unix_find`: RESOLVE_UNIX, ABI 9.
- **Open files:** the rights a file is opened with (TRUNCATE, IOCTL_DEV) are computed once at
  open and kept on the `File`, as Linux's `landlock_file_security`. ftruncate and device ioctls
  check the file, not the path.
- **Network:** `cap::authority::holds_net(domain, port, access)` is checked on bind and
  connect for TCP, and on bind, connect and sendto/sendmsg with an address for UDP (ABI 10).
- **Scopes:** an abstract UNIX socket or a signal may reach only a process whose domain is the
  caller's or below it (`domain_is_scoped`). The domain is the named task's, as
  `hook_task_kill`'s: for a signal sent to one thread (tkill, tgkill, rt_tgsigqueueinfo,
  PIDFD_SIGNAL_THREAD) that thread's, which differs from the leader's when the leader
  restricted itself without TSYNC; for a process, its leader's. The same domain relation
  governs ptrace, process_vm, pidfd_getfd and `/proc/<pid>/mem`, as `hook_ptrace_access_check`;
  ptrace names a process here (attaching one thread of a process is not supported yet), so
  that check takes the leader's domain.
  Abstract UNIX sockets and SIGIO do not exist yet: whoever adds them adds these scope checks
  in the same change (MASTER_TODO item 10), and the runtime test `landlock` has a tripwire
  that fails if an abstract socket outside the domain can be reached.
- **Mounts:** a process with a filesystem domain cannot mount, unmount, remount, move or
  pivot_root (EPERM), as the `sb_*` hooks.
- Errors as Linux: EACCES for filesystem and network denials; EPERM for scopes (signals),
  mount changes and ptrace.
- The checks sit where Linux's do, relative to the other checks: an open after the write
  access (ETXTBSY) and before O_TRUNC; a creation after EEXIST and before the directory
  permission check; an unlink after the type checks; a link or rename after EXDEV and EROFS;
  exec checks READ_FILE and EXECUTE on the program and on its interpreter. A descriptor that
  is not a ruleset is EBADFD, as is a rule anchor without a path (a pipe, a socket, an
  anonymous inode) or on nsfs.
- `landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION)` returns 11, and
  `LANDLOCK_CREATE_RULESET_ERRATA` returns the errata bits this implementation includes.

### Placement

- The ruleset and domain types, and the access-check arithmetic (`landlock_unmask_layers`,
  `is_access_to_paths_allowed`'s per-layer masks, the REFER rules), go into a no_std,
  `forbid(unsafe)` crate, `veridian-landlock`. It is host-tested and shared with linuxd (Track M),
  like `veridian-cbpf`.
- The kernel provides the hooks, the walk ancestry and the descriptors.

## Consequences

- One check site answers both Landlock today and capabilities tomorrow. A v0.29 directory
  capability is an ancestor-rooted allowed mask, which is the same arithmetic.
- An unrestricted process pays one null-pointer test per hook.
- A restricted process pays a second walk of the canonical path, and one small vector, per
  checked operation, until the dentry cache.
- A rule on a BlockFS file names its inode number. BlockFS does not give a number to a new
  file while a node in memory still names the old one (`blockfs::InodeHolds`), and a domain
  holds the nodes its rules name. So a rule on a deleted file never covers another file, as
  Linux keeps the inode until the rule goes.
- `mac::check_file_access` is still consulted beside the new entry, so both must allow an
  access. The MAC module is retired by its own ADR (Track S, v0.29 to v0.30), which removes
  those calls; keeping them here keeps this change to one behaviour, Landlock's.
- **Rejected:**
  - Rules keyed by canonical path strings: cheap, but wrong after a rename and through bind
    mounts.
  - Re-resolving every ancestor prefix on each check: O(depth²) lookups per access.
  - Recording the keys during the lookup itself: the lookup splices symlink targets and
    replays `..` by re-walking from the root, so the list would have to be rebuilt at each of
    those points. One extra walk of the finished canonical path gives the same list and leaves
    the lookup alone until the dentry cache replaces both.
  - Checking only at open: Linux also checks creation, removal, linking, renaming, truncation
    and UNIX socket resolution.

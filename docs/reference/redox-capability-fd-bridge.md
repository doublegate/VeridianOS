# Redox Capability–FD Bridge (Notes for VeridianOS)

## Goal
Understand how Redox OS preserves POSIX-style file descriptors while enforcing capability security, and distill patterns we could adapt for a VeridianOS compatibility layer.

## Redox design snapshot
- **Capsicum-style caps**: Redox is moving toward Capsicum-like capabilities layered over a POSIX API, with an NLnet-funded project to replace legacy FD handling. citeturn0search0  
- **Namespace + “schemes”**: Redox names resources by scheme (`file:`, `tcp:`, `mem:`); processes get a per-process namespace that maps path prefixes to scheme endpoints (like OpenBSD’s `unveil`). citeturn0search2  
- **Capability handles**: Internally, handles carry rights (read, write, seek, execute) and a scheme route. POSIX FDs are thin wrappers around these capability handles. citeturn0search5  
- **openat-first**: All path resolution is relative to a directory capability (dirfd); absolute paths are synthesized by using the root namespace capability. This mirrors Capsicum’s `openat()`-only model. citeturn0search0  
- **FD passing**: Handles can be transferred over Unix-domain sockets; receiving side gains only the rights encoded in the handle (no ambient lookup). citeturn0search7  
- **Null namespace for sandbox**: Processes can start with an empty namespace and explicitly graft only the capabilities they need, enabling tight sandboxing. citeturn0search2  
- **Compatibility surface**: libc exposes classic POSIX calls, but each call ultimately routes through the capability dispatcher and scheme table rather than a global VFS. citeturn0search5  
- **Status (Feb 2026)**: FD-to-cap bridging is under active development; Capsicum parity is a stated deliverable of the NLnet grant. citeturn0search0

## How it maps to VeridianOS
| Need in VeridianOS | Redox pattern | Candidate approach |
| --- | --- | --- |
| POSIX compatibility without ambient authority | Capability-wrapped FDs + per-process namespace | Expose a POSIX libc that resolves paths via a per-process capability namespace; forbid global root by default. |
| Path-based open | `openat()` relative to dirfd | Make `open()` a thin shim that resolves to `openat(dirfd=root_cap, ...)`; encourage direct `openat`. |
| Resource typing (files, sockets, shm) | Schemes carry type + rights | Model “capability classes” (file, socket, mem, device) with explicit rights bits; keep kernel IPC/cap tables as the source of truth. |
| FD passing | UDS capability transfer | Implement descriptor passing over Unix-domain sockets (or Veridian message channels) that clones the cap with restricted rights. |
| Sandbox init | Empty namespace boot | Let launchers start processes with an empty or minimal namespace and graft vetted caps (e.g., `/usr`, `/tmp`, one socket). |
| Revocation | Rights baked into handle; re-open needed for upgrades | Add epoch/generation to caps so revoked namespaces invalidate inherited handles; reopening requires a delegating cap. |

## Proposed compatibility layer sketch
1. **Capability-backed FD type**: POSIX fd is a small index into a per-process table of `(cap_id, rights, scheme)`; syscalls translate to cap-aware operations.  
2. **Namespace table**: Each process holds a map `{ prefix -> dir_cap }`. `open(path)` resolves the longest prefix, then uses `openat`.  
3. **Delegation API**: A launcher builds the namespace by attaching caps (with masked rights) before exec. Provide a `cap_attach(prefix, cap, rights)` syscall.  
4. **FD passing**: Extend IPC/UDS to send `(cap_id, rights_mask)`. Receiver installs it into its FD table with masked rights.  
5. **Revocation/generation**: Store a generation counter per cap; namespace entries cache `(cap_id, gen)`. Kernel rejects stale generations.  
6. **Auditability**: Log namespace grafts and cap transfers; align with VeridianOS invariants document.  
7. **Fallback for pure capability mode**: Allow processes to drop the POSIX shim and operate directly on caps for minimal TCB.

## Migration concerns
- **Libc surface area**: Need wrappers for `open`, `creat`, `chdir`, `fchdir`, `chmod` variants that respect namespaces.  
- **Path resolution cost**: Namespace longest-prefix match adds overhead; cache resolved dirfd per (device,inode) pair with generation checks.  
- **Tooling**: Provide a `cap-ns` CLI to inspect per-process namespaces (debug only).  
- **Testing**: Add regression tests mirroring Capsicum libc tests (openat-only sandboxes, fd passing, rights downgrades).  
- **Interoperability**: Ensure capability rights map cleanly to VeridianOS capability model (read/write/execute/append/ioctl/rename/metadata).  

## Quick win for VeridianOS
- Start with **dirfd-first openat** plus per-process namespace and capability-backed fd table; defer full FD passing until IPC transports rights.  
- Ship a minimal libc shim that routes `open()` -> `openat(root_cap, ...)`, with root_cap explicitly provisioned by the launcher.  
- Use the existing capability registry and add a small namespace table in the kernel process control block.  

## References
- NLnet grant for capability-based Redox (Capsicum parity) – project notes and goals. citeturn0search0  
- Redox scheme-based namespace and empty-namespace sandboxing. citeturn0search2  
- POSIX FD wrapping over capability handles and dispatcher path. citeturn0search5  
- FD/handle passing over Unix-domain sockets with rights preservation. citeturn0search7  

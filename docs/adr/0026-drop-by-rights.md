# ADR 0026: dropping authority by rights

- Status: accepted
- Date: 2026-11-08
- Supersedes: ADR 0024, decision 7, only its "Dropping" point (`CapabilityDrop(kind, id)`
  removes every capability naming the object). The rest of that ADR stands, as does the
  rights decision that amended it.
- Tracking: v0.28.0 order of work, item 4, between steps 5 and 6. On 2026-11-08 the maintainer
  chose "drop by rights mask" over the alternatives and asked for it in its own commit before
  step 6, recorded in a superseding ADR, and accepted this ADR on 2026-11-08.

## Context

ADR 0024 decision 7 lets a process confine what it runs by deleting capabilities from its own
space with `CapabilityDrop(kind, id)` (1056), and step 6 is to put the filesystem's inode
checks behind the mount namespace's capability: READ for lookups and reads, WRITE for any
change, CREATE for device nodes.

`CapabilityDrop` removes a capability whole. Once step 6 lands, the only way to stop a program
mounting (MODIFY on its mount namespace and on the user namespaces above it, which stand for
it) also takes away every file it could read or write, because the same capabilities carry READ
and WRITE. A launcher could then confine mounting only by also confining file access, which
makes the drop useless for the common case, "root that cannot mount". This was found while
planning step 6, before any of it was written.

## Decision

1. `CapabilityDrop` takes a third argument, `rights`, in the token's rights encoding (READ 1,
   WRITE 2, EXECUTE 4, GRANT 8, REVOKE 16, DELETE 32, MODIFY 64, CREATE 128).
   - `rights == 0`, or every right the capability carries, removes the capability as before.
     The call has not been released (it is on the v0.28 branch only), so its callers are this
     repository's tests, which pass the third argument explicitly; a third register left over
     from earlier code is not 0, so no caller may rely on it.
   - Otherwise each capability naming the object is narrowed to the rights it has minus
     `rights`; one left with no right at all is removed.
   - Bits above CREATE are EINVAL.
2. Narrowing only: no call can add a right to a capability, and a dropped right cannot come
   back except through a new grant from the kernel (a new user namespace or a new launch).
3. The return value is the number of capabilities narrowed or removed; a second identical call
   returns 0.
4. Fork copies the narrowed capability, and exec keeps it, as it does any authority capability.
5. Owner chains are unchanged. A namespace still passes a check through a capability on a user
   namespace above it, so a launcher stops a program mounting by dropping MODIFY on the mount
   namespace and on each user namespace above it; READ and WRITE remain and files still work.
6. User space gets a header wrapper, `veridian_capability_drop(kind, id, rights)` in
   `<veridian/capability.h>`, with the kind, id and right constants, so programs built against
   musl and the native libc call it the same way.

## Consequences

- Step 6 can put file access behind the mount namespace without making "cannot mount" mean
  "cannot read". Its runtime test covers both: a MODIFY-only drop keeps files working, a full
  drop gives EACCES.
- The call keeps its number; with `rights` 0 it does what it did. The native libc had no
  wrapper for it before; this adds one.
- A capability can now carry a subset of `authority_rights()`. Every check already asks for the
  specific right it needs (`holds` tests `contains`), so a narrowed capability passes exactly
  the checks for the rights it kept.
- Narrowing changes the entry in the caller's own space only, like a full drop: the parent's
  and siblings' copies of the same token keep their rights.

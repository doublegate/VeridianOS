# ADR 0025: the rights of the authority capabilities

- Status: proposed
- Date: 2026-11-05
- Supersedes: ADR 0024, decision 7, only where it gives "all rights" (at launch, and to the
  creator of a user namespace). The rest of ADR 0024 stands.
- Tracking: v0.28.0 order of work, item 4, step 2. The maintainer chose on 2026-11-05 to record
  this change in a superseding ADR rather than by editing ADR 0024.

## Context

ADR 0024 decision 7 gives every program the kernel starts capabilities "with all rights" over
its namespaces, both clocks and `System`, and gives the process that creates a user namespace a
capability "with all rights" over it. Step 1 implemented that with `Rights::ALL`.

A security review of step 1 found that this lets one process take another's authority away:
- Fork copies the parent's capability space token for token (`clone_from`), so parent and child
  hold the same token, and so do siblings.
- `CapabilityRevoke` needs only the REVOKE right on the caller's own copy, and revokes the token
  everywhere it is held.
- So any child, including one its launcher had confined, could revoke its parent's and its
  siblings' namespace, clock and system authority, and with it their ability to mount, set the
  clock or do anything else a later step puts behind these capabilities.

GRANT and DELETE are not needed either: no check asks for them, granting is the kernel's job
(at launch and at user-namespace creation), and a process removes a capability from its own
space with `CapabilityDrop` (1056), which ADR 0024 added for that purpose and which touches no
other space.

## Decision

1. The authority capabilities (namespace, clock and `System`, at launch and at user-namespace
   creation) carry `cap::authority::authority_rights()`: READ, WRITE, EXECUTE, MODIFY and
   CREATE, every right a check asks for, and not GRANT, REVOKE or DELETE.
2. A process gives up one of its own authority capabilities with `CapabilityDrop`, never with
   `CapabilityRevoke`, which these capabilities refuse.
3. If a later step needs a process to hand authority on (a launcher delegating to a service), it
   gets a derived capability from the kernel, recorded in the derivation tree with the IPC
   redesign (item 16), not a copy of a token it can revoke for everyone.

## Consequences

- A confined child cannot strip its parent or siblings of their authority. A host test
  (`a_forked_copy_cannot_revoke_the_parents_authority`) shows the child's revoke failing and the
  parent still holding.
- Nothing that worked stops working: no caller revoked these capabilities, and every check asks
  only for rights still granted.
- Revoking a whole family's authority at once is no longer possible from user space. The kernel
  can still do it, and the IPC redesign's derivation tree is where a revocable delegation
  belongs.

# ADR 0005: Copy-on-write fork with shared-frame owner counts

- Status: accepted
- Date: 2026-10-07
- Tracking: MEM-PERF-03, MEM-INC-01, PROC-ARCH-01, MEM-ARCH-02, N-07

## Context

`fork` deep-copied every user page of the parent. At the same time it registered the parent's
frames in a global CoW table. That table was keyed by frame number, although its documentation said
virtual address, and it was never consulted or cleaned up. Three CoW code paths were unreachable,
and a global demand-paging table that `mmap` filled was also never read. The kernel had no COW
page bit and no per-frame owner count.

## Decision

- **COW bit.** `PageFlags::COW` is software bit 9 of the PTE, which the MMU ignores.
- **Sharing at fork.** `VirtualAddressSpace::clone_from` maps the parent's frames into the child.
  - Writable pages become read-only + COW in both address spaces.
  - Read-only pages are simply shared.
  - The parent's TLBs are then flushed on every CPU.
- **Owner counts.** `mm::frame_refs` records *extra* owners per shared frame. A frame with a single
  owner has no entry, so unshared memory costs nothing. Every path that frees a user frame first
  drops one owner, and frees the frame only when the last owner lets go:
  - unmap and munmap;
  - `clear` and `clear_user_space` (exec);
  - address-space teardown;
  - a failed fork.
- **Write faults.** `VirtualAddressSpace::resolve_cow_fault` handles a write to a COW page:
  - if the frame is still shared, it copies the page into a private frame;
  - if it was the last owner, it just makes the page writable again.

  The mapping itself must be writable, so COW never grants a write the process could not
  otherwise make. The user-mode fault path and the kernel's user-copy fixup both use it. A
  `read()` into a buffer still shared after `fork` is therefore a COW copy, not EFAULT.
- **Removed.** The global CoW table, the dead `cow_fork`, `VirtualAddressSpace::fork` and
  `handle_page_fault`, and the `demand_paging` module.
- **NX.** `PageTableEntry::flags()` now keeps NX (bit 63). Fork re-maps pages with their translated
  flags, and NX had been dropped, so a forked child's data and stack were executable.

## Consequences

- Fork no longer copies memory. `/proc/meminfo` reports `CowShared` (each frame counted once).
- The runtime suite checks this: `fork_copy_on_write_isolation` verifies sharing while parent and
  child are alive, write isolation in both directions (including a kernel write into a shared
  page), and that `CowShared` drops back to 0.
- The owner map is a `BTreeMap` behind one lock. A per-frame atomic array sized from the memory map
  would be faster under SMP stage S2+; revisit with profiling.
- Huge pages (MEM-ARCH-01) are not shared COW; they do not occur in user mappings today.

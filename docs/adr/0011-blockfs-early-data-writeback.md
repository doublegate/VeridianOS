# ADR 0011: BlockFS writes file data back before sync

- Status: accepted (amends the dirty-block rule of ADR 0003)
- Date: 2026-10-14
- Tracking: BM-FS-01, N-118 / BM-DRV-06 (PERFORMANCE_AND_QUALITY_AUDIT_MATRIX 2026-10 intake)

## Context

ADR 0003 pins every dirty block in memory until `sync`. BlockFS has no journal: the superblock,
bitmap and inode table reach the disk only at `sync`, so the disk must hold exactly the last-synced
state between syncs. Writing a dirty block early was rejected because a block can be freed and
reallocated between syncs while the last-synced inodes still point at it; writing its new contents
would corrupt the old file if the system crashed before the next sync.

The cost is that dirty data is bounded only by how much is written between syncs, and nothing syncs
periodically. A large copy grows the cache past its capacity by the whole file.

An attempt on 2026-10-10 (another agent, not merged) made dirty slots evictable directly. That broke
exactly the rule above, dropped write errors, and deadlocked the read path; it was reverted
(archived in `/opt/veridian/archive/2026-10-10-antigravity/`).

## Decision

Make the corruption case impossible instead of pinning everything:

- **Quarantine.** With a disk attached, a freed block stays marked in the in-memory bitmap until a
  sync has written a bitmap without it. Sync writes that bitmap from a copy and releases the
  quarantine only after all of the metadata is written, so a failed sync leaves it in place.
  `free_blocks` counts quarantined blocks as free (they can still be allocated, see below).
- **File data may be written early.** A dirty block of a regular file's or symlink's contents is
  then either a block that belonged to the same file at the last sync (an in-place overwrite, which
  POSIX does not make atomic anyway) or a block that was free on the disk, which nothing on the disk
  refers to. Writing it cannot corrupt another file.
- **Metadata stays pinned.** Directory blocks and indirect-pointer blocks are written only at sync:
  an early write of either would let the last-synced inodes reach blocks the last-synced bitmap calls
  free. `block_mut` pins by default; only `data_block_mut` (file writes, truncation's tail zeroing)
  does not, and a pinned block stays pinned until the sync that writes it.
- **Reuse under pressure.** When no other block is free, a quarantined block is reused, and that
  block is pinned like metadata.
- **When.** After each modifying operation (`BlockFsNode::mutate`), never half-way through one: if
  more unpinned dirty blocks than the dirty limit (16 MiB on x86_64, 1 MiB elsewhere,
  `DEFAULT_CAPACITY_BLOCKS`) have accumulated, all of them are written and become clean and
  evictable. A write error leaves them dirty for the next sync to report.

Separately, a cache miss no longer holds the cache lock or the disk across the read (N-118): the miss
takes a slot out as a fill ticket, reads with nothing locked but the disk, and puts it back; readers of
the same block sleep on a wait queue.

## Consequences

- Unpinned dirty data is bounded by the dirty limit. Pinned data (metadata, and blocks reused from
  quarantine) is still bounded only by what changes between syncs.
- Space freed since the last sync cannot be reused for unpinned data until the next sync; under
  pressure it is reused pinned, so allocation does not fail because of the quarantine.
- A crash between syncs still mounts as the last-synced filesystem, with in-place overwrites of files
  that existed then possibly visible (as on ext4 with `data=writeback`). A crash *during* a sync is
  as unsafe as before; journaling or ordered metadata is still the long-term fix (v0.29
  crash-consistency work).
- Verification: host tests `early_writeback_keeps_the_last_synced_state_after_a_crash` (snapshot the
  disk after early write-back, mount it, the deleted file reads back intact; with the quarantine
  disabled it reads back the new file's bytes) and `reused_quarantined_blocks_stay_pinned_until_sync`,
  plus the cache tests for pinning, the dirty count and fill tickets.

## References

- ADR 0003 (BlockFS block cache)
- ext4 data modes: <https://docs.kernel.org/admin-guide/ext4.html> (`data=writeback`, `data=ordered`)

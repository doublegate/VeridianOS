# ADR 0003: BlockFS block cache

- Status: accepted
- Date: 2026-10-06
- Tracking: FS-PERF-01, DRV-PERF-02, audit verification N-29

## Context

`BlockFs::open_existing` read every allocated data block at mount, each into its own heap `Vec`,
alongside a `Vec<Vec<u8>>` header for every block of the filesystem.

- **x86_64:** for the 512 MiB root image (7,049 blocks in use of 131,072) that cost about 28 MiB of
  heap, 3 MiB of headers, and 3.5 s of I/O. Batching virtio-blk to one request per 4 KiB block
  (DRV-PERF-02) brought the I/O down to 0.74 s.
- **AArch64 and RISC-V:** these kernels have an 8 MiB heap behind a bump allocator that never frees
  (MEM-SEC-03). They could not mount the image at all. Until the virtio-mmio probe was fixed (N-29)
  they never found the disk, so this went unnoticed. Once the probe was fixed, the mount panicked with
  an allocation failure.

BlockFS writes no journal. The superblock, bitmap and inode table reach the disk only at `sync`,
after the dirty data blocks. Between syncs the disk therefore holds exactly the last-synced state, and
that is what makes a crash between syncs safe.

## Decision

- **Data blocks are read on first use** into a pool of 4 KiB slots (`fs/blockfs/cache.rs`). Mount
  reads only the superblock, bitmap and inode table.
- **Replacement is CLOCK (second chance),** an LRU approximation with O(1) work per access. A
  block-to-slot index (`Vec<u32>`, 4 bytes per filesystem block) replaces the per-block `Vec`
  headers.
- **Slots are recycled, never freed,** so the pool does not churn the allocator. This is required on
  the bump-allocated heaps.
- **Capacity bounds clean data:** 16 MiB on x86_64 and 1 MiB on AArch64/RISC-V
  (`DEFAULT_CAPACITY_BLOCKS`; `open_existing_with_cache` takes another size).
- **Dirty blocks stay in memory until `sync`.** Eviction takes only clean slots. When every slot is
  dirty, the pool grows past its capacity.

  Writing a dirty block back early to make room was rejected. A block can be freed and reallocated
  between syncs while the last-synced inodes on the disk still point at it, so writing its new
  contents early would corrupt the old file if the system crashed before the next sync. Pinning keeps
  the existing crash behaviour.
- **A newly allocated block is inserted zeroed and dirty,** so stale bytes on the disk are never read
  back through it. A freed block is dropped without write-back.
- **Reads take the cache's own lock** inside the filesystem's read lock, so lookups and reads can fill
  the cache without taking the filesystem write lock. Lock order: filesystem, cache, disk backend,
  driver.
- **The device size and read-only flag are sampled once** when the backend is attached.
- **`detach_disk_backend` was removed.** It had no callers, and with lazy loading it would have
  silently replaced every non-resident block with zeros.

## Consequences

Mount time for the 512 MiB root image, with virtio-blk batching already applied:

| Arch | Before | After |
|---|---|---|
| x86_64 (KVM) | 738 ms | 86 ms |
| AArch64 (TCG) | panic: out of memory | 1.26 s |
| RISC-V (TCG) | panic: out of memory | 4.3 s |

On AArch64 and RISC-V, mount time is now dominated by reading the 781-block inode table.

- **Memory:** clean data is bounded. Dirty data is bounded by how much is written between syncs, as
  before; nothing syncs periodically yet.
- **Read cost:** a read that misses costs one virtio request, at most 4 KiB, while holding the cache
  lock.
- **Verification:** host tests cover the cache rules. `lazy_cache_persists_through_remount` writes more
  data than the cache holds, syncs, remounts with a 4-block cache and compares contents, and checks
  that nothing reaches the disk between syncs. The x86_64 BusyBox suite and `audit_runtime_test` pass
  on the BlockFS root. AArch64 and RISC-V mount it, list it and read files from it.
- **Not done:**
  - Read-ahead.
  - A periodic writeback thread. That needs ordered or journaled metadata first, for the reason above.
  - Lazy inode-table loading.

## References

- [Corbató, "A Paging Experiment with the Multics System" (CLOCK)](https://dspace.mit.edu/handle/1721.1/102730)
- [QEMU `hw/arm/virt.c` virtio-mmio layout](https://github.com/qemu/qemu/blob/master/hw/arm/virt.c)
- [QEMU `hw/riscv/virt.c` virtio-mmio layout](https://github.com/qemu/qemu/blob/master/hw/riscv/virt.c)

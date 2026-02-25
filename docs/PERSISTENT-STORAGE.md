# Persistent Storage with BlockFS

**Last Updated**: February 24, 2026

## Overview

BlockFS is VeridianOS's native persistent filesystem. Unlike the default TAR rootfs mode -- which loads an archive into RAM at boot and loses all changes on shutdown -- BlockFS reads and writes directly to a raw disk image backed by virtio-blk. Files created, modified, or deleted during a session survive across reboots.

BlockFS is an ext2-inspired filesystem with a simple, predictable on-disk layout: a superblock, block bitmap, inode table, and data blocks. It supports regular files and directories with standard POSIX-style metadata (permissions, timestamps, link counts). The kernel auto-detects the filesystem format at boot by probing the first sector of the virtio-blk device for the BlockFS magic number.

### When to Use Each Mode

| Mode | Storage | Persistence | RAM | Use Case |
|------|---------|-------------|-----|----------|
| TAR rootfs | RAM only | None (volatile) | 256M | Quick testing, CI boots |
| BlockFS | Disk image | Full persistence | 2048M | Development, native compilation, iterative work |

---

## On-Disk Layout

BlockFS uses 4KB blocks throughout. The image is divided into four regions:

```text
+-------------------+  Block 0
|    Superblock      |  62 bytes serialized, padded to 4096
+-------------------+  Block 1
|   Block Bitmap     |  B blocks (B = ceil(total_blocks / 32768))
|                    |  1 bit per block: 0 = free, 1 = allocated
+-------------------+  Block 1+B
|   Inode Table      |  I blocks (I = ceil(inode_count * 96 / 4096))
|                    |  42 inodes per block (96 bytes each)
+-------------------+  Block 1+B+I
|   Data Blocks      |  Remaining blocks for file/directory content
|                    |
|        ...         |
+-------------------+
```

### Superblock (62 bytes)

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 4 | magic | `0x424C4B46` ("BLKF") |
| 4 | 4 | block_count | Total blocks in image |
| 8 | 4 | inode_count | Total inodes allocated |
| 12 | 4 | free_blocks | Free block count |
| 16 | 4 | free_inodes | Free inode count |
| 20 | 4 | first_data_block | Index of first data block |
| 24 | 4 | block_size | Always 4096 |
| 28 | 2 | inode_size | Always 96 |
| 30 | 4 | blocks_per_group | Block group size (8192) |
| 34 | 4 | inodes_per_group | Inodes per group (2048) |
| 38 | 8 | mount_time | Last mount timestamp |
| 46 | 8 | write_time | Last write timestamp |
| 54 | 2 | mount_count | Mount counter |
| 56 | 2 | max_mount_count | Max mounts before check (100) |
| 58 | 2 | state | Filesystem state (1 = clean) |
| 60 | 2 | errors | Error behavior flags |

All fields are little-endian.

### Inode (96 bytes)

Each inode stores file metadata and block pointers:

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 2 | mode | File type + permissions (e.g., `0x81ED` = regular rwxr-xr-x) |
| 2 | 2 | uid | Owner user ID |
| 4 | 4 | size | File size in bytes |
| 8 | 4 | atime | Access time |
| 12 | 4 | ctime | Creation time |
| 16 | 4 | mtime | Modification time |
| 20 | 4 | dtime | Deletion time |
| 24 | 2 | gid | Owner group ID |
| 26 | 2 | links_count | Hard link count |
| 28 | 4 | blocks | Allocated block count |
| 32 | 4 | flags | Inode flags |
| 36 | 48 | direct_blocks[12] | 12 direct block pointers (4 bytes each) |
| 84 | 4 | indirect_block | Single indirect block pointer |
| 88 | 4 | double_indirect_block | Double indirect block pointer |
| 92 | 4 | triple_indirect_block | Triple indirect block pointer |

With 12 direct blocks + single indirect (1024 pointers), a single file can be up to `(12 + 1024) * 4096 = 4,243,456 bytes` (~4MB) without double indirect blocks.

### Directory Entries

Directories store variable-length entries in their data blocks:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 4 | inode number |
| 4 | 2 | record length (aligned to 4 bytes) |
| 6 | 1 | name length |
| 7 | 1 | file type (1=regular, 2=directory) |
| 8 | N | filename (up to 255 bytes) |

---

## Image Creation

### Prerequisites

Build the `mkfs-blockfs` host tool (runs on Linux, not on VeridianOS):

```bash
cd tools/mkfs-blockfs
cargo build --release
```

Build the cross-compiled BusyBox rootfs (required to populate the image):

```bash
./scripts/build-busybox-rootfs.sh all
```

### Creating a BlockFS Image

The simplest way is to use the rootfs build script:

```bash
# Create a 256MB BlockFS image populated from the BusyBox rootfs
./scripts/build-busybox-rootfs.sh blockfs
```

This runs `mkfs-blockfs` internally with the default 256MB size and populates the image from `target/rootfs-busybox/`.

For manual control, use `mkfs-blockfs` directly:

```bash
# Create a 256MB image populated from a directory
./tools/mkfs-blockfs/target/x86_64-unknown-linux-gnu/release/mkfs-blockfs \
    --output target/rootfs-blockfs.img \
    --size 256 \
    --populate target/rootfs-busybox/

# Create an empty 64MB image (no pre-populated files)
./tools/mkfs-blockfs/target/x86_64-unknown-linux-gnu/release/mkfs-blockfs \
    --output target/empty.img \
    --size 64

# Specify a custom inode count
./tools/mkfs-blockfs/target/x86_64-unknown-linux-gnu/release/mkfs-blockfs \
    --output target/rootfs-blockfs.img \
    --size 256 \
    --inodes 8192 \
    --populate target/rootfs-busybox/
```

### Sizing Recommendations

| Use Case | Image Size | RAM | Notes |
|----------|-----------|-----|-------|
| Quick testing | 64MB | 2048M | Minimal rootfs, limited scratch space |
| Development (default) | 256MB | 2048M | Full rootfs + native compilation output |
| Heavy compilation | 512MB | 2048M | Large build trees, multiple packages |

The 256MB default provides approximately 190MB of usable data space after metadata overhead (superblock, bitmap, inode table). This is sufficient for the full BusyBox rootfs (~58MB unpacked), GCC/binutils/make/ninja toolchain, and substantial scratch space for native compilation.

The `--size` flag accepts values in megabytes. You can override the default with the `BLOCKFS_SIZE` environment variable when using the build script:

```bash
BLOCKFS_SIZE=512 ./scripts/build-busybox-rootfs.sh blockfs
```

---

## Booting with Persistent Storage

### Using the Convenience Script

```bash
# Build the kernel (if not already built)
./build-kernel.sh x86_64 dev

# Boot with BlockFS (serial only)
./scripts/run-veridian.sh --blockfs

# Boot with BlockFS and framebuffer display
./scripts/run-veridian.sh --blockfs --display
```

### Manual QEMU Command

```bash
qemu-system-x86_64 -enable-kvm \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF.4m.fd \
    -drive id=disk0,if=none,format=raw,file=target/x86_64-veridian/debug/veridian-uefi.img \
    -device ide-hd,drive=disk0 \
    -drive file=target/rootfs-blockfs.img,if=none,id=vd0,format=raw \
    -device virtio-blk-pci,drive=vd0 \
    -serial stdio -display none -m 2048M
```

Key differences from TAR mode:
- The rootfs drive points to `target/rootfs-blockfs.img` instead of `target/rootfs-busybox.tar`
- RAM is 2048M (needed for the 512MB kernel heap plus user-space frame headroom during native compilation)

### Verifying the Boot Mode

On boot, the kernel prints one of:

```
[ROOTFS] BlockFS magic detected -- mounting persistent root
```

or (for TAR mode):

```
[ROOTFS] Loading TAR rootfs from virtio-blk...
```

---

## Verifying Persistence

To confirm that changes persist across reboots:

1. Boot with BlockFS:
   ```bash
   ./scripts/run-veridian.sh --blockfs
   ```

2. Create a test file at the shell prompt:
   ```
   root@veridian:/# echo "persistence test" > /tmp/test.txt
   root@veridian:/# sync
   ```

3. Shut down (Ctrl+C or `poweroff`).

4. Boot again:
   ```bash
   ./scripts/run-veridian.sh --blockfs
   ```

5. Read the file:
   ```
   root@veridian:/# cat /tmp/test.txt
   persistence test
   ```

If the file contents survive the reboot, persistence is working correctly.

---

## Sync and Fsync

BlockFS buffers writes in kernel memory and flushes them to disk via the `sync` and `fsync` system calls:

- **`sync`** -- Flushes all dirty BlockFS metadata (superblock, bitmap, inode table) and data blocks to the virtio-blk device. Available as the `sync` shell command and `SYS_SYNC` syscall.

- **`fsync(fd)`** -- Flushes all pending writes for a specific open file descriptor. Available as the `SYS_FSYNC` syscall.

The kernel also performs a sync during clean shutdown. However, if QEMU is terminated abruptly (e.g., `kill -9`), unflushed data may be lost. Always run `sync` before shutting down if you have important unflushed writes.

---

## Auto-Detection Mechanism

The kernel determines the rootfs format at boot by probing the first block (sector 0) of the virtio-blk device:

1. Read the first 4096 bytes from the virtio-blk device.
2. Extract the first 4 bytes as a little-endian `u32`.
3. Compare against `BLOCKFS_MAGIC` (`0x424C4B46`, ASCII "BLKF").
4. If the magic matches, mount as BlockFS (persistent root).
5. Otherwise, treat the entire device as a TAR archive and extract into RamFS.

This logic lives in `kernel/src/bootstrap.rs` (`load_rootfs_from_virtio()`). No user configuration is needed -- the kernel automatically selects the correct handler based on the image content.

---

## Troubleshooting

### "BlockFS image not found"

The convenience script (`run-veridian.sh --blockfs`) expects the image at `target/rootfs-blockfs.img`. Create it with:

```bash
./scripts/build-busybox-rootfs.sh all      # Build rootfs directory
./scripts/build-busybox-rootfs.sh blockfs   # Create BlockFS image
```

### "Failed to get write lock" (QEMU)

A previous QEMU instance is still holding the disk image lock. Kill it:

```bash
pkill -9 -f qemu-system
sleep 2
```

Then retry. The convenience script does this automatically.

### Boot hangs or no serial output

Ensure OVMF firmware is installed and the path is correct. The convenience script auto-detects it. For manual QEMU commands, verify the `-drive if=pflash,...` path matches your distribution:

| Distribution | OVMF Path |
|-------------|-----------|
| CachyOS / Arch | `/usr/share/edk2/x64/OVMF.4m.fd` |
| Ubuntu / Debian | `/usr/share/OVMF/OVMF_CODE.fd` |
| Fedora | `/usr/share/edk2/ovmf/OVMF_CODE.fd` |

### Filesystem corruption after abrupt termination

If the image becomes corrupted (e.g., kernel panic during write, QEMU killed without sync), recreate it:

```bash
./scripts/build-busybox-rootfs.sh blockfs
```

There is currently no `fsck`-equivalent for BlockFS. Rebuilding the image from the host rootfs directory is the recommended recovery path.

### Out of inodes or blocks

The default configuration provides generous inode counts (1 inode per 16KB of image space). If you run out of inodes with many small files, specify a higher count:

```bash
./tools/mkfs-blockfs/target/x86_64-unknown-linux-gnu/release/mkfs-blockfs \
    --output target/rootfs-blockfs.img \
    --size 256 \
    --inodes 16384 \
    --populate target/rootfs-busybox/
```

For running out of data blocks, increase the image size with `--size`.

---

## Architecture Reference

### Key Source Files

| File | Purpose |
|------|---------|
| `kernel/src/fs/blockfs.rs` | BlockFS kernel driver (mount, read, write, sync) |
| `kernel/src/bootstrap.rs` | Auto-detection and rootfs mounting at boot |
| `kernel/src/syscall/filesystem.rs` | `sys_sync()` and `sys_fsync()` syscall implementations |
| `tools/mkfs-blockfs/src/main.rs` | Host-side image creation tool |
| `scripts/build-busybox-rootfs.sh` | Build script with `blockfs` phase |
| `scripts/run-veridian.sh` | Convenience QEMU launcher with `--blockfs` flag |

### Related Documentation

- [BUILD-INSTRUCTIONS.md](BUILD-INSTRUCTIONS.md) -- Full build workflow including persistent storage
- [SELF-HOSTING-STATUS.md](SELF-HOSTING-STATUS.md) -- Self-hosting toolchain tiers
- [SOFTWARE-PORTING-GUIDE.md](SOFTWARE-PORTING-GUIDE.md) -- Porting software to VeridianOS

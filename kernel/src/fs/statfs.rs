//! What statfs(2) and fstatfs(2) report about the filesystem holding a file
//! (`VfsNode::statfs`), and the `struct statfs` they write.
//!
//! The magic numbers are Linux's (`include/uapi/linux/magic.h`), so programs
//! that tell filesystems apart by `f_type` (LTP's `tst_fs_type`, coreutils'
//! `stat -f`, glibc's `statvfs` for `/dev/shm`) see what they expect. BlockFS
//! has no Linux counterpart and reports its own.

/// tmpfs, and the memory-backed files Linux keeps on an internal tmpfs
/// (memfd, devtmpfs).
pub const TMPFS_MAGIC: u64 = 0x0102_1994;
pub const RAMFS_MAGIC: u64 = 0x8584_58f6;
pub const PROC_SUPER_MAGIC: u64 = 0x9fa0;
pub const EXT4_SUPER_MAGIC: u64 = 0xEF53;
pub const MSDOS_SUPER_MAGIC: u64 = 0x4d44;
pub const DEVPTS_SUPER_MAGIC: u64 = 0x1cd1;
pub const PIPEFS_MAGIC: u64 = 0x5049_5045;
/// pidfs, the filesystem of pidfds ("PIDF").
pub const PID_FS_MAGIC: u64 = 0x5049_4446;
/// nsfs (namespace files).
pub const NSFS_MAGIC: u64 = 0x6e73_6673;
pub const SOCKFS_MAGIC: u64 = 0x534F_434B;
/// eventfd, signalfd, timerfd and epoll: Linux's anonymous inodes.
pub const ANON_INODE_FS_MAGIC: u64 = 0x0904_1934;
/// BlockFS, VeridianOS's own disk filesystem ("BLKF").
pub const BLOCKFS_MAGIC: u64 = 0x424C_4B46;

/// `f_flags`: mounted read-only.
pub const ST_RDONLY: u64 = 1;
/// `f_flags` is filled in (Linux sets it on every statfs since 2.6.36).
pub const ST_VALID: u64 = 0x20;

/// The `f_flags` a mount with flags `mnt` (`fs::mount::flags`) and a
/// superblock read-only or not adds, as Linux's calculate_f_flags: ST_RDONLY
/// 1, ST_NOSUID 2, ST_NODEV 4, ST_NOEXEC 8, ST_NOATIME 1024, ST_NODIRATIME
/// 2048, ST_RELATIME 4096 and ST_NOSYMFOLLOW 0x2000.
pub fn mount_flags(mnt: u32, sb_readonly: bool) -> u64 {
    use super::mount::flags as m;
    let mut out = if sb_readonly { ST_RDONLY } else { 0 };
    for (flag, st) in [
        (m::READONLY, ST_RDONLY),
        (m::NOSUID, 2),
        (m::NODEV, 4),
        (m::NOEXEC, 8),
        (m::NOATIME, 1024),
        (m::NODIRATIME, 2048),
        (m::RELATIME, 4096),
        (m::NOSYMFOLLOW, 0x2000),
    ] {
        if mnt & flag != 0 {
            out |= st;
        }
    }
    out
}

/// Longest file name, for filesystems with no limit of their own
/// (Linux's `NAME_MAX`).
pub const NAME_MAX: u64 = 255;

/// Size of `struct statfs` on x86_64, arm64 and riscv64: every field is a
/// 64-bit word except `f_fsid`, two 32-bit words.
pub const LINUX_STATFS_SIZE: usize = 120;

/// One filesystem's statistics, Linux's `struct kstatfs`. Counts are in
/// units of `block_size` (blocks) and of files (inodes); a filesystem with
/// no limit reports 0, as Linux's `simple_statfs` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FsStats {
    pub fs_type: u64,
    pub block_size: u64,
    pub blocks: u64,
    pub blocks_free: u64,
    /// Free blocks an unprivileged user may take (less than `blocks_free`
    /// where the filesystem reserves some for root).
    pub blocks_avail: u64,
    pub files: u64,
    pub files_free: u64,
    pub fsid: [u32; 2],
    pub name_max: u64,
    /// 0 means `block_size`, which statfs then reports.
    pub fragment_size: u64,
    /// `ST_*` flags the filesystem itself knows (`ST_RDONLY`); statfs adds
    /// `ST_VALID`.
    pub flags: u64,
}

impl FsStats {
    /// What Linux's `simple_statfs` reports for a filesystem with no
    /// counts: the type, a page-sized block and `NAME_MAX`.
    pub const fn simple(fs_type: u64) -> Self {
        Self {
            fs_type,
            block_size: 4096,
            blocks: 0,
            blocks_free: 0,
            blocks_avail: 0,
            files: 0,
            files_free: 0,
            fsid: [0, 0],
            name_max: NAME_MAX,
            fragment_size: 0,
            flags: 0,
        }
    }

    /// The `struct statfs` that statfs and fstatfs copy out: what
    /// `statfs_by_dentry` and `do_statfs_native` make of these values
    /// (`fs/statfs.c`): `f_frsize` defaults to `f_bsize`, and `f_flags`
    /// always carries `ST_VALID`.
    pub fn to_linux(&self) -> [u8; LINUX_STATFS_SIZE] {
        let frsize = if self.fragment_size == 0 {
            self.block_size
        } else {
            self.fragment_size
        };
        let words = [
            self.fs_type,
            self.block_size,
            self.blocks,
            self.blocks_free,
            self.blocks_avail,
            self.files,
            self.files_free,
        ];
        let mut out = [0u8; LINUX_STATFS_SIZE];
        for (i, w) in words.iter().enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(&w.to_ne_bytes());
        }
        out[56..60].copy_from_slice(&self.fsid[0].to_ne_bytes());
        out[60..64].copy_from_slice(&self.fsid[1].to_ne_bytes());
        for (i, w) in [self.name_max, frsize, self.flags | ST_VALID]
            .iter()
            .enumerate()
        {
            out[64 + i * 8..72 + i * 8].copy_from_slice(&w.to_ne_bytes());
        }
        // f_spare[4] (88..120) stays zero.
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(buf: &[u8], at: usize) -> u64 {
        u64::from_ne_bytes(buf[at..at + 8].try_into().unwrap())
    }

    #[test]
    fn linux_layout_puts_each_field_at_its_offset() {
        let st = FsStats {
            fs_type: TMPFS_MAGIC,
            block_size: 4096,
            blocks: 100,
            blocks_free: 60,
            blocks_avail: 50,
            files: 30,
            files_free: 20,
            fsid: [0x1122_3344, 0x5566_7788],
            name_max: 255,
            fragment_size: 0,
            flags: ST_RDONLY,
        };
        let b = st.to_linux();
        assert_eq!(
            [0, 8, 16, 24, 32, 40, 48].map(|o| word(&b, o)),
            [TMPFS_MAGIC, 4096, 100, 60, 50, 30, 20]
        );
        assert_eq!(
            u32::from_ne_bytes(b[56..60].try_into().unwrap()),
            0x1122_3344
        );
        assert_eq!(
            u32::from_ne_bytes(b[60..64].try_into().unwrap()),
            0x5566_7788
        );
        assert_eq!(word(&b, 64), 255);
        // f_frsize defaults to f_bsize; f_flags always has ST_VALID.
        assert_eq!(word(&b, 72), 4096);
        assert_eq!(word(&b, 80), ST_RDONLY | ST_VALID);
        assert!(b[88..].iter().all(|&x| x == 0));
    }

    #[test]
    fn mount_flags_are_linux_st_flags() {
        use crate::fs::mount::flags as m;
        assert_eq!(mount_flags(m::RELATIME, false), 4096);
        assert_eq!(mount_flags(0, true), ST_RDONLY);
        assert_eq!(
            mount_flags(m::READONLY | m::NOSUID | m::NODEV | m::NOEXEC, false),
            ST_RDONLY | 2 | 4 | 8
        );
        assert_eq!(
            mount_flags(m::NOATIME | m::NODIRATIME | m::NOSYMFOLLOW, false),
            1024 | 2048 | 0x2000
        );
    }

    #[test]
    fn simple_statfs_reports_a_page_block_and_no_counts() {
        let b = FsStats::simple(PROC_SUPER_MAGIC).to_linux();
        assert_eq!(word(&b, 0), PROC_SUPER_MAGIC);
        assert_eq!(word(&b, 8), 4096);
        assert!([16, 24, 32, 40, 48].iter().all(|&o| word(&b, o) == 0));
        assert_eq!(word(&b, 64), NAME_MAX);
        assert_eq!(word(&b, 80), ST_VALID);
    }
}

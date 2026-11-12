//! System V shared memory (`ipc/shm.c`): shmget, shmat, shmdt, shmctl.
//!
//! A segment's pages are a memfd's (`fs::memfd`, on `fs::shmem` frames), so
//! an attachment is a MAP_SHARED mapping of them, as Linux maps a shmem
//! file. Each piece of an attachment holds a [`crate::mm::vas::MappingHook`]
//! on its [`Attach`], so a split, a fork's copy, munmap, exit and exec keep
//! `shm_nattch` as Linux's shm_open and shm_close do.

use alloc::sync::{Arc, Weak};

use super::{
    encode_perm, ipcget, now_secs, pid_shown, put_long, Asker, GetOutcome, Ids, IpcObject, Perm,
    SysvSpace, IPC64_PERM_SIZE, IPC_INFO, IPC_RMID, IPC_SET, IPC_STAT,
};
use crate::{
    fs::{memfd::MemfdNode, Permissions, VfsNode},
    mm::vas::{MappingHook, MappingOwner},
    syscall::{SyscallError, SyscallResult},
};

/// Linux's limits (`include/uapi/linux/shm.h`).
pub const SHMMIN: usize = 1;
pub const SHMMNI: i32 = 4096;
pub const SHMMAX: u64 = u64::MAX - (1 << 24);
pub const SHMALL: u64 = u64::MAX - (1 << 24);
/// Attachments go on this boundary (a page on every architecture here).
pub const SHMLBA: usize = 4096;
const PAGE: usize = 4096;

/// shmget flags.
pub const SHM_HUGETLB: i32 = 0o4000;
/// shmat flags.
pub const SHM_RDONLY: i32 = 0o10000;
pub const SHM_RND: i32 = 0o20000;
pub const SHM_REMAP: i32 = 0o40000;
pub const SHM_EXEC: i32 = 0o100000;
/// Mode bits the kernel sets.
pub const SHM_DEST: u32 = 0o1000;
pub const SHM_LOCKED: u32 = 0o2000;
/// shmctl commands.
pub const SHM_LOCK: i32 = 11;
pub const SHM_UNLOCK: i32 = 12;
pub const SHM_STAT: i32 = 13;
pub const SHM_INFO: i32 = 14;
pub const SHM_STAT_ANY: i32 = 15;

/// A segment (`struct shmid_kernel`).
pub struct Segment {
    perm: Perm,
    pub segsz: usize,
    pub atime: i64,
    pub dtime: i64,
    pub ctime: i64,
    /// Global pids of the creator and of the last to attach or detach.
    pub cpid: u64,
    pub lpid: u64,
    pub nattch: usize,
    /// The creator, until it exits (`shm_creator`; for shm_rmid_forced).
    pub creator: Option<u64>,
    pub node: Arc<MemfdNode>,
}

impl IpcObject for Segment {
    fn perm(&self) -> &Perm {
        &self.perm
    }
    fn perm_mut(&mut self) -> &mut Perm {
        &mut self.perm
    }
}

impl Segment {
    fn pages(&self) -> usize {
        self.segsz.div_ceil(PAGE)
    }
}

/// One namespace's segments and the pages they hold (`shm_tot`).
pub struct ShmTable {
    pub ids: Ids<Segment>,
    pub tot_pages: u64,
}

impl ShmTable {
    pub const fn new() -> Self {
        Self {
            ids: Ids::new(),
            tot_pages: 0,
        }
    }

    /// Destroy `id` (shm_destroy): out of the table, its pages given back
    /// to the namespace's total. Mappings still holding frames keep them.
    fn destroy(&mut self, id: i32) {
        if let Some(seg) = self.ids.remove(id) {
            self.tot_pages = self.tot_pages.saturating_sub(seg.pages() as u64);
        }
    }

    /// shm_may_destroy, then destroy: no attachment left, and removed
    /// (SHM_DEST) or shm_rmid_forced set.
    fn destroy_if_unused(&mut self, id: i32, rmid_forced: bool) {
        let unused = self
            .ids
            .get(id)
            .is_ok_and(|s| s.nattch == 0 && (rmid_forced || s.perm.mode & SHM_DEST != 0));
        if unused {
            self.destroy(id);
        }
    }
}

impl Default for ShmTable {
    fn default() -> Self {
        Self::new()
    }
}

/// shmget's checks and creation, on the namespace's table.
pub fn get(space: &SysvSpace, asker: &Asker, key: i32, size: usize, flags: i32) -> SyscallResult {
    let mut table = space.shm.lock();
    match ipcget(&table.ids, asker, key, flags, |s: &Segment| {
        if size > s.segsz {
            Err(SyscallError::InvalidArgument)
        } else {
            Ok(())
        }
    })? {
        GetOutcome::Existing(id) => Ok(id as usize),
        GetOutcome::Create => {
            asker.may_make()?;
            let limits = &space.limits;
            if size < SHMMIN
                || size as u64 > limits.shmmax.load(core::sync::atomic::Ordering::Relaxed)
            {
                return Err(SyscallError::InvalidArgument);
            }
            // No huge pages (CONFIG_HUGETLBFS is unset): no hstate for any
            // size, EINVAL as newseg says.
            if flags & SHM_HUGETLB != 0 {
                return Err(SyscallError::InvalidArgument);
            }
            // newseg: a size whose whole pages do not fit the address
            // arithmetic is ENOSPC (`numpages << PAGE_SHIFT < size`); with
            // shmmax raised near the top, `size` can be (PR #40 review).
            let Some(bytes) = size.checked_next_multiple_of(PAGE) else {
                return Err(SyscallError::NoSpace);
            };
            let pages = (bytes / PAGE) as u64;
            let shmall = limits.shmall.load(core::sync::atomic::Ordering::Relaxed);
            match table.tot_pages.checked_add(pages) {
                Some(t) if t <= shmall => {}
                _ => return Err(SyscallError::NoSpace),
            }
            let mni = limits.shmmni.load(core::sync::atomic::Ordering::Relaxed);
            if table.ids.in_use() >= mni.max(0) as usize {
                return Err(SyscallError::NoSpace);
            }
            let node = MemfdNode::new(
                Permissions::from_mode(flags as u32 & super::MODE_MASK),
                asker.euid,
                asker.egid,
                false,
                0,
            );
            node.truncate(bytes)
                .map_err(|_| SyscallError::OutOfMemory)?;
            let pid = super::current_pid();
            let seg = Segment {
                perm: Perm::new(key, flags, asker.euid, asker.egid),
                segsz: size,
                atime: 0,
                dtime: 0,
                ctime: now_secs(),
                cpid: pid,
                lpid: 0,
                nattch: 0,
                creator: Some(pid),
                node,
            };
            let id = table.ids.add(seg, mni.max(0) as usize)?;
            table.tot_pages += pages;
            Ok(id as usize)
        }
    }
}

/// An attachment's owner: tells the segment as pieces come and go.
pub struct Attach {
    space: Weak<SysvSpace>,
    id: i32,
}

impl Attach {
    fn with_segment(&self, f: impl FnOnce(&mut ShmTable, &SysvSpace)) {
        if let Some(space) = self.space.upgrade() {
            let mut table = space.shm.lock();
            f(&mut table, &space);
        }
    }
}

impl MappingOwner for Attach {
    /// shm_open: one more attachment; the attach time and pid.
    fn opened(&self) {
        let id = self.id;
        self.with_segment(|table, _| {
            if let Ok(seg) = table.ids.get_mut(id) {
                seg.nattch += 1;
                seg.atime = now_secs();
                seg.lpid = super::current_pid();
            }
        });
    }

    /// shm_close: one fewer; the detach time and pid; destroyed when
    /// nothing is attached and it was removed (or shm_rmid_forced).
    fn closed(&self) {
        let id = self.id;
        self.with_segment(|table, space| {
            if let Ok(seg) = table.ids.get_mut(id) {
                seg.nattch = seg.nattch.saturating_sub(1);
                seg.dtime = now_secs();
                seg.lpid = super::current_pid();
            }
            let forced = space
                .limits
                .shm_rmid_forced
                .load(core::sync::atomic::Ordering::Relaxed)
                != 0;
            table.destroy_if_unused(id, forced);
        });
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// What shmat maps: the checked request.
pub struct AttachRequest {
    /// Where (`None`: the kernel chooses), already rounded for SHM_RND.
    pub at: Option<usize>,
    pub prot: usize,
    pub remap: bool,
}

/// shmat's checks of the address and flags, before the segment is looked
/// up (do_shmat): a misaligned address without SHM_RND, SHM_RND rounding
/// to 0 with SHM_REMAP, or SHM_REMAP without an address, is EINVAL.
pub fn attach_request(id: i32, addr: usize, flags: i32) -> Result<AttachRequest, SyscallError> {
    if id < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let mut at = None;
    if addr != 0 {
        let mut a = addr;
        if a & (SHMLBA - 1) != 0 {
            if flags & SHM_RND != 0 {
                a &= !(SHMLBA - 1);
                if a == 0 && flags & SHM_REMAP != 0 {
                    return Err(SyscallError::InvalidArgument);
                }
            } else {
                return Err(SyscallError::InvalidArgument);
            }
        }
        at = Some(a);
    } else if flags & SHM_REMAP != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    const PROT_READ: usize = 1;
    const PROT_WRITE: usize = 2;
    const PROT_EXEC: usize = 4;
    let mut prot = if flags & SHM_RDONLY != 0 {
        PROT_READ
    } else {
        PROT_READ | PROT_WRITE
    };
    if flags & SHM_EXEC != 0 {
        prot |= PROT_EXEC;
    }
    Ok(AttachRequest {
        at,
        prot,
        remap: flags & SHM_REMAP != 0,
    })
}

/// The access bits shmat asks for `prot` (S_IRUGO, S_IWUGO, S_IXUGO).
fn access_of(prot: usize) -> u32 {
    let mut acc = 0o444;
    if prot & 2 != 0 {
        acc |= 0o222;
    }
    if prot & 4 != 0 {
        acc |= 0o111;
    }
    acc
}

/// shmat: map segment `id` of the caller's namespace.
pub fn attach(
    space: &Arc<SysvSpace>,
    asker: &Asker,
    id: i32,
    addr: usize,
    flags: i32,
) -> SyscallResult {
    let req = attach_request(id, addr, flags)?;
    // Look up and check, and hold the segment with one attachment while it
    // is mapped (do_shmat's shm_nattch++), handed to the mapping's hook.
    let (node, size) = {
        let mut table = space.shm.lock();
        let seg = table.ids.get_mut(id)?;
        if !asker.permits(&seg.perm, access_of(req.prot)) {
            return Err(SyscallError::PermissionDenied);
        }
        seg.nattch += 1;
        seg.atime = now_secs();
        seg.lpid = super::current_pid();
        (seg.node.clone(), seg.pages() * PAGE)
    };
    let owner: Arc<dyn MappingOwner> = Arc::new(Attach {
        space: Arc::downgrade(space),
        id,
    });
    // Dropped on any failure below, which takes the attachment back.
    let hook = MappingHook::adopt(owner);
    super::super::memory::map_hooked_shared(
        node,
        size,
        req.at,
        req.prot,
        req.remap,
        flags & SHM_RDONLY == 0,
        hook,
    )
}

/// shmdt: detach the attachment that starts at `addr` -- its first piece,
/// the one whose distance from `addr` is its offset in the segment, and
/// every later piece of the same segment within the segment's size from
/// `addr` (ksys_shmdt). EINVAL for a misaligned address or none there.
pub fn detach(addr: usize) -> SyscallResult {
    if addr & (PAGE - 1) != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let pieces = proc.memory_space.lock().hooked_mappings();
    let addr = addr as u64;
    let attach_of = |o: &Arc<dyn MappingOwner>| {
        o.as_any()
            .downcast_ref::<Attach>()
            .map(|a| (a.space.as_ptr(), a.id))
    };
    let Some(first) = pieces.iter().find(|(start, _, offset, owner)| {
        *start >= addr && *start - addr == *offset as u64 && attach_of(owner).is_some()
    }) else {
        return Err(SyscallError::InvalidArgument);
    };
    let which = attach_of(&first.3);
    // The segment's size bounds what goes (its node's, page-rounded).
    let size = first
        .3
        .as_any()
        .downcast_ref::<Attach>()
        .and_then(|a| {
            let space = a.space.upgrade()?;
            let table = space.shm.lock();
            table.ids.get(a.id).ok().map(|s| s.pages() * PAGE)
        })
        .unwrap_or(first.1 + first.2) as u64;
    let doomed: alloc::vec::Vec<(u64, usize)> = pieces
        .iter()
        .filter(|(start, len, offset, owner)| {
            *start == first.0
                || (*start > first.0
                    && *start + *len as u64 - addr <= size
                    && *start - addr == *offset as u64
                    && attach_of(owner) == which)
        })
        .map(|(start, len, _, _)| (*start, *len))
        .collect();
    drop(pieces);
    let vas = proc.memory_space.lock();
    for (start, len) in doomed {
        vas.unmap(start as usize, len)
            .map_err(|_| SyscallError::InvalidArgument)?;
    }
    Ok(0)
}

/// `struct shmid64_ds` (the same on every 64-bit architecture).
pub const SHMID_DS_SIZE: usize = 112;
/// `struct shminfo64`.
pub const SHMINFO_SIZE: usize = 72;
/// `struct shm_info`.
pub const SHM_INFO_SIZE: usize = 48;

fn encode_ds(asker: &Asker, seg: &Segment) -> [u8; SHMID_DS_SIZE] {
    let mut out = [0u8; SHMID_DS_SIZE];
    encode_perm(&asker.shown(&seg.perm), &mut out[..IPC64_PERM_SIZE]);
    put_long(&mut out, 48, seg.segsz as u64);
    put_long(&mut out, 56, seg.atime as u64);
    put_long(&mut out, 64, seg.dtime as u64);
    put_long(&mut out, 72, seg.ctime as u64);
    out[80..84].copy_from_slice(&pid_shown(seg.cpid).to_ne_bytes());
    out[84..88].copy_from_slice(&pid_shown(seg.lpid).to_ne_bytes());
    put_long(&mut out, 88, seg.nattch as u64);
    out
}

/// What shmctl writes back to the caller, if anything.
pub enum CtlOut {
    None,
    Bytes(alloc::vec::Vec<u8>),
}

/// shmctl on the caller's namespace. `set` is the caller's `shmid_ds` for
/// IPC_SET (its perm's uid, gid and mode). `memlock` is RLIMIT_MEMLOCK's
/// soft limit for SHM_LOCK. Returns the result and what to copy out.
pub fn control(
    space: &SysvSpace,
    asker: &Asker,
    id: i32,
    cmd: i32,
    set: Option<(u32, u32, u32)>,
    memlock: u64,
) -> Result<(usize, CtlOut), SyscallError> {
    // The commands that look without an object's own permission check
    // need READ on the namespace, as every look does (ADR 0027; PR #40
    // review).
    if matches!(cmd, IPC_INFO | SHM_INFO | SHM_STAT_ANY) && !asker.may_read {
        return Err(SyscallError::PermissionDenied);
    }
    let limits = &space.limits;
    let load = core::sync::atomic::Ordering::Relaxed;
    let mut table = space.shm.lock();
    match cmd {
        IPC_INFO => {
            let mut out = alloc::vec![0u8; SHMINFO_SIZE];
            let mni = limits.shmmni.load(load) as u64;
            put_long(&mut out, 0, limits.shmmax.load(load));
            put_long(&mut out, 8, SHMMIN as u64);
            put_long(&mut out, 16, mni);
            put_long(&mut out, 24, mni);
            put_long(&mut out, 32, limits.shmall.load(load));
            Ok((table.ids.max_idx() as usize, CtlOut::Bytes(out)))
        }
        SHM_INFO => {
            let mut out = alloc::vec![0u8; SHM_INFO_SIZE];
            out[0..4].copy_from_slice(&(table.ids.in_use() as i32).to_ne_bytes());
            put_long(&mut out, 8, table.tot_pages);
            // Every page is resident: no swap (CONFIG_SWAP is unset).
            put_long(&mut out, 16, table.tot_pages);
            Ok((table.ids.max_idx() as usize, CtlOut::Bytes(out)))
        }
        IPC_STAT | SHM_STAT | SHM_STAT_ANY => {
            let seg = if cmd == IPC_STAT {
                table.ids.get(id)?
            } else {
                table.ids.by_index(id)?
            };
            if cmd != SHM_STAT_ANY && !asker.permits(&seg.perm, 0o444) {
                return Err(SyscallError::PermissionDenied);
            }
            let ret = if cmd == IPC_STAT {
                0
            } else {
                seg.perm.id as usize
            };
            Ok((ret, CtlOut::Bytes(encode_ds(asker, seg).to_vec())))
        }
        SHM_LOCK | SHM_UNLOCK => {
            let seg = table.ids.get_mut(id)?;
            if !asker.may_modify {
                return Err(SyscallError::OperationNotPermitted);
            }
            if !asker.ipc_lock {
                if asker.euid != seg.perm.uid && asker.euid != seg.perm.cuid {
                    return Err(SyscallError::OperationNotPermitted);
                }
                if cmd == SHM_LOCK && memlock == 0 {
                    return Err(SyscallError::OperationNotPermitted);
                }
            }
            // Nothing is ever swapped out: locking only records it.
            if cmd == SHM_LOCK {
                seg.perm.mode |= SHM_LOCKED;
            } else {
                seg.perm.mode &= !SHM_LOCKED;
            }
            Ok((0, CtlOut::None))
        }
        IPC_RMID | IPC_SET => {
            let seg = table.ids.get_mut(id)?;
            if !asker.may_control(&seg.perm) {
                return Err(SyscallError::OperationNotPermitted);
            }
            if cmd == IPC_SET {
                let (uid, gid, mode) = set.ok_or(SyscallError::InvalidPointer)?;
                asker.update(&mut seg.perm, uid, gid, mode)?;
                seg.ctime = now_secs();
                return Ok((0, CtlOut::None));
            }
            // do_shm_rmid: destroyed now with nothing attached, else once
            // the last attachment goes; its key is free at once.
            if seg.nattch > 0 {
                seg.perm.mode |= SHM_DEST;
                table.ids.make_private(id);
            } else {
                table.destroy(id);
            }
            Ok((0, CtlOut::None))
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// exit_shm for process `pid`: its segments lose their creator, and with
/// shm_rmid_forced those with nothing attached are destroyed.
pub fn creator_exited(space: &SysvSpace, pid: u64) {
    let forced = space
        .limits
        .shm_rmid_forced
        .load(core::sync::atomic::Ordering::Relaxed)
        != 0;
    let mut table = space.shm.lock();
    let mine: alloc::vec::Vec<i32> = table
        .ids
        .iter_mut()
        .filter(|s| s.creator == Some(pid))
        .map(|s| {
            s.creator = None;
            s.perm.id
        })
        .collect();
    if forced {
        for id in mine {
            table.destroy_if_unused(id, true);
        }
    }
}

/// Setting shm_rmid_forced destroys every orphaned segment with nothing
/// attached (shm_destroy_orphaned).
pub fn destroy_orphaned(space: &SysvSpace) {
    let mut table = space.shm.lock();
    let orphans: alloc::vec::Vec<i32> = table
        .ids
        .iter()
        .filter(|s| s.creator.is_none())
        .map(|s| s.perm.id)
        .collect();
    for id in orphans {
        table.destroy_if_unused(id, true);
    }
}

/// `/proc/sysvipc/shm`'s lines (sysvipc_shm_proc_show), for `asker`.
pub fn proc_lines(space: &SysvSpace, asker: &Asker) -> alloc::string::String {
    use core::fmt::Write;
    let mut s = alloc::string::String::from(
        "       key      shmid perms                  size  cpid  lpid nattch   uid   gid  cuid  \
         cgid      atime      dtime      ctime                   rss                  swap\n",
    );
    // No READ on the namespace: the header and nothing else (ADR 0027).
    if !asker.may_read {
        return s;
    }
    let table = space.shm.lock();
    for seg in table.ids.iter() {
        let p = asker.shown(&seg.perm);
        let rss = (seg.pages() * PAGE) as u64;
        let _ = writeln!(
            s,
            "{:>10} {:>10}  {:>4o} {:>21} {:>5} {:>5}  {:>5} {:>5} {:>5} {:>5} {:>5} {:>10} \
             {:>10} {:>10} {:>21} {:>21}",
            p.key,
            p.id,
            p.mode,
            seg.segsz,
            pid_shown(seg.cpid),
            pid_shown(seg.lpid),
            seg.nattch,
            p.uid,
            p.gid,
            p.cuid,
            p.cgid,
            seg.atime,
            seg.dtime,
            seg.ctime,
            rss,
            0
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{super::tests::asker, *};

    #[test]
    fn shmat_checks_the_address_and_flags_first() {
        assert_eq!(
            attach_request(-1, 0, 0).err(),
            Some(SyscallError::InvalidArgument)
        );
        assert_eq!(
            attach_request(0, 0x1001, 0).err(),
            Some(SyscallError::InvalidArgument)
        );
        let r = attach_request(0, 0x1001, SHM_RND).unwrap();
        assert_eq!(r.at, Some(0x1000));
        assert_eq!(
            attach_request(0, 0x10, SHM_RND | SHM_REMAP).err(),
            Some(SyscallError::InvalidArgument),
            "rounds to 0 with SHM_REMAP"
        );
        assert_eq!(
            attach_request(0, 0, SHM_REMAP).err(),
            Some(SyscallError::InvalidArgument)
        );
        assert_eq!(attach_request(0, 0, SHM_RDONLY).unwrap().prot, 1);
        assert_eq!(attach_request(0, 0, SHM_EXEC).unwrap().prot, 7);
        assert_eq!(access_of(1), 0o444);
        assert_eq!(access_of(3), 0o666);
    }

    /// A size whose whole pages overflow is ENOSPC, not a wrapped (empty)
    /// segment, even with shmmax raised to the top (PR #40 review).
    #[test]
    fn shmget_refuses_a_size_whose_pages_overflow() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        space
            .limits
            .shmmax
            .store(u64::MAX, core::sync::atomic::Ordering::Relaxed);
        space
            .limits
            .shmall
            .store(u64::MAX, core::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            get(&space, &me, 0, usize::MAX - 10, 0o600).err(),
            Some(SyscallError::NoSpace)
        );
    }

    #[test]
    fn shmget_follows_linux_limits() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        assert_eq!(
            get(&space, &me, 0, 0, 0o600).err(),
            Some(SyscallError::InvalidArgument)
        );
        assert_eq!(
            get(&space, &me, 0, 4096, 0o600 | SHM_HUGETLB).err(),
            Some(SyscallError::InvalidArgument)
        );
        space
            .limits
            .shmall
            .store(1, core::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            get(&space, &me, 0, 8192, 0o600).err(),
            Some(SyscallError::NoSpace)
        );
        let id = get(&space, &me, 7, 100, 0o600 | super::super::IPC_CREAT).unwrap() as i32;
        // A bigger size for an existing key is EINVAL, a smaller one finds it.
        assert_eq!(
            get(&space, &me, 7, 200, 0o600).err(),
            Some(SyscallError::InvalidArgument)
        );
        assert_eq!(get(&space, &me, 7, 50, 0o600), Ok(id as usize));
        assert_eq!(space.shm.lock().tot_pages, 1);
    }

    #[test]
    fn rmid_waits_for_the_last_attachment_and_frees_the_key() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        let id = get(&space, &me, 9, 4096, 0o600 | super::super::IPC_CREAT).unwrap() as i32;
        let owner = Attach {
            space: Arc::downgrade(&space),
            id,
        };
        owner.opened();
        assert!(control(&space, &me, id, IPC_RMID, None, 0).is_ok());
        {
            let table = space.shm.lock();
            let seg = table.ids.get(id).unwrap();
            assert_ne!(seg.perm.mode & SHM_DEST, 0);
            assert_eq!(seg.perm.key, 0);
            assert_eq!(table.ids.find_key(9), None);
        }
        owner.opened();
        owner.closed();
        assert!(space.shm.lock().ids.get(id).is_ok(), "one attachment left");
        owner.closed();
        assert!(
            space.shm.lock().ids.get(id).is_err(),
            "destroyed with the last"
        );
        assert_eq!(space.shm.lock().tot_pages, 0);
    }

    #[test]
    fn stat_by_index_returns_the_id_and_lock_needs_ownership() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        let id = get(&space, &me, 0, 4096, 0o600).unwrap() as i32;
        let (ret, out) = control(&space, &me, id & 0x7fff, SHM_STAT, None, 0).unwrap();
        assert_eq!(ret, id as usize);
        assert!(matches!(out, CtlOut::Bytes(ref b) if b.len() == SHMID_DS_SIZE));
        let other = asker(2000, 2000);
        assert_eq!(
            control(&space, &other, id, IPC_STAT, None, 0).err(),
            Some(SyscallError::PermissionDenied)
        );
        assert!(control(&space, &other, id & 0x7fff, SHM_STAT_ANY, None, 0).is_ok());
        // Without READ on the namespace no look is allowed (PR #40 review).
        let mut blind = asker(1000, 1000);
        blind.may_read = false;
        for cmd in [SHM_STAT_ANY, IPC_INFO, SHM_INFO] {
            assert_eq!(
                control(&space, &blind, id & 0x7fff, cmd, None, 0).err(),
                Some(SyscallError::PermissionDenied),
                "cmd {cmd}"
            );
        }
        assert_eq!(proc_lines(&space, &blind).lines().count(), 1, "header only");
        assert_eq!(proc_lines(&space, &me).lines().count(), 2);
        assert_eq!(
            control(&space, &other, id, SHM_LOCK, None, 64).err(),
            Some(SyscallError::OperationNotPermitted)
        );
        assert_eq!(
            control(&space, &me, id, SHM_LOCK, None, 0).err(),
            Some(SyscallError::OperationNotPermitted),
            "RLIMIT_MEMLOCK 0"
        );
        assert!(control(&space, &me, id, SHM_LOCK, None, 64).is_ok());
        assert_ne!(
            space.shm.lock().ids.get(id).unwrap().perm.mode & SHM_LOCKED,
            0
        );
        assert_eq!(
            control(&space, &me, id, 99, None, 0).err(),
            Some(SyscallError::InvalidArgument)
        );
    }

    #[test]
    fn rmid_forced_destroys_an_orphan_at_its_creators_exit() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        let id = get(&space, &me, 0, 4096, 0o600).unwrap() as i32;
        let pid = space.shm.lock().ids.get(id).unwrap().cpid;
        creator_exited(&space, pid);
        assert!(
            space.shm.lock().ids.get(id).is_ok(),
            "kept without rmid_forced"
        );
        space
            .limits
            .shm_rmid_forced
            .store(1, core::sync::atomic::Ordering::Relaxed);
        destroy_orphaned(&space);
        assert!(space.shm.lock().ids.get(id).is_err());
    }
}

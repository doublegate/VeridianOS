//! System V message queues (`ipc/msg.c`): msgget, msgsnd, msgrcv, msgctl.
//!
//! A queue holds messages in the order sent, bounded by `msg_qbytes` in
//! bytes and in count, as msg_fits_inqueue bounds it. A sender with no room
//! or a receiver with nothing of its type sleeps on the namespace's queue
//! and tries again after every change.

use alloc::{collections::VecDeque, vec::Vec};

use super::{
    encode_perm, get_long, ipcget, now_secs, pid_shown, put_long, Asker, GetOutcome, IpcObject,
    Perm, SysvSpace, IPC64_PERM_SIZE, IPC_INFO, IPC_NOWAIT, IPC_RMID, IPC_SET, IPC_STAT,
};
use crate::syscall::SyscallError;

/// Linux's limits (`include/uapi/linux/msg.h`).
pub const MSGMNI: i32 = 32000;
pub const MSGMAX: i32 = 8192;
pub const MSGMNB: i32 = 16384;
const MSGPOOL: i32 = (MSGMNI as i64 * MSGMNB as i64 / 1024) as i32;
const MSGTQL: i32 = MSGMNB;
const MSGMAP: i32 = MSGMNB;
const MSGSSZ: i32 = 16;
const MSGSEG: u16 = 0xffff;

/// msgrcv flags.
pub const MSG_NOERROR: i32 = 0o10000;
pub const MSG_EXCEPT: i32 = 0o20000;
pub const MSG_COPY: i32 = 0o40000;
/// msgctl commands.
pub const MSG_STAT: i32 = 11;
pub const MSG_INFO: i32 = 12;
pub const MSG_STAT_ANY: i32 = 13;

/// A message: its type and text.
pub struct Message {
    pub mtype: i64,
    pub text: Vec<u8>,
}

/// A queue (`struct msg_queue`).
pub struct Queue {
    perm: Perm,
    pub msgs: VecDeque<Message>,
    pub cbytes: usize,
    pub qbytes: usize,
    pub stime: i64,
    pub rtime: i64,
    pub ctime: i64,
    pub lspid: u64,
    pub lrpid: u64,
}

impl IpcObject for Queue {
    fn perm(&self) -> &Perm {
        &self.perm
    }
    fn perm_mut(&mut self) -> &mut Perm {
        &mut self.perm
    }
}

impl Queue {
    /// msg_fits_inqueue: room for `size` more bytes and one more message.
    fn fits(&self, size: usize) -> bool {
        size + self.cbytes <= self.qbytes && self.msgs.len() < self.qbytes
    }

    /// find_msg: the first message `msgtyp` and `flags` select. 0: any; >0:
    /// that type (any other with MSG_EXCEPT); <0: the lowest type at most
    /// -msgtyp, the first of them.
    pub fn find(&self, msgtyp: i64, flags: i32) -> Option<usize> {
        if msgtyp == 0 {
            return (!self.msgs.is_empty()).then_some(0);
        }
        if msgtyp > 0 {
            let except = flags & MSG_EXCEPT != 0;
            return self.msgs.iter().position(|m| (m.mtype == msgtyp) != except);
        }
        let limit = if msgtyp == i64::MIN {
            i64::MAX
        } else {
            -msgtyp
        };
        let mut best: Option<(usize, i64)> = None;
        for (i, m) in self.msgs.iter().enumerate() {
            if m.mtype <= limit && best.is_none_or(|(_, t)| m.mtype < t) {
                best = Some((i, m.mtype));
            }
        }
        best.map(|(i, _)| i)
    }
}

/// msgget's checks and creation.
pub fn get(space: &SysvSpace, asker: &Asker, key: i32, flags: i32) -> Result<usize, SyscallError> {
    let mut ids = space.msg.lock();
    match ipcget(&ids, asker, key, flags, |_: &Queue| Ok(()))? {
        GetOutcome::Existing(id) => Ok(id as usize),
        GetOutcome::Create => {
            asker.may_make()?;
            let load = core::sync::atomic::Ordering::Relaxed;
            let q = Queue {
                perm: Perm::new(key, flags, asker.euid, asker.egid),
                msgs: VecDeque::new(),
                cbytes: 0,
                qbytes: space.limits.msgmnb.load(load).max(0) as usize,
                stime: 0,
                rtime: 0,
                ctime: now_secs(),
                lspid: 0,
                lrpid: 0,
            };
            let mni = space.limits.msgmni.load(load).max(0) as usize;
            Ok(ids.add(q, mni)? as usize)
        }
    }
}

/// msgsnd of `text` with type `mtype` (do_msgsnd): a size past msgmax or
/// a negative id EINVAL, a type below 1 EINVAL, then the queue (EINVAL),
/// write access (EACCES), and room, waited for unless IPC_NOWAIT (EAGAIN);
/// EIDRM if the queue goes meanwhile, EINTR for a signal.
pub fn send(
    space: &SysvSpace,
    asker: &Asker,
    id: i32,
    mtype: i64,
    text: Vec<u8>,
    flags: i32,
) -> Result<usize, SyscallError> {
    let msgmax = space
        .limits
        .msgmax
        .load(core::sync::atomic::Ordering::Relaxed);
    if text.len() > msgmax.max(0) as usize || id < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if mtype < 1 {
        return Err(SyscallError::InvalidArgument);
    }
    {
        let ids = space.msg.lock();
        let q = ids.get(id)?;
        if !asker.permits(&q.perm, 0o222) {
            return Err(SyscallError::PermissionDenied);
        }
    }
    let mut msg = Some(Message { mtype, text });
    let pid = super::current_pid();
    // Queue it if it fits; Err when the queue is gone.
    let try_send = |msg: &mut Option<Message>| -> Result<bool, SyscallError> {
        let mut ids = space.msg.lock();
        let q = ids.get_mut(id).map_err(|_| SyscallError::IdRemoved)?;
        let size = msg.as_ref().map_or(0, |m| m.text.len());
        if !q.fits(size) {
            return Ok(false);
        }
        if let Some(m) = msg.take() {
            q.cbytes += m.text.len();
            q.msgs.push_back(m);
        }
        q.lspid = pid;
        q.stime = now_secs();
        Ok(true)
    };
    let first = try_send(&mut msg).map_err(|_| SyscallError::InvalidArgument)?;
    if !first {
        if flags & IPC_NOWAIT != 0 {
            return Err(SyscallError::WouldBlock);
        }
        let mut outcome = Ok(false);
        let waited = crate::sched::dispatch::wait_event(&space.msg_waiters, None, || {
            outcome = try_send(&mut msg);
            !matches!(outcome, Ok(false))
        });
        if waited.is_err() {
            return Err(SyscallError::Interrupted);
        }
        outcome?;
    }
    space.msg_waiters.wake_all();
    Ok(0)
}

/// msgrcv (do_msgrcv): a negative id or size EINVAL; MSG_COPY needs
/// IPC_NOWAIT and no MSG_EXCEPT (EINVAL), and is then ENOSYS, as without
/// CONFIG_CHECKPOINT_RESTORE; read access (EACCES); a message longer than
/// `size` without MSG_NOERROR is E2BIG and stays queued; nothing of the
/// type waits unless IPC_NOWAIT (ENOMSG). Returns the message, its text cut
/// to `size`.
pub fn receive(
    space: &SysvSpace,
    asker: &Asker,
    id: i32,
    size: isize,
    msgtyp: i64,
    flags: i32,
) -> Result<Message, SyscallError> {
    if id < 0 || size < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if flags & MSG_COPY != 0 {
        if flags & MSG_EXCEPT != 0 || flags & IPC_NOWAIT == 0 {
            return Err(SyscallError::InvalidArgument);
        }
        return Err(SyscallError::NotImplemented);
    }
    let size = size as usize;
    {
        let ids = space.msg.lock();
        let q = ids.get(id)?;
        if !asker.permits(&q.perm, 0o444) {
            return Err(SyscallError::PermissionDenied);
        }
        // Receiving takes the message out: a change (ADR 0027).
        asker.may_change()?;
    }
    let pid = super::current_pid();
    let try_receive = || -> Result<Option<Message>, SyscallError> {
        let mut ids = space.msg.lock();
        let q = ids.get_mut(id).map_err(|_| SyscallError::IdRemoved)?;
        let Some(i) = q.find(msgtyp, flags) else {
            return Ok(None);
        };
        if q.msgs[i].text.len() > size && flags & MSG_NOERROR == 0 {
            return Err(SyscallError::ArgumentListTooLong);
        }
        let Some(mut m) = q.msgs.remove(i) else {
            return Ok(None);
        };
        q.cbytes -= m.text.len();
        q.lrpid = pid;
        q.rtime = now_secs();
        m.text.truncate(size);
        Ok(Some(m))
    };
    let got = match try_receive() {
        Err(SyscallError::IdRemoved) => return Err(SyscallError::InvalidArgument),
        other => other?,
    };
    let m = match got {
        Some(m) => m,
        None if flags & IPC_NOWAIT != 0 => return Err(SyscallError::NoMessage),
        None => {
            let mut outcome = Ok(None);
            let waited = crate::sched::dispatch::wait_event(&space.msg_waiters, None, || {
                outcome = try_receive();
                !matches!(outcome, Ok(None))
            });
            if waited.is_err() {
                return Err(SyscallError::Interrupted);
            }
            outcome?.ok_or(SyscallError::Interrupted)?
        }
    };
    space.msg_waiters.wake_all();
    Ok(m)
}

/// `struct msqid64_ds` (generic 64-bit, also x86_64's).
pub const MSQID_DS_SIZE: usize = 120;
/// `struct msginfo`: seven ints and a short.
pub const MSGINFO_SIZE: usize = 32;

fn encode_ds(asker: &Asker, q: &Queue) -> Vec<u8> {
    let mut out = alloc::vec![0u8; MSQID_DS_SIZE];
    encode_perm(&asker.shown(&q.perm), &mut out[..IPC64_PERM_SIZE]);
    put_long(&mut out, 48, q.stime as u64);
    put_long(&mut out, 56, q.rtime as u64);
    put_long(&mut out, 64, q.ctime as u64);
    put_long(&mut out, 72, q.cbytes as u64);
    put_long(&mut out, 80, q.msgs.len() as u64);
    put_long(&mut out, 88, q.qbytes as u64);
    out[96..100].copy_from_slice(&pid_shown(q.lspid).to_ne_bytes());
    out[100..104].copy_from_slice(&pid_shown(q.lrpid).to_ne_bytes());
    out
}

/// IPC_SET's fields from the caller's `msqid_ds`: uid, gid, mode, qbytes.
pub fn decode_set(buf: &[u8]) -> (u32, u32, u32, u64) {
    let (uid, gid, mode) = super::decode_set_perm(buf);
    (uid, gid, mode, get_long(buf, 88))
}

/// msgctl.
pub fn control(
    space: &SysvSpace,
    asker: &Asker,
    id: i32,
    cmd: i32,
    set: Option<(u32, u32, u32, u64)>,
) -> Result<(usize, Option<Vec<u8>>), SyscallError> {
    // The commands that look without an object's own permission check
    // need READ on the namespace, as every look does (ADR 0027; PR #40
    // review).
    if matches!(cmd, IPC_INFO | MSG_INFO | MSG_STAT_ANY) && !asker.may_read {
        return Err(SyscallError::PermissionDenied);
    }
    let load = core::sync::atomic::Ordering::Relaxed;
    let mut ids = space.msg.lock();
    match cmd {
        IPC_INFO | MSG_INFO => {
            let mut info = [
                MSGPOOL,
                MSGMAP,
                space.limits.msgmax.load(load),
                space.limits.msgmnb.load(load),
                space.limits.msgmni.load(load),
                MSGSSZ,
                MSGTQL,
            ];
            if cmd == MSG_INFO {
                info[0] = ids.in_use() as i32;
                info[1] = ids.iter().map(|q| q.msgs.len()).sum::<usize>() as i32;
                info[6] = ids.iter().map(|q| q.cbytes).sum::<usize>() as i32;
            }
            let mut out = alloc::vec![0u8; MSGINFO_SIZE];
            for (i, v) in info.iter().enumerate() {
                out[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
            }
            out[28..30].copy_from_slice(&MSGSEG.to_ne_bytes());
            Ok((ids.max_idx() as usize, Some(out)))
        }
        IPC_STAT | MSG_STAT | MSG_STAT_ANY => {
            let q = if cmd == IPC_STAT {
                ids.get(id)?
            } else {
                ids.by_index(id)?
            };
            if cmd != MSG_STAT_ANY && !asker.permits(&q.perm, 0o444) {
                return Err(SyscallError::PermissionDenied);
            }
            let ret = if cmd == IPC_STAT {
                0
            } else {
                q.perm.id as usize
            };
            Ok((ret, Some(encode_ds(asker, q))))
        }
        IPC_RMID | IPC_SET => {
            let q = ids.get_mut(id)?;
            if !asker.may_control(&q.perm) {
                return Err(SyscallError::OperationNotPermitted);
            }
            if cmd == IPC_SET {
                let (uid, gid, mode, qbytes) = set.ok_or(SyscallError::InvalidPointer)?;
                let msgmnb = space.limits.msgmnb.load(load).max(0) as u64;
                if qbytes > msgmnb && !asker.sys_resource {
                    return Err(SyscallError::OperationNotPermitted);
                }
                asker.update(&mut q.perm, uid, gid, mode)?;
                q.qbytes = qbytes as usize;
                q.ctime = now_secs();
            } else {
                ids.remove(id);
            }
            drop(ids);
            space.msg_waiters.wake_all();
            Ok((0, None))
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// `/proc/sysvipc/msg`'s lines (sysvipc_msg_proc_show).
pub fn proc_lines(space: &SysvSpace, asker: &Asker) -> alloc::string::String {
    use core::fmt::Write;
    let mut s =
        alloc::string::String::from(
            "       key      msqid perms      cbytes       qnum lspid lrpid   uid   gid  cuid  \
             cgid      stime      rtime      ctime\n",
        );
    // No READ on the namespace: the header and nothing else (ADR 0027).
    if !asker.may_read {
        return s;
    }
    for q in space.msg.lock().iter() {
        let p = asker.shown(&q.perm);
        let _ = writeln!(
            s,
            "{:>10} {:>10}  {:>4o}  {:>10} {:>10} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>10} \
             {:>10} {:>10}",
            p.key,
            p.id,
            p.mode,
            q.cbytes,
            q.msgs.len(),
            pid_shown(q.lspid),
            pid_shown(q.lrpid),
            p.uid,
            p.gid,
            p.cuid,
            p.cgid,
            q.stime,
            q.rtime,
            q.ctime
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{super::tests::asker, *};

    fn text(n: usize) -> Vec<u8> {
        alloc::vec![b'x'; n]
    }

    #[test]
    fn messages_are_found_by_type_as_linux_finds_them() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        let id = get(&space, &me, 0, 0o600).unwrap() as i32;
        for t in [3, 1, 2, 1] {
            send(&space, &me, id, t, text(1), 0).unwrap();
        }
        let ids = space.msg.lock();
        let q = ids.get(id).unwrap();
        assert_eq!(q.find(0, 0), Some(0));
        assert_eq!(q.find(2, 0), Some(2));
        assert_eq!(q.find(3, MSG_EXCEPT), Some(1));
        assert_eq!(
            q.find(-2, 0),
            Some(1),
            "lowest type at most 2, first of them"
        );
        assert_eq!(q.find(-0x7fff_ffff_ffff_ffff - 1, 0), Some(1));
        assert_eq!(q.find(4, 0), None);
    }

    #[test]
    fn send_and_receive_check_in_linux_order() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        let id = get(&space, &me, 0, 0o600).unwrap() as i32;
        assert_eq!(
            send(&space, &me, id, 1, text(MSGMAX as usize + 1), 0).err(),
            Some(SyscallError::InvalidArgument)
        );
        assert_eq!(
            send(&space, &me, id, 0, text(1), 0).err(),
            Some(SyscallError::InvalidArgument)
        );
        assert_eq!(
            send(&space, &asker(2000, 2000), id, 1, text(1), 0).err(),
            Some(SyscallError::PermissionDenied)
        );
        send(&space, &me, id, 5, text(10), 0).unwrap();
        assert_eq!(
            receive(&space, &me, id, 4, 0, 0).err(),
            Some(SyscallError::ArgumentListTooLong)
        );
        let m = receive(&space, &me, id, 4, 0, MSG_NOERROR).unwrap();
        assert_eq!((m.mtype, m.text.len()), (5, 4));
        assert_eq!(
            receive(&space, &me, id, 4, 0, IPC_NOWAIT).err(),
            Some(SyscallError::NoMessage)
        );
        assert_eq!(
            receive(&space, &me, id, 4, 0, MSG_COPY).err(),
            Some(SyscallError::InvalidArgument)
        );
        assert_eq!(
            receive(&space, &me, id, 4, 0, MSG_COPY | IPC_NOWAIT).err(),
            Some(SyscallError::NotImplemented)
        );
        let ids = space.msg.lock();
        let q = ids.get(id).unwrap();
        assert_eq!((q.cbytes, q.msgs.len()), (0, 0));
    }

    #[test]
    fn a_full_queue_is_eagain_and_qbytes_needs_cap_sys_resource_to_grow() {
        let space = SysvSpace::new();
        let me = asker(1000, 1000);
        let id = get(&space, &me, 0, 0o600).unwrap() as i32;
        assert!(control(&space, &me, id, IPC_SET, Some((1000, 1000, 0o600, 8))).is_ok());
        send(&space, &me, id, 1, text(8), 0).unwrap();
        assert_eq!(
            send(&space, &me, id, 1, text(1), IPC_NOWAIT).err(),
            Some(SyscallError::WouldBlock)
        );
        assert_eq!(
            control(
                &space,
                &me,
                id,
                IPC_SET,
                Some((1000, 1000, 0o600, MSGMNB as u64 + 1))
            )
            .err(),
            Some(SyscallError::OperationNotPermitted)
        );
        let (ret, out) = control(&space, &me, id & 0x7fff, MSG_STAT, None).unwrap();
        assert_eq!(ret, id as usize);
        let out = out.unwrap();
        assert_eq!(get_long(&out, 80), 1, "qnum");
        assert_eq!(get_long(&out, 88), 8, "qbytes");
        assert!(control(&space, &me, id, IPC_RMID, None).is_ok());
        assert_eq!(
            send(&space, &me, id, 1, text(1), 0).err(),
            Some(SyscallError::InvalidArgument)
        );
    }
}

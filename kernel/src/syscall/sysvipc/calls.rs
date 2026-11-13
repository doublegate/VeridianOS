//! The System V IPC system calls: user memory in and out, around the
//! namespace's tables (`shm`, `sem`, `msg`).

use alloc::vec::Vec;

use super::{
    ctl_cmd, current_ns, decode_set_perm, msg, sem, shm, space_of, Asker, IPC64_PERM_SIZE, IPC_SET,
};
use crate::syscall::{
    userspace::{read_user, read_user_bytes, read_user_vec, write_user, write_user_bytes},
    SyscallError, SyscallResult,
};

fn asker_and_space() -> (Asker, alloc::sync::Arc<super::SysvSpace>) {
    let ns = current_ns();
    let asker = Asker::current(&ns);
    (asker, space_of(&ns).clone())
}

/// shmget(key, size, shmflg).
pub fn sys_shmget(key: usize, size: usize, flags: usize) -> SyscallResult {
    let (asker, space) = asker_and_space();
    shm::get(&space, &asker, key as i32, size, flags as i32)
}

/// shmat(shmid, shmaddr, shmflg): the address the segment is attached at.
pub fn sys_shmat(id: usize, addr: usize, flags: usize) -> SyscallResult {
    let (asker, space) = asker_and_space();
    shm::attach(&space, &asker, id as i32, addr, flags as i32)
}

/// shmdt(shmaddr).
pub fn sys_shmdt(addr: usize) -> SyscallResult {
    shm::detach(addr)
}

/// shmctl(shmid, cmd, buf).
pub fn sys_shmctl(id: usize, cmd: usize, buf: usize) -> SyscallResult {
    let cmd = ctl_cmd(cmd as i32);
    let (asker, space) = asker_and_space();
    let set = if cmd == IPC_SET {
        let mut b = [0u8; IPC64_PERM_SIZE];
        read_user_bytes(buf, &mut b)?;
        Some(decode_set_perm(&b))
    } else {
        None
    };
    let memlock = crate::process::current_process().map_or(0, |p| {
        p.limits().cur(crate::process::rlimit::RLIMIT_MEMLOCK)
    });
    let (ret, out) = shm::control(&space, &asker, id as i32, cmd, set, memlock)?;
    if let shm::CtlOut::Bytes(b) = out {
        write_user_bytes(buf, &b)?;
    }
    Ok(ret)
}

/// semget(key, nsems, semflg).
pub fn sys_semget(key: usize, nsems: usize, flags: usize) -> SyscallResult {
    let (asker, space) = asker_and_space();
    sem::get(&space, &asker, key as i32, nsems as i32, flags as i32)
}

/// semop(semid, sops, nsops): semtimedop without a timeout.
pub fn sys_semop(id: usize, sops: usize, nsops: usize) -> SyscallResult {
    sys_semtimedop(id, sops, nsops, 0)
}

/// semtimedop(semid, sops, nsops, timeout): `timeout` relative, EAGAIN
/// once it passes.
pub fn sys_semtimedop(id: usize, sops: usize, nsops: usize, timeout: usize) -> SyscallResult {
    let (asker, space) = asker_and_space();
    let [_, _, semopm, _] = space.limits.sem_limits();
    // The counts are checked before the operations are read (ksys_semtimedop).
    if nsops < 1 || (id as i32) < 0 {
        return Err(SyscallError::InvalidArgument);
    }
    if nsops > semopm.max(0) as usize {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let raw = read_user_vec(sops, nsops * 6, nsops * 6)?;
    let ops: Vec<sem::SemBuf> = raw.chunks_exact(6).map(sem::SemBuf::decode).collect();
    let deadline = if timeout != 0 {
        let ts: [i64; 2] = read_user(timeout)?;
        if ts[0] < 0 || !(0..1_000_000_000).contains(&ts[1]) {
            return Err(SyscallError::InvalidArgument);
        }
        let ns = (ts[0] as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(ts[1] as u64);
        Some(crate::sched::dispatch::clock_ns().saturating_add(ns))
    } else {
        None
    };
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    sem::semop(&space, &asker, id as i32, &ops, deadline, || {
        sem::undo_of(&process)
    })
}

/// semctl(semid, semnum, cmd, arg): `arg` is the union semun's value
/// (SETVAL's int, or a pointer).
pub fn sys_semctl(id: usize, num: usize, cmd: usize, arg: usize) -> SyscallResult {
    let cmd = ctl_cmd(cmd as i32);
    let (asker, space) = asker_and_space();
    let id = id as i32;
    let ctl_arg = match cmd {
        sem::SETVAL => sem::CtlArg::Val(arg as u32 as i32),
        sem::SETALL => {
            let n = sem::nsems(&space, id)?;
            let raw = read_user_vec(arg, n * 2, n * 2)?;
            sem::CtlArg::Values(
                raw.chunks_exact(2)
                    .map(|b| u16::from_ne_bytes([b[0], b[1]]))
                    .collect(),
            )
        }
        IPC_SET => {
            let mut b = [0u8; IPC64_PERM_SIZE];
            read_user_bytes(arg, &mut b)?;
            let (uid, gid, mode) = decode_set_perm(&b);
            sem::CtlArg::Set(uid, gid, mode)
        }
        _ => sem::CtlArg::None,
    };
    let (ret, out) = sem::control(&space, &asker, id, num as i32, cmd, ctl_arg)?;
    match out {
        sem::CtlOut::Bytes(b) => write_user_bytes(arg, &b)?,
        sem::CtlOut::Values(v) => {
            let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_ne_bytes()).collect();
            write_user_bytes(arg, &bytes)?;
        }
        sem::CtlOut::None => {}
    }
    Ok(ret)
}

/// msgget(key, msgflg).
pub fn sys_msgget(key: usize, flags: usize) -> SyscallResult {
    let (asker, space) = asker_and_space();
    msg::get(&space, &asker, key as i32, flags as i32)
}

/// msgsnd(msqid, msgp, msgsz, msgflg): `msgp` is a long type, then the
/// text. The type is read first (EFAULT before any other check), as
/// ksys_msgsnd reads it.
pub fn sys_msgsnd(id: usize, msgp: usize, size: usize, flags: usize) -> SyscallResult {
    let mtype: i64 = read_user(msgp)?;
    let (asker, space) = asker_and_space();
    let msgmax = space
        .limits
        .msgmax
        .load(core::sync::atomic::Ordering::Relaxed);
    if (size as isize) < 0 || size > msgmax.max(0) as usize || (id as i32) < 0 || mtype < 1 {
        return Err(SyscallError::InvalidArgument);
    }
    let text = read_user_vec(msgp + 8, size, size)?;
    msg::send(&space, &asker, id as i32, mtype, text, flags as i32)
}

/// msgrcv(msqid, msgp, msgsz, msgtyp, msgflg): the length of the text
/// copied to `msgp` after its type.
pub fn sys_msgrcv(
    id: usize,
    msgp: usize,
    size: usize,
    msgtyp: usize,
    flags: usize,
) -> SyscallResult {
    let (asker, space) = asker_and_space();
    let m = msg::receive(
        &space,
        &asker,
        id as i32,
        size as isize,
        msgtyp as i64,
        flags as i32,
    )?;
    write_user(msgp, m.mtype)?;
    write_user_bytes(msgp + 8, &m.text)?;
    Ok(m.text.len())
}

/// msgctl(msqid, cmd, buf).
pub fn sys_msgctl(id: usize, cmd: usize, buf: usize) -> SyscallResult {
    let cmd = ctl_cmd(cmd as i32);
    let (asker, space) = asker_and_space();
    let set = if cmd == IPC_SET {
        let mut b = [0u8; msg::MSQID_DS_SIZE];
        read_user_bytes(buf, &mut b)?;
        Some(msg::decode_set(&b))
    } else {
        None
    };
    let (ret, out) = msg::control(&space, &asker, id as i32, cmd, set)?;
    if let Some(b) = out {
        write_user_bytes(buf, &b)?;
    }
    Ok(ret)
}

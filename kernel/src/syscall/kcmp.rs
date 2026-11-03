//! kcmp(2), as Linux 7.3's `kernel/kcmp.c`: whether two tasks share a
//! kernel object, ordered by an obfuscated address so the comparison
//! reveals no kernel pointer.

use alloc::sync::Arc;

use super::{SyscallError, SyscallResult};
use crate::process::{self, Process, Thread, ThreadId};

/// `enum kcmp_type`.
const KCMP_FILE: usize = 0;
const KCMP_VM: usize = 1;
const KCMP_FILES: usize = 2;
const KCMP_FS: usize = 3;
const KCMP_SIGHAND: usize = 4;
const KCMP_IO: usize = 5;
const KCMP_SYSVSEM: usize = 6;
const KCMP_EPOLL_TFD: usize = 7;
const KCMP_TYPES: usize = 8;

/// Per-type random cookies (Linux's kcmp_cookies_init): an address is
/// XORed with the first and multiplied by the second, which has its top
/// and bottom bits set, before two are compared.
static COOKIES: spin::Once<[[u64; 2]; KCMP_TYPES]> = spin::Once::new();

fn cookies() -> &'static [[u64; 2]; KCMP_TYPES] {
    COOKIES.call_once(|| {
        let mut bytes = [0u8; KCMP_TYPES * 16];
        let _ = crate::crypto::random::get_random().fill_bytes(&mut bytes);
        let mut c = [[0u64; 2]; KCMP_TYPES];
        for (i, pair) in c.iter_mut().enumerate() {
            for (j, word) in pair.iter_mut().enumerate() {
                let at = (i * 2 + j) * 8;
                let mut b = [0u8; 8];
                b.copy_from_slice(&bytes[at..at + 8]);
                *word = u64::from_ne_bytes(b);
            }
            pair[1] |= (1 << 63) | 1;
        }
        c
    })
}

/// Linux's kcmp_ptr: 0 if `a` and `b` are the same object, otherwise 1 or
/// 2 as the obfuscated `a` sorts below or above the obfuscated `b`.
fn compare(a: usize, b: usize, cookie: [u64; 2]) -> usize {
    let hide = |v: usize| ((v as u64 ^ cookie[0]).wrapping_mul(cookie[1])) as i64;
    let (x, y) = (hide(a), hide(b));
    usize::from(x < y) | (usize::from(x > y) << 1)
}

/// The task `id` names (a process's or thread's ID, a pid_t), with its
/// process: ESRCH for none.
fn task(id: usize) -> Result<(Arc<Process>, Arc<Thread>), SyscallError> {
    let id = id as u32 as i32;
    if id <= 0 {
        return Err(SyscallError::ProcessNotFound);
    }
    let process = process::get_process(process::ProcessId(id as u64))
        .or_else(|| super::process::process_of_thread(id as usize).and_then(process::get_process))
        .ok_or(SyscallError::ProcessNotFound)?;
    let thread = process
        .get_thread(ThreadId(id as u64))
        .ok_or(SyscallError::ProcessNotFound)?;
    Ok((process, thread))
}

/// The value that stands for each task's object of `kind` (not
/// KCMP_FILE or KCMP_EPOLL_TFD, which take descriptors): the address space
/// is the process's `vm_id` (a CLONE_VM child shares it until either
/// execs), the file table and signal handlers the process's (shared by its
/// threads and by no other process here), the filesystem state the
/// thread's (CLONE_FS shares it). No task has an I/O context or a System V
/// semaphore undo list, so KCMP_IO and KCMP_SYSVSEM always find them equal,
/// as Linux does for two tasks with none.
fn object(kind: usize, process: &Process, thread: &Thread) -> usize {
    match kind {
        KCMP_VM => process.vm_id.load(core::sync::atomic::Ordering::Acquire) as usize,
        KCMP_FILES => &process.file_table as *const _ as usize,
        KCMP_FS => Arc::as_ptr(&thread.fs()) as usize,
        KCMP_SIGHAND => &process.signal_handlers as *const _ as usize,
        _ => 0,
    }
}

/// kcmp(pid1, pid2, type, idx1, idx2): whether tasks `pid1` and `pid2`
/// share an object of `type` -- 0 if they do, 1 or 2 by an order that
/// stays the same until reboot. Either task missing is ESRCH; the caller
/// must be allowed to read both as ptrace would (EPERM), checked under
/// both credential guards as Linux holds both exec_update_locks; then an
/// unknown type is EINVAL. KCMP_FILE compares descriptor `idx1` of the
/// first with `idx2` of the second (EBADF if either is closed);
/// KCMP_EPOLL_TFD compares `idx1` with the file the second task's epoll
/// instance watches under the `struct kcmp_epoll_slot` at `idx2` (EFAULT,
/// EBADF, EINVAL for a descriptor that is no epoll instance, ENOENT for no
/// such registration).
pub fn sys_kcmp(pid1: usize, pid2: usize, kind: usize, idx1: usize, idx2: usize) -> SyscallResult {
    let (p1, t1) = task(pid1)?;
    let (p2, t2) = task(pid2)?;
    let caller = process::creds::Caller::current();
    let permitted = super::debug::current_permitted();
    // Both guards, the lower address first (one if it is one process).
    let (first, second) = if (Arc::as_ptr(&p1) as usize) <= (Arc::as_ptr(&p2) as usize) {
        (&p1, &p2)
    } else {
        (&p2, &p1)
    };
    let _g1 = first.cred_guard.lock();
    let _g2 = (!Arc::ptr_eq(&p1, &p2)).then(|| second.cred_guard.lock());
    let may = |p: &Arc<Process>| super::debug::may_access_process(&caller, permitted, p);
    if !may(&p1) || !may(&p2) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let kind = kind as u32 as usize;
    let cookie = |k: usize| cookies()[k];
    match kind {
        KCMP_FILE => {
            let f1 = p1.file_table.lock().get_path(idx1 as u32 as usize);
            let f2 = p2.file_table.lock().get_path(idx2 as u32 as usize);
            match (f1, f2) {
                (Some(a), Some(b)) => Ok(compare(
                    Arc::as_ptr(&a) as usize,
                    Arc::as_ptr(&b) as usize,
                    cookie(KCMP_FILE),
                )),
                _ => Err(SyscallError::BadFileDescriptor),
            }
        }
        KCMP_VM | KCMP_FILES | KCMP_FS | KCMP_SIGHAND | KCMP_IO | KCMP_SYSVSEM => Ok(compare(
            object(kind, &p1, &t1),
            object(kind, &p2, &t2),
            cookie(kind),
        )),
        KCMP_EPOLL_TFD => {
            // struct kcmp_epoll_slot: efd, tfd, toff (u32 each).
            let mut raw = [0u8; 12];
            super::userspace::read_user_bytes(idx2, &mut raw)?;
            let word = |i: usize| u32::from_ne_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]);
            let (efd, tfd, toff) = (word(0), word(4), word(8));
            let file = p1
                .file_table
                .lock()
                .get_path(idx1 as u32 as usize)
                .ok_or(SyscallError::BadFileDescriptor)?;
            let epoll = p2
                .file_table
                .lock()
                .get_path(efd as usize)
                .ok_or(SyscallError::BadFileDescriptor)?;
            let instance =
                crate::net::epoll::EpollNode::of(&epoll).ok_or(SyscallError::InvalidArgument)?;
            let target = instance
                .watched_file(tfd as i32, toff as usize)
                .ok_or(SyscallError::ResourceNotFound)?;
            Ok(compare(
                Arc::as_ptr(&file) as usize,
                target,
                cookie(KCMP_FILE),
            ))
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_is_zero_for_one_object_and_antisymmetric() {
        let cookie = [0x1234_5678_9abc_def0, (1 << 63) | 0x9e37_79b9_7f4a_7c15];
        assert_eq!(compare(0x1000, 0x1000, cookie), 0);
        let (ab, ba) = (
            compare(0x1000, 0x2000, cookie),
            compare(0x2000, 0x1000, cookie),
        );
        assert!(ab == 1 || ab == 2);
        assert_eq!(ab + ba, 3);
    }

    #[test]
    fn compare_orders_consistently() {
        let cookie = [0xdead_beef, (1 << 63) | 0x2545_f491_4f6c_dd1d];
        let v = [0x1000usize, 0x5000, 0x9000];
        // Whatever order the cookies give, it is a total order.
        let less = |a: usize, b: usize| compare(a, b, cookie) == 1;
        for &a in &v {
            for &b in &v {
                for &c in &v {
                    if less(a, b) && less(b, c) {
                        assert!(less(a, c));
                    }
                }
            }
        }
    }
}

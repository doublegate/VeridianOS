//! acct(2): BSD process accounting, version 3 records, as Linux 7.3's
//! `kernel/acct.c` (ADR 0027, item 5 decisions). Accounting is per PID
//! namespace: a process's record goes to every namespace, from its own up,
//! that has accounting on, numbered as that namespace sees it
//! (slow_acct_process; PR #49 review).
//!
//! Each namespace's accounting has a sleeping lock, held across a record's
//! write (`acct->lock`): the table's spinlock is only held to find it, never
//! across file I/O (PR #49 review). A writer takes the lock and then checks
//! that the accounting is still on, as acct_get rechecks `ns->bacct`, so a
//! record is never written into a file `acct(NULL)` or a switch has closed.

use alloc::{collections::BTreeMap, sync::Arc};

use spin::Mutex;

use super::{SyscallError, SyscallResult};
use crate::{
    fs::file::File,
    process::{creds::Caller, linux_caps, pidns::PidNs, Process},
    sync::sleep_mutex::SleepMutex,
};

/// The accounting of one PID namespace (`struct bsd_acct_struct`).
struct Acct {
    file: Arc<File>,
    /// Who opened it: its ids are shown as its user namespace sees them.
    /// (Linux writes the records with its credentials, override_creds; the
    /// write here checks none, so only the namespace is kept from them.)
    opener: Caller,
    ns: Arc<PidNs>,
    state: SleepMutex<State>,
}

/// What an [`Acct`]'s lock guards.
struct State {
    /// Still the namespace's accounting (false once switched off or over).
    live: bool,
    /// Writing records (free space above SUSPEND), or paused.
    active: bool,
    /// When free space is next checked (monotonic ns).
    needcheck: u64,
}

/// Accounting, by PID namespace id.
static ACCT: Mutex<BTreeMap<u64, Arc<Acct>>> = Mutex::new(BTreeMap::new());

/// acct_get: `f` with the accounting of namespace `id` and its lock held,
/// if it has accounting on; one switched off while its lock was awaited is
/// looked up again.
fn with_acct<R>(id: u64, f: impl FnOnce(&Acct, &mut State) -> R) -> Option<R> {
    loop {
        let acct = ACCT.lock().get(&id).cloned()?;
        let mut state = acct.state.lock();
        if state.live {
            return Some(f(&acct, &mut state));
        }
    }
}

/// acct_pin_kill: under its lock, `acct` writes `process`'s record with
/// `exit_code` (fill_ac of the caller) and stops being the namespace's.
fn retire(acct: &Arc<Acct>, process: &Process, exit_code: u32) {
    let mut state = acct.state.lock();
    if !state.live {
        return;
    }
    write_record(acct, &mut state, process, exit_code);
    state.live = false;
    let mut table = ACCT.lock();
    if table
        .get(&acct.ns.ident.id)
        .is_some_and(|a| Arc::ptr_eq(a, acct))
    {
        table.remove(&acct.ns.ident.id);
    }
}

/// acct_parm: resume above 4 % free, suspend below 2 %, check every 30 s.
const RESUME: u64 = 4;
const SUSPEND: u64 = 2;
const ACCT_TIMEOUT_NS: u64 = 30_000_000_000;

/// Linux's AHZ: the records' clock ticks per second.
const AHZ: u64 = 100;

/// The record's flags (AFORK, ASU, ACORE, AXSIG).
const AFORK: u8 = 0x01;
const ACORE: u8 = 0x08;
const AXSIG: u8 = 0x10;

/// O_WRONLY | O_APPEND | O_LARGEFILE: how the file is opened.
const OPEN_FLAGS: usize = 0o1 | 0o2000 | 0o100000;

/// acct(name): needs CAP_SYS_PACCT (EPERM); NULL turns the caller's PID
/// namespace's accounting off, a name turns it on into that file (a
/// regular file opened for appending: EACCES otherwise). Switching off or
/// over writes the caller's record into the old file first, as Linux's
/// pin_kill.
pub fn sys_acct(name: usize) -> SyscallResult {
    let me = Caller::current();
    if !me.capable(linux_caps::CAP_SYS_PACCT) {
        return Err(SyscallError::OperationNotPermitted);
    }
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let ns = process.pid_ns();
    let new = if name != 0 {
        let path = super::filesystem::read_user_path(name)?;
        Some(open_acct_file(&path)?)
    } else {
        None
    };
    let old = {
        let mut table = ACCT.lock();
        match new {
            Some(file) => table.insert(
                ns.ident.id,
                Arc::new(Acct {
                    file,
                    opener: me,
                    ns: ns.clone(),
                    state: SleepMutex::new(State {
                        live: true,
                        active: true,
                        needcheck: crate::timer::monotonic_ns().saturating_add(ACCT_TIMEOUT_NS),
                    }),
                }),
            ),
            None => table.get(&ns.ident.id).cloned(),
        }
    };
    if let Some(old) = old {
        // The caller is still running: exit code 0 (its pacct so far).
        retire(&old, &process, 0);
    }
    Ok(0)
}

/// file_open_name(name, O_WRONLY | O_APPEND | O_LARGEFILE): the open's own
/// checks (ENOENT, EACCES, EISDIR, EROFS...), then a regular file
/// (EACCES) on a filesystem that can be mounted (EINVAL for pipes,
/// sockets and the like).
fn open_acct_file(path: &str) -> Result<Arc<File>, SyscallError> {
    let abs = super::filesystem::resolve_at_path(super::filesystem::AT_FDCWD, path)?;
    let view = super::filesystem::vfs()?;
    let flags = crate::fs::file::OpenFlags::from_bits(OPEN_FLAGS as u32)
        .ok_or(SyscallError::InvalidArgument)?;
    let (node, canonical) = view
        .open_follow_canonical(&abs, flags, true)
        .map_err(super::filesystem::map_resolve_err)?;
    let file = super::filesystem::open_file(node, false, canonical, OPEN_FLAGS, false, true)?;
    if file.node.node_type() != crate::fs::NodeType::File {
        return Err(SyscallError::PermissionDenied);
    }
    if file.path.is_none() {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(file)
}

/// The last thread of `process` is exiting with `exit_code` (a wait
/// status): its record in every namespace it is in that accounts, its own
/// first (acct_process and slow_acct_process). A namespace's init is no
/// exception: its namespace's accounting ends only once the namespace is
/// empty ([`ns_exited`]), so the processes its exit kills are recorded
/// there too (PR #49 review).
pub fn process_exited(process: &Process, exit_code: u32) {
    let mut ns = Some(process.pid_ns());
    while let Some(n) = ns {
        let _ = with_acct(n.ident.id, |acct, state| {
            write_record(acct, state, process, exit_code)
        });
        ns = n.parent.clone();
    }
}

/// The init `process` of a PID namespace has exited with `exit_code` and
/// every other process in the namespace is gone (the end of
/// zap_pid_ns_processes): the namespace's accounting ends, with init's
/// record written last, as acct_exit_ns's pin_kill does. `exit_code` is the
/// one its own record took: Linux fills both from the same pacct, before a
/// reboot(2) signal replaces the code its parent sees.
pub fn ns_exited(process: &Process, exit_code: u32) {
    let acct = ACCT.lock().get(&process.pid_ns().ident.id).cloned();
    if let Some(acct) = acct {
        retire(&acct, process, exit_code);
    }
}

/// check_free_space: pause below SUSPEND % free, resume above RESUME %,
/// looked at every ACCT_TIMEOUT.
fn check_space(acct: &Acct, state: &mut State) -> bool {
    let now = crate::timer::monotonic_ns();
    if now < state.needcheck {
        return state.active;
    }
    if let Ok(st) = acct.file.node.statfs() {
        let blocks = st.blocks;
        if state.active && st.blocks_avail <= blocks * SUSPEND / 100 {
            state.active = false;
            crate::println!("Process accounting paused");
        } else if !state.active && st.blocks_avail >= blocks * RESUME / 100 {
            state.active = true;
            crate::println!("Process accounting resumed");
        }
    }
    state.needcheck = now.saturating_add(ACCT_TIMEOUT_NS);
    state.active
}

/// fill_ac and do_acct_process: the 64-byte `struct acct_v3`, appended to
/// the file whatever RLIMIT_FSIZE says. The caller holds `acct`'s lock.
fn write_record(acct: &Acct, state: &mut State, p: &Process, exit_code: u32) {
    if !check_space(acct, state) {
        return;
    }
    let rec = record(acct, p, exit_code);
    let node = &acct.file.node;
    let _io = acct.file.io_lock.lock();
    let end = node.metadata().map_or(0, |m| m.size);
    let _ = node.write(end, &rec);
}

/// The record's bytes (`struct acct_v3`, native byte order).
fn record(acct: &Acct, p: &Process, exit_code: u32) -> [u8; 64] {
    let mut r = [0u8; 64];
    let mut flag = 0u8;
    if !p.did_exec.load(core::sync::atomic::Ordering::Acquire) {
        flag |= AFORK;
    }
    if exit_code & 0x7f != 0 {
        flag |= AXSIG;
    }
    if exit_code & 0x80 != 0 {
        flag |= ACORE;
    }
    r[0] = flag;
    r[1] = 3; // ACCT_VERSION, little-endian (ACCT_BYTEORDER clear)
              // ac_tty: no controlling terminals yet.
    let c = p.credentials();
    let ns_user = &acct.opener.userns;
    let pid = crate::process::pidns::nr_in(p.pid.0, &acct.ns) as u32;
    let ppid = p
        .parent()
        .map_or(0, |pp| crate::process::pidns::nr_in(pp.0, &acct.ns) as u32);
    let elapsed_ns = crate::timer::monotonic_ns().saturating_sub(p.started_ns);
    let elapsed = elapsed_ns / (1_000_000_000 / AHZ);
    let now_s = u64::try_from(crate::timer::realtime::now_ns() / 1_000_000_000).unwrap_or(0);
    let btime = now_s.saturating_sub(elapsed / AHZ).min(u64::from(u32::MAX)) as u32;
    let (_, (user_ns, sys_ns)) = p.cpu_split();
    let put32 =
        |r: &mut [u8; 64], at: usize, v: u32| r[at..at + 4].copy_from_slice(&v.to_ne_bytes());
    let put16 =
        |r: &mut [u8; 64], at: usize, v: u16| r[at..at + 2].copy_from_slice(&v.to_ne_bytes());
    put32(&mut r, 4, exit_code);
    put32(&mut r, 8, ns_user.uid_munged(c.ruid));
    put32(&mut r, 12, ns_user.gid_munged(c.rgid));
    put32(&mut r, 16, pid);
    put32(&mut r, 20, ppid);
    put32(&mut r, 24, btime);
    put32(&mut r, 28, encode_float(elapsed));
    put16(&mut r, 32, encode_comp_t(user_ns / (1_000_000_000 / AHZ)));
    put16(&mut r, 34, encode_comp_t(sys_ns / (1_000_000_000 / AHZ)));
    // ac_mem, ac_io, ac_rw, ac_minflt, ac_majflt, ac_swaps: not counted.
    let comm = crate::process::current_thread().map(|t| *t.comm.lock());
    if let Some(comm) = comm {
        let n = comm
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(comm.len())
            .min(16);
        r[48..48 + n].copy_from_slice(&comm[..n]);
    }
    r
}

/// encode_comp_t: a 13-bit mantissa and a 3-bit base-8 exponent, rounded.
fn encode_comp_t(mut value: u64) -> u16 {
    const MANTSIZE: u32 = 13;
    const EXPSIZE: u32 = 3;
    const MAXFRACT: u64 = (1 << MANTSIZE) - 1;
    let (mut exp, mut rnd) = (0u32, 0u64);
    while value > MAXFRACT {
        rnd = value & (1 << (EXPSIZE - 1));
        value >>= EXPSIZE;
        exp += 1;
    }
    if rnd != 0 {
        value += 1;
        if value > MAXFRACT {
            value >>= EXPSIZE;
            exp += 1;
        }
    }
    if exp > (0xffff >> MANTSIZE) {
        return 0xffff;
    }
    ((exp << MANTSIZE) as u64 + value) as u16
}

/// encode_float: `value` as an IEEE 754 single (ac_etime).
fn encode_float(mut value: u64) -> u32 {
    if value == 0 {
        return 0;
    }
    let mut exp: u32 = 190;
    while (value as i64) > 0 {
        value <<= 1;
        exp -= 1;
    }
    let u = ((value >> 40) as u32) & 0x7f_ffff;
    u | (exp << 23)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The encodings Linux's records use.
    #[test]
    fn comp_t_and_float_encode_as_linux() {
        assert_eq!(encode_comp_t(0), 0);
        assert_eq!(encode_comp_t(8191), 8191);
        // 8192 = 1024 * 8: mantissa 1024, exponent 1.
        assert_eq!(encode_comp_t(8192), (1 << 13) | 1024);
        assert_eq!(encode_comp_t(u64::MAX), 0xffff);
        assert_eq!(encode_float(0), 0);
        assert_eq!(f32::from_bits(encode_float(1)), 1.0);
        assert_eq!(f32::from_bits(encode_float(100)), 100.0);
        assert_eq!(f32::from_bits(encode_float(12345)), 12345.0);
    }
}

//! The system tier's smaller calls (v0.28 item 5, ADR 0027): vhangup,
//! ioperm and iopl, syslog, and reboot.

use super::{SyscallError, SyscallResult};
use crate::process::{creds::Caller, linux_caps};

/// vhangup(): hang up the caller's controlling terminal (Linux's
/// tty_vhangup_self), which needs CAP_SYS_TTY_CONFIG (EPERM). No process
/// has a controlling terminal yet (the terminal work of v0.29, MASTER_TODO
/// item 10), and for one without, Linux does nothing and returns 0.
pub fn sys_vhangup() -> SyscallResult {
    if !Caller::current().capable(linux_caps::CAP_SYS_TTY_CONFIG) {
        return Err(SyscallError::OperationNotPermitted);
    }
    Ok(0)
}

/// The I/O port space (IO_BITMAP_BITS).
const IO_PORTS: usize = 65536;

/// ioperm(from, num, turn_on) (x86_64). Ports are reached only through an
/// IoPort capability naming them exactly (ADR 0027, item 5 decisions), and
/// none is granted yet (the device capabilities of Track M, M2, with the
/// TSS I/O bitmap): turning ports on is EPERM even with CAP_SYS_RAWIO, and
/// the PCI configuration ports 0xCF8 to 0xCFF are never given. Turning
/// them off, which needs no privilege, has nothing to take away.
pub fn sys_ioperm(from: usize, num: usize, turn_on: usize) -> SyscallResult {
    let end = from.checked_add(num).ok_or(SyscallError::InvalidArgument)?;
    if end <= from || end > IO_PORTS {
        return Err(SyscallError::InvalidArgument);
    }
    if turn_on as u32 != 0 {
        return Err(SyscallError::OperationNotPermitted);
    }
    Ok(0)
}

/// iopl(level) (x86_64), as Linux's emulation: level 3 opens every port,
/// which no capability allows (EPERM); levels 1 and 2 open none and need
/// CAP_SYS_RAWIO to raise to, as on Linux; above 3 is EINVAL. No thread is
/// above level 0, so lowering is always to 0.
pub fn sys_iopl(level: usize) -> SyscallResult {
    match level as u32 {
        0 => Ok(0),
        1 | 2 if Caller::current().capable(linux_caps::CAP_SYS_RAWIO) => Ok(0),
        1..=3 => Err(SyscallError::OperationNotPermitted),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// `/proc/sys/kernel/dmesg_restrict`: on (1), reading the kernel log needs
/// CAP_SYSLOG for everyone. Linux's built-in default is 0; VeridianOS
/// starts at 1, as Debian's and Ubuntu's kernels do, because the log names
/// kernel addresses (ADR 0027 decision 29).
pub static DMESG_RESTRICT: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(true);

/// syslog's actions (SYSLOG_ACTION_*).
mod action {
    pub const CLOSE: u32 = 0;
    pub const OPEN: u32 = 1;
    pub const READ: u32 = 2;
    pub const READ_ALL: u32 = 3;
    pub const READ_CLEAR: u32 = 4;
    pub const CLEAR: u32 = 5;
    pub const CONSOLE_OFF: u32 = 6;
    pub const CONSOLE_ON: u32 = 7;
    pub const CONSOLE_LEVEL: u32 = 8;
    pub const SIZE_UNREAD: u32 = 9;
    pub const SIZE_BUFFER: u32 = 10;
}

/// syslog(type, buf, len), as Linux's do_syslog: CAP_SYSLOG for every
/// action but READ_ALL and SIZE_BUFFER (for those too with
/// dmesg_restrict), and with it authority over the `System` object (ADR
/// 0024), checked first (EPERM); then the action's own arguments (EINVAL,
/// EFAULT). READ waits for unread text, interruptibly.
pub fn sys_syslog(ty: usize, buf: usize, len: usize) -> SyscallResult {
    use core::sync::atomic::Ordering;

    use crate::printk;
    let ty = ty as u32;
    let len = len as u32 as i32;
    let restricted = DMESG_RESTRICT.load(Ordering::Relaxed)
        || (ty != action::READ_ALL && ty != action::SIZE_BUFFER);
    if restricted && !Caller::current().capable(linux_caps::CAP_SYSLOG) {
        return Err(SyscallError::OperationNotPermitted);
    }
    // ADR 0024: the kernel log is the `System` object's, so an action that
    // needs CAP_SYSLOG needs authority over it too: READ to read it, MODIFY
    // to clear it or change the console (EPERM). An unprivileged READ_ALL
    // with dmesg_restrict off needs neither, as on Linux (PR #49 review).
    #[cfg(feature = "alloc")]
    if restricted {
        let changes = matches!(
            ty,
            action::READ
                | action::READ_CLEAR
                | action::CLEAR
                | action::CONSOLE_OFF
                | action::CONSOLE_ON
                | action::CONSOLE_LEVEL
        );
        let held = if changes {
            crate::cap::authority::current_may_change_system()
        } else {
            crate::cap::authority::current_may_read_system()
        };
        if !held {
            return Err(SyscallError::OperationNotPermitted);
        }
    }
    let reads = |n: i32| -> Result<Option<usize>, SyscallError> {
        if buf == 0 || n < 0 {
            return Err(SyscallError::InvalidArgument);
        }
        if n == 0 {
            return Ok(None);
        }
        super::validate_user_buffer(buf, n as usize)?;
        Ok(Some(n as usize))
    };
    match ty {
        action::CLOSE | action::OPEN => Ok(0),
        action::CLEAR => {
            printk::clear();
            Ok(0)
        }
        action::READ => {
            let Some(n) = reads(len)? else {
                return Ok(0);
            };
            // No read gives more than the ring holds: the buffer is that at
            // most, made once (PR #49 review).
            let mut out = super::userspace::zeroed_buf(n.min(printk::size_buffer()))?;
            loop {
                let got = printk::read(&mut out);
                if got > 0 {
                    super::userspace::write_user_bytes(buf, &out[..got])?;
                    return Ok(got);
                }
                let deadline = crate::timer::monotonic_ns().saturating_add(10_000_000);
                match crate::sched::dispatch::wait_event(
                    &crate::sched::dispatch::IO_EVENTS,
                    Some(deadline),
                    || printk::size_unread() > 0,
                ) {
                    Err(crate::sched::dispatch::WaitError::Interrupted) => {
                        return Err(SyscallError::Interrupted)
                    }
                    _ => continue,
                }
            }
        }
        action::READ_ALL | action::READ_CLEAR => {
            let Some(n) = reads(len)? else {
                return Ok(0);
            };
            let mut out = super::userspace::zeroed_buf(n.min(printk::size_buffer()))?;
            let (got, end) = printk::read_all(&mut out);
            // Cleared only once the caller has the text: a copy that fails
            // leaves it for a retry (PR #49 review).
            super::userspace::write_user_bytes(buf, &out[..got])?;
            if ty == action::READ_CLEAR {
                printk::clear_to(end);
            }
            Ok(got)
        }
        action::CONSOLE_OFF => {
            printk::console_off();
            Ok(0)
        }
        action::CONSOLE_ON => {
            printk::console_on();
            Ok(0)
        }
        action::CONSOLE_LEVEL => {
            if !(1..=8).contains(&len) {
                return Err(SyscallError::InvalidArgument);
            }
            printk::console_level(len as u8);
            Ok(0)
        }
        action::SIZE_UNREAD => Ok(printk::size_unread()),
        action::SIZE_BUFFER => Ok(printk::size_buffer()),
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// reboot(2)'s magic numbers (LINUX_REBOOT_MAGIC1, MAGIC2, 2A, 2B, 2C).
const REBOOT_MAGIC1: u32 = 0xfee1_dead;
const REBOOT_MAGIC2: [u32; 4] = [672_274_793, 85_072_278, 369_367_448, 537_993_216];

/// reboot(2)'s commands (LINUX_REBOOT_CMD_*).
mod reboot_cmd {
    pub const RESTART: u32 = 0x0123_4567;
    pub const HALT: u32 = 0xcdef_0123;
    pub const CAD_ON: u32 = 0x89ab_cdef;
    pub const CAD_OFF: u32 = 0;
    pub const POWER_OFF: u32 = 0x4321_fedc;
    pub const RESTART2: u32 = 0xa1b2_c3d4;
}

/// Ctrl-Alt-Del restarts at once (1) or signals init (0): Linux's C_A_D,
/// set by reboot's CAD_ON and CAD_OFF. Nothing reads it yet: the keyboard
/// does not report the combination (KNOWN-LIMITATIONS).
pub static CTRL_ALT_DEL: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// reboot(magic1, magic2, cmd, arg), as Linux 7.3's kernel/reboot.c:
/// CAP_SYS_BOOT in the user namespace owning the caller's PID namespace
/// (EPERM), then the magic numbers (EINVAL).
///
/// In a PID namespace below the initial one (reboot_pid_ns), which also
/// needs MODIFY on that namespace (ADR 0024; EPERM), only RESTART,
/// RESTART2, POWER_OFF and HALT are taken (EINVAL otherwise): the
/// namespace's init is killed and reported killed by SIGHUP (restart) or
/// SIGINT (halt, power off), and the calling thread exits.
///
/// In the initial one the call changes the machine, which also needs
/// MODIFY on the `System` object (ADR 0024; EPERM). RESTART and RESTART2
/// (with a command string of up to 255 bytes, EFAULT if unreadable) reset
/// the machine, HALT halts it, POWER_OFF powers it off (halting where it
/// cannot), CAD_ON and CAD_OFF set [`CTRL_ALT_DEL`]. As on Linux nothing is
/// synced first: that is the caller's (sync(2)). KEXEC and SW_SUSPEND are
/// EINVAL, as a Linux built without kexec or hibernation.
pub fn sys_reboot(magic1: usize, magic2: usize, cmd: usize, arg: usize) -> SyscallResult {
    use core::sync::atomic::Ordering;
    let process = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let ns = process.pid_ns();
    if !Caller::current().ns_capable(&ns.owner, linux_caps::CAP_SYS_BOOT) {
        return Err(SyscallError::OperationNotPermitted);
    }
    if magic1 as u32 != REBOOT_MAGIC1 || !REBOOT_MAGIC2.contains(&(magic2 as u32)) {
        return Err(SyscallError::InvalidArgument);
    }
    let cmd = cmd as u32;
    if !ns.is_initial() {
        // ADR 0024: ending a PID namespace kills its init, which needs
        // authority over that namespace as well as CAP_SYS_BOOT (EPERM; PR
        // #49 review).
        #[cfg(feature = "alloc")]
        if !crate::cap::authority::current_holds_namespace(
            &crate::process::ns::NsRef::Pid(ns.clone()),
            crate::cap::Rights::MODIFY,
        ) {
            return Err(SyscallError::OperationNotPermitted);
        }
        // The thread exits there without unwinding this frame: nothing it
        // owns may be left on it (PR #49 review).
        drop(process);
        return reboot_pid_ns(ns, cmd);
    }
    if !crate::cap::authority::current_may_change_system() {
        return Err(SyscallError::OperationNotPermitted);
    }
    match cmd {
        reboot_cmd::RESTART => crate::arch::power::restart(None),
        reboot_cmd::RESTART2 => {
            let mut buf = [0u8; 255];
            let n = super::userspace::strncpy_from_user(arg, &mut buf)
                .map_err(|_| SyscallError::InvalidPointer)?;
            let text = core::str::from_utf8(&buf[..n]).unwrap_or("?");
            crate::arch::power::restart(Some(text))
        }
        reboot_cmd::HALT => crate::arch::power::halt(),
        reboot_cmd::POWER_OFF if crate::arch::power::can_power_off() => {
            crate::arch::power::power_off()
        }
        reboot_cmd::POWER_OFF => crate::arch::power::halt(),
        reboot_cmd::CAD_ON => {
            CTRL_ALT_DEL.store(true, Ordering::Relaxed);
            Ok(0)
        }
        reboot_cmd::CAD_OFF => {
            CTRL_ALT_DEL.store(false, Ordering::Relaxed);
            Ok(0)
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// reboot_pid_ns: record the signal the namespace's init is reported
/// killed by, kill it, and end the calling thread (do_exit(0)).
fn reboot_pid_ns(ns: alloc::sync::Arc<crate::process::pidns::PidNs>, cmd: u32) -> SyscallResult {
    use crate::process::{exit, signals::SigInfo};
    let sig = match cmd {
        reboot_cmd::RESTART | reboot_cmd::RESTART2 => exit::signals::SIGHUP,
        reboot_cmd::POWER_OFF | reboot_cmd::HALT => exit::signals::SIGINT,
        _ => return Err(SyscallError::InvalidArgument),
    };
    ns.set_reboot(sig as u32);
    if let Some(init) = ns.reaper() {
        let _ = exit::kill_process_info(
            crate::process::ProcessId(init),
            exit::signals::SIGKILL,
            SigInfo::kernel(exit::signals::SIGKILL as usize),
        );
    }
    // exit_thread does not return to this frame: the namespace reference
    // goes first, or the namespace (and its PID quota) would never be
    // freed.
    drop(ns);
    crate::process::exit_thread(0);
    Ok(0)
}

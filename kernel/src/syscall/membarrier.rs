//! membarrier(2) (v0.28 item 5, ADR 0027; Linux's
//! `kernel/sched/membarrier.c`). Every expedited barrier interrupts each
//! other online CPU and waits for it (`sched::dispatch::barrier_cpus`),
//! whose interrupt runs a full barrier and returns through a core
//! serialising instruction, which serves SYNC_CORE as well.
//! MEMBARRIER_CMD_GLOBAL (Linux's synchronize_rcu) is the same barrier. The
//! RSEQ commands are EINVAL, as on a Linux built without CONFIG_RSEQ: rseq(2)
//! is not implemented.

use super::{SyscallError, SyscallResult};

pub const CMD_QUERY: u32 = 0;
pub const CMD_GLOBAL: u32 = 1;
pub const CMD_GLOBAL_EXPEDITED: u32 = 1 << 1;
pub const CMD_REGISTER_GLOBAL_EXPEDITED: u32 = 1 << 2;
pub const CMD_PRIVATE_EXPEDITED: u32 = 1 << 3;
pub const CMD_REGISTER_PRIVATE_EXPEDITED: u32 = 1 << 4;
pub const CMD_PRIVATE_EXPEDITED_SYNC_CORE: u32 = 1 << 5;
pub const CMD_REGISTER_PRIVATE_EXPEDITED_SYNC_CORE: u32 = 1 << 6;
pub const CMD_PRIVATE_EXPEDITED_RSEQ: u32 = 1 << 7;
pub const CMD_REGISTER_PRIVATE_EXPEDITED_RSEQ: u32 = 1 << 8;
pub const CMD_GET_REGISTRATIONS: u32 = 1 << 9;
/// The one flag (with PRIVATE_EXPEDITED_RSEQ only): `cpu_id` is valid.
pub const CMD_FLAG_CPU: u32 = 1;

/// What QUERY reports: every command that works here (Linux's
/// MEMBARRIER_CMD_BITMASK with SYNC_CORE and without RSEQ): 639.
pub const SUPPORTED: u32 = CMD_GLOBAL
    | CMD_GLOBAL_EXPEDITED
    | CMD_REGISTER_GLOBAL_EXPEDITED
    | CMD_PRIVATE_EXPEDITED
    | CMD_REGISTER_PRIVATE_EXPEDITED
    | CMD_PRIVATE_EXPEDITED_SYNC_CORE
    | CMD_REGISTER_PRIVATE_EXPEDITED_SYNC_CORE
    | CMD_GET_REGISTRATIONS;

/// The registrations GET_REGISTRATIONS reports.
const REGISTRATIONS: u32 = CMD_REGISTER_GLOBAL_EXPEDITED
    | CMD_REGISTER_PRIVATE_EXPEDITED
    | CMD_REGISTER_PRIVATE_EXPEDITED_SYNC_CORE
    | CMD_REGISTER_PRIVATE_EXPEDITED_RSEQ;

/// What a command does, given the process's registrations (`registered`):
/// the check part of `sys_membarrier`, without the barrier.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Return this value, nothing else to do.
    Value(usize),
    /// Add these registration bits, return 0.
    Register(u32),
    /// Run the barrier, return 0.
    Barrier,
}

/// membarrier(cmd, flags, cpu_id)'s checks, in Linux's order: flags (EINVAL
/// for any but FLAG_CPU with PRIVATE_EXPEDITED_RSEQ, and that command is
/// not built: EINVAL too); an unknown command EINVAL; a private expedited
/// command without its registration EPERM.
pub fn decide(cmd: u32, flags: u32, registered: u32) -> Result<Action, SyscallError> {
    let flags_ok = if cmd == CMD_PRIVATE_EXPEDITED_RSEQ {
        flags & !CMD_FLAG_CPU == 0
    } else {
        flags == 0
    };
    if !flags_ok {
        return Err(SyscallError::InvalidArgument);
    }
    let needs = |reg: u32| {
        if registered & reg != 0 {
            Ok(Action::Barrier)
        } else {
            Err(SyscallError::OperationNotPermitted)
        }
    };
    match cmd {
        CMD_QUERY => Ok(Action::Value(SUPPORTED as usize)),
        CMD_GLOBAL | CMD_GLOBAL_EXPEDITED => Ok(Action::Barrier),
        CMD_REGISTER_GLOBAL_EXPEDITED
        | CMD_REGISTER_PRIVATE_EXPEDITED
        | CMD_REGISTER_PRIVATE_EXPEDITED_SYNC_CORE => Ok(Action::Register(cmd)),
        CMD_PRIVATE_EXPEDITED => needs(CMD_REGISTER_PRIVATE_EXPEDITED),
        CMD_PRIVATE_EXPEDITED_SYNC_CORE => needs(CMD_REGISTER_PRIVATE_EXPEDITED_SYNC_CORE),
        CMD_GET_REGISTRATIONS => Ok(Action::Value((registered & REGISTRATIONS) as usize)),
        // RSEQ (not built) and anything unknown.
        _ => Err(SyscallError::InvalidArgument),
    }
}

/// membarrier(cmd, flags, cpu_id). The registrations are the process's
/// (its threads share them), kept by fork and cleared by exec.
pub fn sys_membarrier(cmd: usize, flags: usize, _cpu_id: usize) -> SyscallResult {
    let proc = crate::process::current_process().ok_or(SyscallError::InvalidState)?;
    let vas = proc.memory_space.lock();
    let registered = vas.mm_state().membarrier;
    match decide(cmd as u32, flags as u32, registered)? {
        Action::Value(v) => Ok(v),
        Action::Register(bits) => {
            vas.update_mm_state(|m| m.membarrier |= bits);
            drop(vas);
            // Linux syncs the run queues after registering, so a later
            // command finds every thread: the barrier does that here.
            crate::sched::dispatch::barrier_cpus();
            Ok(0)
        }
        Action::Barrier => {
            drop(vas);
            crate::sched::dispatch::barrier_cpus();
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_reports_exactly_what_works() {
        assert_eq!(decide(CMD_QUERY, 0, 0), Ok(Action::Value(639)));
        assert_eq!(
            decide(CMD_QUERY, 1, 0),
            Err(SyscallError::InvalidArgument),
            "flags"
        );
    }

    #[test]
    fn private_commands_need_their_registration() {
        assert_eq!(
            decide(CMD_PRIVATE_EXPEDITED, 0, 0),
            Err(SyscallError::OperationNotPermitted)
        );
        assert_eq!(
            decide(CMD_PRIVATE_EXPEDITED, 0, CMD_REGISTER_PRIVATE_EXPEDITED),
            Ok(Action::Barrier)
        );
        assert_eq!(
            decide(
                CMD_PRIVATE_EXPEDITED_SYNC_CORE,
                0,
                CMD_REGISTER_PRIVATE_EXPEDITED
            ),
            Err(SyscallError::OperationNotPermitted),
            "SYNC_CORE has its own registration"
        );
        assert_eq!(
            decide(CMD_REGISTER_PRIVATE_EXPEDITED, 0, 0),
            Ok(Action::Register(CMD_REGISTER_PRIVATE_EXPEDITED))
        );
        assert_eq!(decide(CMD_GLOBAL, 0, 0), Ok(Action::Barrier));
        assert_eq!(
            decide(CMD_GET_REGISTRATIONS, 0, CMD_REGISTER_GLOBAL_EXPEDITED | 1),
            Ok(Action::Value(CMD_REGISTER_GLOBAL_EXPEDITED as usize))
        );
    }

    #[test]
    fn rseq_and_unknown_commands_are_einval() {
        for cmd in [
            CMD_PRIVATE_EXPEDITED_RSEQ,
            CMD_REGISTER_PRIVATE_EXPEDITED_RSEQ,
            3,
            1 << 10,
        ] {
            assert_eq!(decide(cmd, 0, u32::MAX), Err(SyscallError::InvalidArgument));
        }
        assert_eq!(
            decide(CMD_PRIVATE_EXPEDITED_RSEQ, CMD_FLAG_CPU, u32::MAX),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            decide(CMD_PRIVATE_EXPEDITED_RSEQ, 2, 0),
            Err(SyscallError::InvalidArgument)
        );
    }
}

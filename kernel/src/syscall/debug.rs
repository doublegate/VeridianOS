//! Debug and tracing system calls
//!
//! Provides the `ptrace` syscall (140) for process tracing and debugging.
//! Used by debuggers (gdb, lldb) to inspect and control other processes.
//!
//! Currently implemented:
//! - TRACEME: Mark process as traceable (accepted, no enforcement yet)
//! - PEEKTEXT/PEEKDATA: Read a word from tracee's address space via VAS
//! - POKETEXT/POKEDATA: Write a word to tracee's address space via VAS
//!
//! Deferred (requires scheduler integration):
//! - GETREGS/SETREGS: Read/write register state
//! - ATTACH/DETACH: Tracer relationship management
//! - CONT/SINGLESTEP: Resume control

use super::{SyscallError, SyscallResult};
use crate::{mm::VirtualAddress, process};

// ============================================================================
// Ptrace request codes (matching POSIX/Linux conventions)
// ============================================================================

/// Ptrace operation to perform.
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtraceRequest {
    /// Allow the current process to be traced by its parent.
    TraceMe = 0,
    /// Read a word from the tracee's memory at `addr`.
    PeekText = 1,
    /// Read a word from the tracee's data segment at `addr`.
    PeekData = 2,
    /// Write a word to the tracee's memory at `addr`.
    PokeText = 4,
    /// Write a word to the tracee's data segment at `addr`.
    PokeData = 5,
    /// Read the tracee's general-purpose register set.
    GetRegs = 12,
    /// Write the tracee's general-purpose register set.
    SetRegs = 13,
    /// Attach to a running process (become its tracer).
    Attach = 16,
    /// Detach from a tracee, optionally delivering a signal.
    Detach = 17,
    /// Resume the tracee, optionally delivering a signal.
    Continue = 7,
    /// Execute a single instruction in the tracee.
    SingleStep = 9,
}

impl TryFrom<usize> for PtraceRequest {
    type Error = ();

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(PtraceRequest::TraceMe),
            1 => Ok(PtraceRequest::PeekText),
            2 => Ok(PtraceRequest::PeekData),
            4 => Ok(PtraceRequest::PokeText),
            5 => Ok(PtraceRequest::PokeData),
            7 => Ok(PtraceRequest::Continue),
            9 => Ok(PtraceRequest::SingleStep),
            12 => Ok(PtraceRequest::GetRegs),
            13 => Ok(PtraceRequest::SetRegs),
            16 => Ok(PtraceRequest::Attach),
            17 => Ok(PtraceRequest::Detach),
            _ => Err(()),
        }
    }
}

// ============================================================================
// Helper: read/write a word from another process's address space
// ============================================================================

/// Read a usize-sized word from the target process's virtual address space.
///
/// Looks up the target process via the process table, finds the VAS mapping
/// containing the target address, locates the backing physical frame, and
/// reads the word from the kernel's physical memory window.
fn ptrace_peek(
    target_pid: process::ProcessId,
    caller_pid: u64,
    addr: usize,
) -> Result<usize, SyscallError> {
    let target = process::find_process(target_pid).ok_or(SyscallError::ProcessNotFound)?;
    let memory_space = target.memory_space.lock();
    still_traced(&target, caller_pid)?;

    // Find the mapping that contains this address: EIO for none, as
    // Linux's ptrace_access_vm, and for device memory, whose reads can
    // have side effects and which the physical map need not cover.
    let mapping = memory_space
        .find_mapping(VirtualAddress(addr as u64))
        .ok_or(SyscallError::IoError)?;
    if mapping.mapping_type == crate::mm::vas::MappingType::Device {
        return Err(SyscallError::IoError);
    }

    // Calculate which page and offset within the mapping
    let page_offset_in_mapping = (addr as u64 - mapping.start.0) as usize;
    let page_index = page_offset_in_mapping / 4096;
    let offset_in_page = page_offset_in_mapping % 4096;

    // Verify we have physical frames recorded
    if page_index >= mapping.physical_frames.len() {
        return Err(SyscallError::IoError);
    }

    // Get the physical frame number and compute the physical address
    let frame = mapping.physical_frames[page_index];
    let phys_addr = frame.as_u64() as usize * 4096 + offset_in_page;

    // Read the word from the physical address via identity mapping
    // On x86_64, physical memory is mapped at 0xFFFF_8000_0000_0000.
    // On AArch64/RISC-V, physical memory is identity-mapped during boot.
    let kernel_vaddr = phys_to_kernel_vaddr(phys_addr);

    // ptrace semantics allow unaligned reads in practice, but we require
    // alignment here for safety.
    if !kernel_vaddr.is_multiple_of(core::mem::align_of::<usize>()) {
        return Err(SyscallError::InvalidArgument);
    }

    // SAFETY: The physical address was obtained from a valid VAS mapping
    // with allocated frames. The kernel virtual address is the kernel's
    // identity/offset mapping of physical memory, and it is usize-aligned
    // (checked above).
    let value = unsafe { *(kernel_vaddr as *const usize) };
    Ok(value)
}

/// Write a usize-sized word to the target process's virtual address space.
///
/// Into the tracee only: a page it still shares with another process after
/// fork (copy-on-write, or read-only like its code) is copied first. The
/// word was written into the shared frame itself, so a breakpoint set in a
/// child also landed in its parent. Read-only private pages may be written,
/// as ptrace allows (breakpoints in code); kernel addresses, device memory
/// and shared pages the tracee may not write are refused.
fn ptrace_poke(
    target_pid: process::ProcessId,
    caller_pid: u64,
    addr: usize,
    value: usize,
) -> Result<(), SyscallError> {
    let target = process::find_process(target_pid).ok_or(SyscallError::ProcessNotFound)?;
    let memory_space = target.memory_space.lock();
    still_traced(&target, caller_pid)?;
    #[cfg(feature = "alloc")]
    {
        memory_space
            .write_bytes_private(addr as u64, &value.to_ne_bytes(), true)
            // EIO for an unmapped or refused address, as Linux's
            // ptrace_access_vm.
            .map_err(|_| SyscallError::IoError)
    }
    #[cfg(not(feature = "alloc"))]
    {
        let _ = (memory_space, addr, value);
        Err(SyscallError::InvalidState)
    }
}

/// Convert a physical address to a kernel virtual address.
///
/// Uses the architecture-specific physical memory mapping offset.
fn phys_to_kernel_vaddr(phys_addr: usize) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // x86_64: through the physical map, wherever the bootloader put it.
        crate::mm::phys_to_virt_addr(phys_addr as u64) as usize
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64 QEMU virt: physical memory is identity-mapped
        phys_addr
    }
    #[cfg(target_arch = "riscv64")]
    {
        // RISC-V QEMU virt: physical memory is identity-mapped
        phys_addr
    }
}

// ============================================================================
// Syscall implementation
// ============================================================================

/// Process trace syscall (syscall 140).
///
/// Provides debugger-level control over another process. The tracer must
/// either be the parent of the tracee (via TRACEME) or attach to a running
/// process (via ATTACH).
///
/// # Arguments
/// - `request`: Ptrace operation (see [`PtraceRequest`]).
/// - `pid`: Target process ID (ignored for TRACEME).
/// - `addr`: Address in tracee's address space (request-specific).
/// - `data`: Data to write or buffer for results (request-specific).
///
/// # Returns
/// Request-specific value on success, or error.
/// Whether a process may attach to `target`: never to itself or to a
/// process that already has a tracer; and unless it holds CAP_SYS_PTRACE,
/// the target's real, effective and saved IDs must all be the caller's real
/// ones (Linux `ptrace_may_access`), so a setuid program running
/// privileged cannot be traced.
fn may_attach(
    caller_pid: u64,
    caller: &process::creds::Caller,
    target_pid: u64,
    target: &process::creds::Credentials,
    target_tracer: u64,
    target_dumpable: bool,
) -> Result<(), SyscallError> {
    if target_pid == caller_pid
        || target_tracer != 0
        || !may_access(caller, target, target_dumpable)
    {
        return Err(SyscallError::OperationNotPermitted);
    }
    Ok(())
}

/// Linux `ptrace_may_access`: the caller holds CAP_SYS_PTRACE, or the
/// target's real, effective and saved IDs are all the caller's real ones
/// (so a setuid program running privileged is out of reach) and the target
/// is dumpable (PR_SET_DUMPABLE). Also guards get_robust_list.
pub(super) fn may_access(
    caller: &process::creds::Caller,
    target: &process::creds::Credentials,
    target_dumpable: bool,
) -> bool {
    let c = &caller.creds;
    let same_user = [target.ruid, target.euid, target.suid]
        .iter()
        .all(|&id| id == c.ruid)
        && [target.rgid, target.egid, target.sgid]
            .iter()
            .all(|&id| id == c.rgid);
    (same_user && target_dumpable) || caller.capable(process::linux_caps::CAP_SYS_PTRACE)
}

/// Checked again with the target's address space locked (the lock exec's
/// clear takes): the caller still traces it -- a detach by another thread
/// of the tracer may have come in between -- and no exec is replacing the
/// image, whose new set-ID program a tracer must not write before the
/// set-ID decision (PR #23 review). ESRCH, as for a process not traced.
fn still_traced(target: &process::Process, caller_pid: u64) -> Result<(), SyscallError> {
    use core::sync::atomic::Ordering;
    if target.tracer.load(Ordering::Acquire) != caller_pid
        || target.exec_in_progress.load(Ordering::Acquire)
    {
        return Err(SyscallError::ProcessNotFound);
    }
    Ok(())
}

/// The process `pid` if the caller is its tracer; ESRCH otherwise, as
/// Linux answers requests about a process the caller does not trace.
/// Reading or writing another process's memory needs this relationship;
/// any process could peek and poke any pid.
fn traced_by_caller(
    pid: usize,
    caller_pid: u64,
) -> Result<alloc::sync::Arc<process::Process>, SyscallError> {
    let target = process::find_process(process::ProcessId(pid as u64))
        .ok_or(SyscallError::ProcessNotFound)?;
    if target.tracer.load(core::sync::atomic::Ordering::Acquire) != caller_pid {
        return Err(SyscallError::ProcessNotFound);
    }
    Ok(target)
}

pub fn sys_ptrace(request: usize, pid: usize, addr: usize, data: usize) -> SyscallResult {
    use core::sync::atomic::Ordering;

    let req = PtraceRequest::try_from(request).map_err(|_| SyscallError::InvalidArgument)?;
    let caller = process::current_process().ok_or(SyscallError::InvalidState)?;
    let caller_pid = caller.pid.0;

    match req {
        PtraceRequest::TraceMe => {
            // The parent becomes the tracer; a process can be traced once.
            let parent = caller.parent().ok_or(SyscallError::OperationNotPermitted)?;
            // The tracer's privilege is the parent's (Linux's ptrace_link
            // takes the new parent's credentials), here its main thread's.
            let parent_capable = process::find_process(parent).is_some_and(|p| {
                p.get_main_thread_id()
                    .and_then(|tid| p.get_thread(tid))
                    .is_some_and(|t| t.caps.lock().capable(process::linux_caps::CAP_SYS_PTRACE))
            });
            let _guard = caller.cred_guard.lock();
            caller
                .tracer
                .compare_exchange(0, parent.0, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| SyscallError::OperationNotPermitted)?;
            caller
                .tracer_capable
                .store(parent_capable, Ordering::Release);
            Ok(0)
        }

        PtraceRequest::Attach => {
            let target = process::find_process(process::ProcessId(pid as u64))
                .ok_or(SyscallError::ProcessNotFound)?;
            // Checked and attached under the target's credential guard, as
            // a set-ID exec decides under it: the target's credentials
            // cannot change between the check and the attach.
            let me = process::creds::Caller::current();
            let _guard = target.cred_guard.lock();
            may_attach(
                caller_pid,
                &me,
                target.pid.0,
                &target.credentials(),
                target.tracer.load(Ordering::Acquire),
                target.dumpable.load(Ordering::Acquire),
            )?;
            // Lost race with another tracer: EPERM, as above.
            target
                .tracer
                .compare_exchange(0, caller_pid, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| SyscallError::OperationNotPermitted)?;
            target.tracer_capable.store(
                me.capable(process::linux_caps::CAP_SYS_PTRACE),
                Ordering::Release,
            );
            Ok(0)
        }

        PtraceRequest::Detach => {
            let target = traced_by_caller(pid, caller_pid)?;
            // Under the guard an exec decides set-ID credentials under,
            // and only if still this caller's (another of its threads may
            // have detached and someone else attached since).
            let _guard = target.cred_guard.lock();
            target
                .tracer
                .compare_exchange(caller_pid, 0, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| SyscallError::ProcessNotFound)?;
            Ok(0)
        }

        PtraceRequest::PeekText | PtraceRequest::PeekData => {
            // The word goes to *data and the call returns 0: the raw
            // syscall's convention, which the C library wrappers unpack.
            let target = traced_by_caller(pid, caller_pid)?;
            let word = ptrace_peek(target.pid, caller_pid, addr)?;
            super::userspace::write_user::<usize>(data, word)?;
            Ok(0)
        }

        PtraceRequest::PokeText | PtraceRequest::PokeData => {
            let target = traced_by_caller(pid, caller_pid)?;
            ptrace_poke(target.pid, caller_pid, addr, data)?;
            Ok(0)
        }

        PtraceRequest::GetRegs => {
            // Read the tracee's register state into the tracer's buffer.
            // Requires: stopped tracee, ThreadContext access, user buffer
            // validation. Deferred until scheduler can stop/resume threads.
            let _target_pid = process::ProcessId(pid as u64);
            Err(SyscallError::InvalidSyscall)
        }

        PtraceRequest::SetRegs => {
            // Write the tracee's register state from the tracer's buffer.
            // Same requirements as GetRegs.
            let _target_pid = process::ProcessId(pid as u64);
            Err(SyscallError::InvalidSyscall)
        }

        PtraceRequest::Continue => {
            // Resume the stopped tracee, optionally delivering a signal.
            // Requires: clear single-step flag, deliver signal if non-zero,
            // set tracee to Ready state via scheduler.
            let _target_pid = process::ProcessId(pid as u64);
            Err(SyscallError::InvalidSyscall)
        }

        PtraceRequest::SingleStep => {
            // Execute one instruction in the tracee, then stop.
            // Requires: architecture-specific single-step flag
            // (x86_64: TF in RFLAGS, AArch64: MDSCR_EL1.SS,
            // RISC-V: dcsr.step).
            let _target_pid = process::ProcessId(pid as u64);
            Err(SyscallError::InvalidSyscall)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ptrace ATTACH: not to oneself or an already traced process; only
    /// to a process whose IDs are all the caller's, unless the caller holds
    /// CAP_SYS_PTRACE.
    #[test]
    fn attach_needs_same_ids_or_cap_sys_ptrace_and_no_tracer() {
        use process::{
            creds::{Caller, Credentials},
            linux_caps::LinuxCaps,
        };
        let user = Caller {
            creds: Credentials::new(1000, 100),
            caps: LinuxCaps::EMPTY,
        };
        let root = Caller::KERNEL;
        let bare_root = Caller {
            creds: Credentials::new(0, 0),
            caps: LinuxCaps::EMPTY,
        };
        let other = Credentials::new(1001, 100);
        let mut setuid_root = Credentials::new(1000, 100);
        setuid_root.euid = 0;
        setuid_root.suid = 0;
        let me = user.creds;
        assert_eq!(may_attach(10, &user, 20, &me, 0, true), Ok(()));
        assert_eq!(may_attach(10, &root, 20, &me, 0, true), Ok(()));
        let eperm = Err(SyscallError::OperationNotPermitted);
        assert_eq!(may_attach(10, &user, 20, &other, 0, true), eperm);
        assert_eq!(may_attach(10, &user, 20, &setuid_root, 0, true), eperm);
        assert_eq!(may_attach(10, &user, 10, &me, 0, true), eperm);
        assert_eq!(may_attach(10, &root, 20, &me, 30, true), eperm);
        // Uid 0 without CAP_SYS_PTRACE is an ordinary user.
        assert_eq!(may_attach(10, &bare_root, 20, &me, 0, true), eperm);
        // A process that made itself non-dumpable (PR_SET_DUMPABLE 0), or
        // exec'd a set-ID or unreadable program, only CAP_SYS_PTRACE may
        // trace.
        assert_eq!(may_attach(10, &user, 20, &me, 0, false), eperm);
        assert_eq!(may_attach(10, &root, 20, &me, 0, false), Ok(()));
    }
}

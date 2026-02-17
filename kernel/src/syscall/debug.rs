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
fn ptrace_peek(target_pid: process::ProcessId, addr: usize) -> Result<usize, SyscallError> {
    let target = process::find_process(target_pid).ok_or(SyscallError::ProcessNotFound)?;
    let memory_space = target.memory_space.lock();

    // Find the mapping that contains this address
    let mapping = memory_space
        .find_mapping(VirtualAddress(addr as u64))
        .ok_or(SyscallError::InvalidArgument)?;

    // Calculate which page and offset within the mapping
    let page_offset_in_mapping = (addr as u64 - mapping.start.0) as usize;
    let page_index = page_offset_in_mapping / 4096;
    let offset_in_page = page_offset_in_mapping % 4096;

    // Verify we have physical frames recorded
    if page_index >= mapping.physical_frames.len() {
        return Err(SyscallError::InvalidArgument);
    }

    // Get the physical frame number and compute the physical address
    let frame = mapping.physical_frames[page_index];
    let phys_addr = frame.as_u64() as usize * 4096 + offset_in_page;

    // Read the word from the physical address via identity mapping
    // On x86_64, physical memory is mapped at 0xFFFF_8000_0000_0000.
    // On AArch64/RISC-V, physical memory is identity-mapped during boot.
    let kernel_vaddr = phys_to_kernel_vaddr(phys_addr);

    // SAFETY: The physical address was obtained from a valid VAS mapping
    // with allocated frames. The kernel virtual address is the kernel's
    // identity/offset mapping of physical memory. We read a usize-aligned
    // word (ptrace semantics allow unaligned reads in practice, but we
    // require alignment here for safety).
    if !kernel_vaddr.is_multiple_of(core::mem::align_of::<usize>()) {
        return Err(SyscallError::InvalidArgument);
    }

    let value = unsafe { *(kernel_vaddr as *const usize) };
    Ok(value)
}

/// Write a usize-sized word to the target process's virtual address space.
fn ptrace_poke(
    target_pid: process::ProcessId,
    addr: usize,
    value: usize,
) -> Result<(), SyscallError> {
    let target = process::find_process(target_pid).ok_or(SyscallError::ProcessNotFound)?;
    let memory_space = target.memory_space.lock();

    let mapping = memory_space
        .find_mapping(VirtualAddress(addr as u64))
        .ok_or(SyscallError::InvalidArgument)?;

    let page_offset_in_mapping = (addr as u64 - mapping.start.0) as usize;
    let page_index = page_offset_in_mapping / 4096;
    let offset_in_page = page_offset_in_mapping % 4096;

    if page_index >= mapping.physical_frames.len() {
        return Err(SyscallError::InvalidArgument);
    }

    let frame = mapping.physical_frames[page_index];
    let phys_addr = frame.as_u64() as usize * 4096 + offset_in_page;
    let kernel_vaddr = phys_to_kernel_vaddr(phys_addr);

    if !kernel_vaddr.is_multiple_of(core::mem::align_of::<usize>()) {
        return Err(SyscallError::InvalidArgument);
    }

    // SAFETY: Same as ptrace_peek, but writing. The mapping must be writable
    // for this to be meaningful, but ptrace explicitly allows writing to
    // read-only pages (e.g., setting breakpoints in code sections).
    unsafe {
        *(kernel_vaddr as *mut usize) = value;
    }
    Ok(())
}

/// Convert a physical address to a kernel virtual address.
///
/// Uses the architecture-specific physical memory mapping offset.
fn phys_to_kernel_vaddr(phys_addr: usize) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // x86_64: physical memory is mapped at 0xFFFF_8000_0000_0000
        // via the bootloader's physical memory offset
        phys_addr + 0xFFFF_8000_0000_0000
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
pub fn sys_ptrace(request: usize, pid: usize, addr: usize, data: usize) -> SyscallResult {
    let req = PtraceRequest::try_from(request).map_err(|_| SyscallError::InvalidArgument)?;
    let _caller = process::current_process().ok_or(SyscallError::InvalidState)?;

    match req {
        PtraceRequest::TraceMe => {
            // Mark the current process as traceable by its parent.
            // Full tracer relationship enforcement requires a `traced_by`
            // field on the PCB and permission checks. For now, accept the
            // call -- user-space debuggers expect this to succeed.
            Ok(0)
        }

        PtraceRequest::PeekText | PtraceRequest::PeekData => {
            // Read a word from the tracee's address space via VAS.
            let target_pid = process::ProcessId(pid as u64);
            ptrace_peek(target_pid, addr)
        }

        PtraceRequest::PokeText | PtraceRequest::PokeData => {
            // Write a word to the tracee's address space via VAS.
            let target_pid = process::ProcessId(pid as u64);
            ptrace_poke(target_pid, addr, data)?;
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

        PtraceRequest::Attach => {
            // Become the tracer of an existing process.
            // Requires: permission check, tracer field on PCB, SIGSTOP
            // delivery, scheduler integration to stop tracee.
            let _target_pid = process::ProcessId(pid as u64);
            Err(SyscallError::InvalidSyscall)
        }

        PtraceRequest::Detach => {
            // Release the tracee, optionally delivering a signal.
            // Requires: tracer relationship cleanup, signal delivery,
            // resume tracee via scheduler.
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

//! System call interface for VeridianOS
//!
//! Provides the kernel-side implementation of system calls including IPC
//! operations.

#![allow(dead_code)]

use crate::ipc::{IpcError, SmallMessage};

mod process;
use process::*;

/// System call numbers
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syscall {
    // IPC system calls
    IpcSend = 0,
    IpcReceive = 1,
    IpcCall = 2,
    IpcReply = 3,
    IpcCreateEndpoint = 4,
    IpcBindEndpoint = 5,
    IpcShareMemory = 6,
    IpcMapMemory = 7,

    // Process management
    ProcessYield = 10,
    ProcessExit = 11,
    ProcessFork = 12,
    ProcessExec = 13,
    ProcessWait = 14,
    ProcessGetPid = 15,
    ProcessGetPPid = 16,
    ProcessSetPriority = 17,
    ProcessGetPriority = 18,

    // Thread management
    ThreadCreate = 40,
    ThreadExit = 41,
    ThreadJoin = 42,
    ThreadGetTid = 43,
    ThreadSetAffinity = 44,
    ThreadGetAffinity = 45,

    // Memory management
    MemoryMap = 20,
    MemoryUnmap = 21,

    // Capability management
    CapabilityGrant = 30,
    CapabilityRevoke = 31,
}

/// System call result type
pub type SyscallResult = Result<usize, SyscallError>;

/// System call error codes
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallError {
    InvalidSyscall = -1,
    InvalidArgument = -2,
    PermissionDenied = -3,
    ResourceNotFound = -4,
    OutOfMemory = -5,
    WouldBlock = -6,
    Interrupted = -7,
}

impl From<IpcError> for SyscallError {
    fn from(err: IpcError) -> Self {
        match err {
            IpcError::InvalidCapability => SyscallError::PermissionDenied,
            IpcError::ProcessNotFound => SyscallError::ResourceNotFound,
            IpcError::EndpointNotFound => SyscallError::ResourceNotFound,
            IpcError::OutOfMemory => SyscallError::OutOfMemory,
            IpcError::WouldBlock => SyscallError::WouldBlock,
            IpcError::PermissionDenied => SyscallError::PermissionDenied,
            _ => SyscallError::InvalidArgument,
        }
    }
}

/// System call handler entry point
#[no_mangle]
pub extern "C" fn syscall_handler(
    syscall_num: usize,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
) -> isize {
    let result = match Syscall::try_from(syscall_num) {
        Ok(syscall) => handle_syscall(syscall, arg1, arg2, arg3, arg4, arg5),
        Err(_) => Err(SyscallError::InvalidSyscall),
    };

    match result {
        Ok(value) => value as isize,
        Err(error) => error as i32 as isize,
    }
}

/// Handle individual system calls
fn handle_syscall(
    syscall: Syscall,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
) -> SyscallResult {
    match syscall {
        // IPC system calls
        Syscall::IpcSend => sys_ipc_send(arg1, arg2, arg3, arg4),
        Syscall::IpcReceive => sys_ipc_receive(arg1, arg2),
        Syscall::IpcCall => sys_ipc_call(arg1, arg2, arg3, arg4, arg5),
        Syscall::IpcReply => sys_ipc_reply(arg1, arg2, arg3),

        // Process management
        Syscall::ProcessYield => sys_yield(),
        Syscall::ProcessExit => sys_exit(arg1),
        Syscall::ProcessFork => sys_fork(),
        Syscall::ProcessExec => sys_exec(arg1, arg2, arg3),
        Syscall::ProcessWait => sys_wait(arg1 as isize, arg2, arg3),
        Syscall::ProcessGetPid => sys_getpid(),
        Syscall::ProcessGetPPid => sys_getppid(),
        Syscall::ProcessSetPriority => sys_setpriority(arg1, arg2, arg3),
        Syscall::ProcessGetPriority => sys_getpriority(arg1, arg2),

        // Thread management
        Syscall::ThreadCreate => sys_thread_create(arg1, arg2, arg3, arg4),
        Syscall::ThreadExit => sys_thread_exit(arg1),
        Syscall::ThreadJoin => sys_thread_join(arg1, arg2),
        Syscall::ThreadGetTid => sys_gettid(),
        Syscall::ThreadSetAffinity => sys_thread_setaffinity(arg1, arg2, arg3),
        Syscall::ThreadGetAffinity => sys_thread_getaffinity(arg1, arg2, arg3),

        _ => Err(SyscallError::InvalidSyscall),
    }
}

/// IPC send system call
///
/// # Arguments
/// - capability: Capability token for the endpoint
/// - msg_ptr: Pointer to message structure
/// - msg_size: Size of message
/// - flags: Send flags
fn sys_ipc_send(
    _capability: usize,
    msg_ptr: usize,
    msg_size: usize,
    _flags: usize,
) -> SyscallResult {
    // Validate arguments
    if msg_ptr == 0 || msg_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // Check if this is a small message (fast path)
    if msg_size <= core::mem::size_of::<SmallMessage>() {
        // Fast path for small messages
        unsafe {
            let _msg = *(msg_ptr as *const SmallMessage);
            // TODO: Perform actual IPC send
            // This would involve:
            // 1. Validate capability
            // 2. Find target process
            // 3. Copy message to target
            // 4. Context switch if synchronous
        }
    } else {
        // Large message path
        // TODO: Handle large messages with shared memory
    }

    Ok(0)
}

/// IPC receive system call
///
/// # Arguments
/// - endpoint: Endpoint to receive from
/// - buffer: Buffer to receive message into
fn sys_ipc_receive(_endpoint: usize, buffer: usize) -> SyscallResult {
    if buffer == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // TODO: Implement receive
    // 1. Find endpoint
    // 2. Check for waiting messages
    // 3. If none, block current process
    // 4. Copy message to buffer when available

    Ok(0)
}

/// IPC call (send and wait for reply)
fn sys_ipc_call(
    _capability: usize,
    _send_msg: usize,
    _send_size: usize,
    _recv_buf: usize,
    _recv_size: usize,
) -> SyscallResult {
    // TODO: Implement call semantics
    // 1. Send message
    // 2. Block waiting for reply
    // 3. Return reply in recv_buf

    Ok(0)
}

/// IPC reply to a previous call
fn sys_ipc_reply(_caller: usize, _msg_ptr: usize, _msg_size: usize) -> SyscallResult {
    // TODO: Implement reply
    // 1. Validate caller is waiting for reply
    // 2. Copy reply message
    // 3. Wake up caller

    Ok(0)
}

/// Yield CPU to another process
fn sys_yield() -> SyscallResult {
    // TODO: Trigger scheduler
    Ok(0)
}

impl TryFrom<usize> for Syscall {
    type Error = ();

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            // IPC system calls
            0 => Ok(Syscall::IpcSend),
            1 => Ok(Syscall::IpcReceive),
            2 => Ok(Syscall::IpcCall),
            3 => Ok(Syscall::IpcReply),
            4 => Ok(Syscall::IpcCreateEndpoint),
            5 => Ok(Syscall::IpcBindEndpoint),
            6 => Ok(Syscall::IpcShareMemory),
            7 => Ok(Syscall::IpcMapMemory),

            // Process management
            10 => Ok(Syscall::ProcessYield),
            11 => Ok(Syscall::ProcessExit),
            12 => Ok(Syscall::ProcessFork),
            13 => Ok(Syscall::ProcessExec),
            14 => Ok(Syscall::ProcessWait),
            15 => Ok(Syscall::ProcessGetPid),
            16 => Ok(Syscall::ProcessGetPPid),
            17 => Ok(Syscall::ProcessSetPriority),
            18 => Ok(Syscall::ProcessGetPriority),

            // Memory management
            20 => Ok(Syscall::MemoryMap),
            21 => Ok(Syscall::MemoryUnmap),

            // Capability management
            30 => Ok(Syscall::CapabilityGrant),
            31 => Ok(Syscall::CapabilityRevoke),

            // Thread management
            40 => Ok(Syscall::ThreadCreate),
            41 => Ok(Syscall::ThreadExit),
            42 => Ok(Syscall::ThreadJoin),
            43 => Ok(Syscall::ThreadGetTid),
            44 => Ok(Syscall::ThreadSetAffinity),
            45 => Ok(Syscall::ThreadGetAffinity),

            _ => Err(()),
        }
    }
}

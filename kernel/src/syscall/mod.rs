//! System call interface for VeridianOS
//! 
//! Provides the kernel-side implementation of system calls including IPC operations.

use crate::ipc::{Message, SmallMessage, IpcError};

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
        Syscall::IpcSend => sys_ipc_send(arg1, arg2, arg3, arg4),
        Syscall::IpcReceive => sys_ipc_receive(arg1, arg2),
        Syscall::IpcCall => sys_ipc_call(arg1, arg2, arg3, arg4, arg5),
        Syscall::IpcReply => sys_ipc_reply(arg1, arg2, arg3),
        Syscall::ProcessYield => sys_yield(),
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
fn sys_ipc_send(capability: usize, msg_ptr: usize, msg_size: usize, flags: usize) -> SyscallResult {
    // Validate arguments
    if msg_ptr == 0 || msg_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    
    // Check if this is a small message (fast path)
    if msg_size <= core::mem::size_of::<SmallMessage>() {
        // Fast path for small messages
        unsafe {
            let msg = *(msg_ptr as *const SmallMessage);
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
fn sys_ipc_receive(endpoint: usize, buffer: usize) -> SyscallResult {
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
    capability: usize,
    send_msg: usize,
    send_size: usize,
    recv_buf: usize,
    recv_size: usize,
) -> SyscallResult {
    // TODO: Implement call semantics
    // 1. Send message
    // 2. Block waiting for reply
    // 3. Return reply in recv_buf
    
    Ok(0)
}

/// IPC reply to a previous call
fn sys_ipc_reply(caller: usize, msg_ptr: usize, msg_size: usize) -> SyscallResult {
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
            0 => Ok(Syscall::IpcSend),
            1 => Ok(Syscall::IpcReceive),
            2 => Ok(Syscall::IpcCall),
            3 => Ok(Syscall::IpcReply),
            4 => Ok(Syscall::IpcCreateEndpoint),
            5 => Ok(Syscall::IpcBindEndpoint),
            6 => Ok(Syscall::IpcShareMemory),
            7 => Ok(Syscall::IpcMapMemory),
            10 => Ok(Syscall::ProcessYield),
            11 => Ok(Syscall::ProcessExit),
            20 => Ok(Syscall::MemoryMap),
            21 => Ok(Syscall::MemoryUnmap),
            30 => Ok(Syscall::CapabilityGrant),
            31 => Ok(Syscall::CapabilityRevoke),
            _ => Err(()),
        }
    }
}
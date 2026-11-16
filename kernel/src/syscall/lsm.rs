//! lsm_list_modules, lsm_get_self_attr and lsm_set_self_attr, as Linux 7.3
//! (`security/lsm_syscalls.c`, `security/security.c`). The security
//! modules a program can name are the capability module and Landlock
//! (ADR 0029); neither has process attributes, so getting or setting one
//! is EOPNOTSUPP once the arguments pass Linux's checks.

use super::{userspace, SyscallError, SyscallResult};

/// LSM_ID_CAPABILITY and LSM_ID_LANDLOCK, in the order Linux lists them
/// (capability first).
const MODULES: [u64; 2] = [100, 110];

/// LSM_FLAG_SINGLE.
const FLAG_SINGLE: usize = 1;

/// `struct lsm_ctx`'s fixed part: id, flags, len, ctx_len.
const LSM_CTX_SIZE: usize = 32;

/// lsm_list_modules(ids, size, flags): the modules' ids; `*size` is set
/// to the bytes they take, and a smaller buffer is E2BIG.
pub fn sys_lsm_list_modules(ids: usize, size: usize, flags: usize) -> SyscallResult {
    if flags as u32 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let usize_: u32 = userspace::read_user(size)?;
    let total = (MODULES.len() * 8) as u32;
    userspace::write_user(size, total)?;
    if usize_ < total {
        return Err(SyscallError::ArgumentListTooLong);
    }
    // The whole array, its range checked once (Linux puts each id in turn,
    // with the same EFAULT).
    userspace::write_user_slice(ids, &MODULES)?;
    Ok(MODULES.len())
}

/// lsm_get_self_attr(attr, ctx, size, flags): no module has an
/// attribute, so after the checks `*size` is 0 and the answer EOPNOTSUPP.
pub fn sys_lsm_get_self_attr(attr: usize, ctx: usize, size: usize, flags: usize) -> SyscallResult {
    if attr as u32 == 0 || size == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let _left: u32 = userspace::read_user(size)?;
    if flags as u32 as usize != 0 {
        if flags as u32 as usize != FLAG_SINGLE || ctx == 0 {
            return Err(SyscallError::InvalidArgument);
        }
        let id: u64 = userspace::read_user(ctx)?;
        // The rest of the context is copied too (EFAULT).
        let mut rest = [0u8; LSM_CTX_SIZE - 8];
        let after_id = ctx.checked_add(8).ok_or(SyscallError::InvalidPointer)?;
        userspace::read_user_bytes(after_id, &mut rest)?;
        if id == 0 {
            return Err(SyscallError::InvalidArgument);
        }
    }
    userspace::write_user(size, 0u32)?;
    Err(SyscallError::NotSupported)
}

/// lsm_set_self_attr(attr, ctx, size, flags): the context checked, then
/// EOPNOTSUPP, as no module sets attributes.
pub fn sys_lsm_set_self_attr(_attr: usize, ctx: usize, size: usize, flags: usize) -> SyscallResult {
    if flags as u32 != 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let size = size as u32 as usize;
    if size < LSM_CTX_SIZE {
        return Err(SyscallError::InvalidArgument);
    }
    if size > 4096 {
        return Err(SyscallError::ArgumentListTooLong);
    }
    let raw = userspace::read_user_vec(ctx, size, 4096)?;
    let word = |i: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&raw[i * 8..i * 8 + 8]);
        u64::from_ne_bytes(b)
    };
    let (len, ctx_len) = (word(2), word(3));
    let required = (LSM_CTX_SIZE as u64).checked_add(ctx_len);
    if (size as u64) < len || required.is_none_or(|r| len < r) {
        return Err(SyscallError::InvalidArgument);
    }
    Err(SyscallError::NotSupported)
}

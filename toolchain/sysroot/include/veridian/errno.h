/*
 * VeridianOS Error Codes
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Error codes matching kernel/src/syscall/mod.rs SyscallError enum.
 * Syscall return values: >= 0 is success, < 0 is -errno.
 */

#ifndef VERIDIAN_ERRNO_H
#define VERIDIAN_ERRNO_H

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Core Error Codes (matching SyscallError repr(i32))                        */
/* ========================================================================= */

/** Invalid system call number */
#define ENOSYS              1   /* SyscallError::InvalidSyscall = -1 */

/** Invalid argument */
#define EINVAL              2   /* SyscallError::InvalidArgument = -2 */

/** Permission denied */
#define EPERM               3   /* SyscallError::PermissionDenied = -3 */

/** Resource not found (file, process, endpoint) */
#define ENOENT              4   /* SyscallError::ResourceNotFound = -4 */

/** Out of memory */
#define ENOMEM              5   /* SyscallError::OutOfMemory = -5 */

/** Operation would block (non-blocking mode) */
#define EAGAIN              6   /* SyscallError::WouldBlock = -6 */

/** Operation interrupted by signal */
#define EINTR               7   /* SyscallError::Interrupted = -7 */

/** Invalid internal state */
#define EILSEQ              8   /* SyscallError::InvalidState = -8 */

/** Invalid pointer (null, misaligned, or out of bounds) */
#define EFAULT              9   /* SyscallError::InvalidPointer = -9 */

/* ========================================================================= */
/* Capability Error Codes                                                    */
/* ========================================================================= */

/** Invalid capability token */
#define ECAPINVAL           10  /* SyscallError::InvalidCapability = -10 */

/** Capability has been revoked */
#define ECAPREVOKED         11  /* SyscallError::CapabilityRevoked = -11 */

/** Insufficient capability rights */
#define ECAPRIGHTS          12  /* SyscallError::InsufficientRights = -12 */

/** Capability not found */
#define ECAPNOTFOUND        13  /* SyscallError::CapabilityNotFound = -13 */

/** Capability already exists */
#define ECAPEXISTS          14  /* SyscallError::CapabilityAlreadyExists = -14 */

/** Invalid capability object */
#define ECAPOBJECT          15  /* SyscallError::InvalidCapabilityObject = -15 */

/** Capability delegation denied */
#define ECAPDELEG           16  /* SyscallError::CapabilityDelegationDenied = -16 */

/* ========================================================================= */
/* Extended Error Codes                                                      */
/* ========================================================================= */

/** Unmapped memory region */
#define ENOMAPPING          17  /* SyscallError::UnmappedMemory = -17 */

/** Access denied (kernel memory, etc.) */
#define EACCES              18  /* SyscallError::AccessDenied = -18 */

/** Target process not found */
#define ESRCH               19  /* SyscallError::ProcessNotFound = -19 */

/* ========================================================================= */
/* POSIX-Compatible Aliases                                                  */
/* ========================================================================= */

/** Same as EAGAIN (POSIX compatibility) */
#define EWOULDBLOCK         EAGAIN

/* ========================================================================= */
/* errno access                                                              */
/* ========================================================================= */

/*
 * Thread-local errno.
 *
 * In VeridianOS, syscall wrappers return negative error codes directly.
 * The libc layer translates: if (ret < 0) { errno = -ret; return -1; }
 *
 * For bare-metal programs that bypass libc, inspect the raw return value
 * and use VERIDIAN_IS_ERR / VERIDIAN_ERR_CODE below.
 */

#ifndef __VERIDIAN_KERNEL__
extern int *__veridian_errno_location(void);
#define errno (*__veridian_errno_location())
#endif

/* ========================================================================= */
/* Raw Syscall Error Helpers                                                 */
/* ========================================================================= */

/** Check if a raw syscall return value indicates an error */
#define VERIDIAN_IS_ERR(ret)    ((long)(ret) < 0)

/** Extract the positive error code from a raw syscall return value */
#define VERIDIAN_ERR_CODE(ret)  ((int)(-(long)(ret)))

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_ERRNO_H */

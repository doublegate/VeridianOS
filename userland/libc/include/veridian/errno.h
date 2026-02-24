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
/* Additional POSIX Error Codes                                              */
/* ========================================================================= */

/*
 * These error codes are required by GCC, make, and other POSIX utilities.
 * The kernel may not return all of them, but they must be defined for
 * source compatibility.  Values 20+ are unambiguous additions.
 */

/** File exists */
#define EEXIST              20

/** Bad file descriptor */
#define EBADF               21

/** I/O error */
#define EIO                 22

/** No such device or address */
#define ENXIO               23

/** Argument list too long */
#define E2BIG               24

/** Exec format error */
#define ENOEXEC             25

/** No child processes */
#define ECHILD              26

/** Device or resource busy */
#define EBUSY               27

/** Not a directory */
#define ENOTDIR             28

/** Is a directory */
#define EISDIR              29

/** Too many open files */
#define EMFILE              30

/** File table overflow */
#define ENFILE              31

/** Not a typewriter (inappropriate ioctl) */
#define ENOTTY              32

/** Text file busy */
#define ETXTBSY             33

/** File too large */
#define EFBIG               34

/** No space left on device */
#define ENOSPC              35

/** Illegal seek */
#define ESPIPE              36

/** Read-only file system */
#define EROFS               37

/** Too many links */
#define EMLINK              38

/** Broken pipe */
#define EPIPE               39

/** Math argument out of domain */
#define EDOM                40

/** Math result not representable */
#define ERANGE              41

/** Resource deadlock would occur */
#define EDEADLK             42

/** File name too long */
#define ENAMETOOLONG        43

/** No record locks available */
#define ENOLCK              44

/** Directory not empty */
#define ENOTEMPTY           45

/** Too many symbolic links encountered */
#define ELOOP               46

/** No message of desired type */
#define ENOMSG              47

/** Cross-device link */
#define EXDEV               48

/** Connection refused */
#define ECONNREFUSED        49

/** Connection reset by peer */
#define ECONNRESET          50

/** No buffer space available */
#define ENOBUFS             51

/** Protocol not supported */
#define EPROTONOSUPPORT     52

/** Operation not supported */
#define ENOTSUP             53
#define EOPNOTSUPP          ENOTSUP

/** Address already in use */
#define EADDRINUSE          54

/** Address not available */
#define EADDRNOTAVAIL       55

/** Network is unreachable */
#define ENETUNREACH         56

/** Connection timed out */
#define ETIMEDOUT           57

/** Operation already in progress */
#define EALREADY            58

/** Operation now in progress */
#define EINPROGRESS         59

/** Socket operation on non-socket */
#define ENOTSOCK            60

/** Destination address required */
#define EDESTADDRREQ        61

/** Message too long */
#define EMSGSIZE            62

/** Protocol wrong type for socket */
#define EPROTOTYPE          63

/** Transport endpoint is not connected */
#define ENOTCONN            64

/** Transport endpoint is already connected */
#define EISCONN             65

/** Address family not supported */
#define EAFNOSUPPORT        66

/** Connection aborted */
#define ECONNABORTED        67

/** No route to host */
#define EHOSTUNREACH        68

/** Network is down */
#define ENETDOWN            69

/** Network dropped connection because of reset */
#define ENETRESET           70

/** Protocol not available */
#define ENOPROTOOPT         71

/** No such device */
#define ENODEV              72

/** Value too large for defined data type */
#define EOVERFLOW           73

/** Protocol error */
#define EPROTO              74

/** Operation canceled */
#define ECANCELED           75

/** Owner died */
#define EOWNERDEAD          76

/** State not recoverable */
#define ENOTRECOVERABLE     77

/** Link has been severed */
#define ENOLINK             78

/** Resource limit exceeded (process table full, fd table full) */
#define ERESOURCELIMIT      79  /* SyscallError::ResourceLimitExceeded = -79 */

/* ========================================================================= */
/* POSIX-Compatible Aliases                                                  */
/* ========================================================================= */

/** Same as EAGAIN (POSIX compatibility) */
#define EWOULDBLOCK         EAGAIN
#define EDEADLOCK           EDEADLK

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

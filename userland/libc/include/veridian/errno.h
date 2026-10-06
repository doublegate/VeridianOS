/*
 * VeridianOS Error Codes
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Syscall return values: >= 0 is success, < 0 is -errno.
 *
 * Values are the Linux x86_64 errno numbers: the kernel translates every
 * syscall error with to_linux_errno() (musl expects Linux numbers), so
 * this libc must use the same table. VeridianOS-only codes use 200+.
 */
#ifndef VERIDIAN_ERRNO_H
#define VERIDIAN_ERRNO_H

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Core Error Codes                                                          */
/* ========================================================================= */

/** Invalid system call number */
#define ENOSYS              38

/** Invalid argument */
#define EINVAL              22

/** Permission denied */
#define EPERM               1

/** Resource not found (file, process, endpoint) */
#define ENOENT              2

/** Out of memory */
#define ENOMEM              12

/** Operation would block (non-blocking mode) */
#define EAGAIN              11

/** Operation interrupted by signal */
#define EINTR               4

/** Invalid internal state */
#define EILSEQ              84

/** Invalid pointer (null, misaligned, or out of bounds) */
#define EFAULT              14

/* ========================================================================= */
/* Capability Error Codes                                                    */
/* ========================================================================= */

/** Invalid capability token */
#define ECAPINVAL           200

/** Capability has been revoked */
#define ECAPREVOKED         201

/** Insufficient capability rights */
#define ECAPRIGHTS          202

/** Capability not found */
#define ECAPNOTFOUND        203

/** Capability already exists */
#define ECAPEXISTS          204

/** Invalid capability object */
#define ECAPOBJECT          205

/** Capability delegation denied */
#define ECAPDELEG           206

/* ========================================================================= */
/* Extended Error Codes                                                      */
/* ========================================================================= */

/** Unmapped memory region */
#define ENOMAPPING          207

/** Access denied (kernel memory, etc.) */
#define EACCES              13

/** Target process not found */
#define ESRCH               3

/* ========================================================================= */
/* Additional POSIX Error Codes                                              */
/* ========================================================================= */

/*
 * These error codes are required by GCC, make, and other POSIX utilities.
 * The kernel may not return all of them, but they must be defined for
 * source compatibility.  Values 20+ are unambiguous additions.
 */

/** File exists */
#define EEXIST              17

/** Bad file descriptor */
#define EBADF               9

/** I/O error */
#define EIO                 5

/** No such device or address */
#define ENXIO               6

/** Argument list too long */
#define E2BIG               7

/** Exec format error */
#define ENOEXEC             8

/** No child processes */
#define ECHILD              10

/** Device or resource busy */
#define EBUSY               16

/** Not a directory */
#define ENOTDIR             20

/** Is a directory */
#define EISDIR              21

/** Too many open files */
#define EMFILE              24

/** File table overflow */
#define ENFILE              23

/** Not a typewriter (inappropriate ioctl) */
#define ENOTTY              25

/** Text file busy */
#define ETXTBSY             26

/** File too large */
#define EFBIG               27

/** No space left on device */
#define ENOSPC              28

/** Illegal seek */
#define ESPIPE              29

/** Read-only file system */
#define EROFS               30

/** Too many links */
#define EMLINK              31

/** Broken pipe */
#define EPIPE               32

/** Math argument out of domain */
#define EDOM                33

/** Math result not representable */
#define ERANGE              34

/** Resource deadlock would occur */
#define EDEADLK             35

/** File name too long */
#define ENAMETOOLONG        36

/** No record locks available */
#define ENOLCK              37

/** Directory not empty */
#define ENOTEMPTY           39

/** Too many symbolic links encountered */
#define ELOOP               40

/** No message of desired type */
#define ENOMSG              42

/** Cross-device link */
#define EXDEV               18

/** Connection refused */
#define ECONNREFUSED        111

/** Connection reset by peer */
#define ECONNRESET          104

/** No buffer space available */
#define ENOBUFS             105

/** Protocol not supported */
#define EPROTONOSUPPORT     93

/** Operation not supported */
#define ENOTSUP             95
#define EOPNOTSUPP          ENOTSUP

/** Address already in use */
#define EADDRINUSE          98

/** Address not available */
#define EADDRNOTAVAIL       99

/** Network is unreachable */
#define ENETUNREACH         101

/** Connection timed out */
#define ETIMEDOUT           110

/** Operation already in progress */
#define EALREADY            114

/** Operation now in progress */
#define EINPROGRESS         115

/** Socket operation on non-socket */
#define ENOTSOCK            88

/** Destination address required */
#define EDESTADDRREQ        89

/** Message too long */
#define EMSGSIZE            90

/** Protocol wrong type for socket */
#define EPROTOTYPE          91

/** Transport endpoint is not connected */
#define ENOTCONN            107

/** Transport endpoint is already connected */
#define EISCONN             106

/** Address family not supported */
#define EAFNOSUPPORT        97

/** Connection aborted */
#define ECONNABORTED        103

/** No route to host */
#define EHOSTUNREACH        113

/** Network is down */
#define ENETDOWN            100

/** Network dropped connection because of reset */
#define ENETRESET           102

/** Protocol not available */
#define ENOPROTOOPT         92

/** No such device */
#define ENODEV              19

/** Value too large for defined data type */
#define EOVERFLOW           75

/** Protocol error */
#define EPROTO              71

/** Operation canceled */
#define ECANCELED           125

/** Owner died */
#define EOWNERDEAD          130

/** State not recoverable */
#define ENOTRECOVERABLE     131

/** Link has been severed */
#define ENOLINK             67

/** Resource limit exceeded (process table full, fd table full) */
#define ERESOURCELIMIT      208
#define ENODATA             61  /* No such attribute / no data (xattr) */

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

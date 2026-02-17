/*
 * VeridianOS libc -- <errno.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Re-exports the canonical error codes from <veridian/errno.h> and provides
 * additional POSIX errno values that don't map to VeridianOS SyscallError.
 */

#ifndef _ERRNO_H
#define _ERRNO_H

#include <veridian/errno.h>

/* ========================================================================= */
/* Additional POSIX error codes                                              */
/* ========================================================================= */

/* These start at 100 to avoid collision with kernel SyscallError codes. */

/** File exists */
#define EEXIST          100

/** Bad file descriptor */
#define EBADF           101

/** Resource busy */
#define EBUSY           102

/** Not a directory */
#define ENOTDIR         103

/** Is a directory */
#define EISDIR          104

/** Too many open files */
#define EMFILE          105

/** No space left on device */
#define ENOSPC          106

/** Read-only file system */
#define EROFS           107

/** Broken pipe */
#define EPIPE           108

/** Math argument out of domain of function */
#define EDOM            109

/** Result too large */
#define ERANGE          110

/** No child processes */
#define ECHILD          111

/** Not a typewriter */
#define ENOTTY          112

/** Cross-device link */
#define EXDEV           113

/** Illegal seek */
#define ESPIPE          114

#endif /* _ERRNO_H */

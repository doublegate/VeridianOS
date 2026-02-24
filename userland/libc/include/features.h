/*
 * VeridianOS libc -- <features.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Feature test macros. Minimal implementation for BusyBox compatibility.
 */

#ifndef _FEATURES_H
#define _FEATURES_H

/* VeridianOS identifies as a POSIX-like system */
#define __VERIDIAN__            1
#define __VERIDIAN_LIBC__       1

/* glibc compatibility version macros (BusyBox checks these) */
#define __GLIBC__               2
#define __GLIBC_MINOR__         17

/* Feature macros */
#define _POSIX_SOURCE           1
#define _POSIX_C_SOURCE         200809L
#define _BSD_SOURCE             1
#define _GNU_SOURCE             1

/* VeridianOS doesn't have __GLIBC_PREREQ but BusyBox uses it */
#ifndef __GLIBC_PREREQ
# define __GLIBC_PREREQ(maj, min) \
    ((__GLIBC__ << 16) + __GLIBC_MINOR__ >= ((maj) << 16) + (min))
#endif

#endif /* _FEATURES_H */

/*
 * VeridianOS Primitive Type Definitions
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Standard type definitions for user-space programs targeting VeridianOS.
 * All types are explicitly sized for ABI stability across architectures.
 */

#ifndef VERIDIAN_TYPES_H
#define VERIDIAN_TYPES_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Process and User Identity Types                                           */
/* ========================================================================= */

/** Process identifier (matches kernel ProcessId = u64) */
typedef int64_t     pid_t;

/** User identifier */
typedef uint32_t    uid_t;

/** Group identifier */
typedef uint32_t    gid_t;

/* ========================================================================= */
/* Filesystem Types                                                          */
/* ========================================================================= */

/** File permission mode bits */
typedef uint32_t    mode_t;

/** File offset (signed for seek operations) */
typedef int64_t     off_t;

/** Device identifier */
typedef uint64_t    dev_t;

/** Inode number */
typedef uint64_t    ino_t;

/** Link count */
typedef uint64_t    nlink_t;

/** Block size for filesystem I/O */
typedef int64_t     blksize_t;

/** Block count (512-byte units) */
typedef int64_t     blkcnt_t;

/* ========================================================================= */
/* Size Types                                                                */
/* ========================================================================= */

/*
 * size_t is provided by <stddef.h>.
 * ssize_t is the signed counterpart for return values that may be -1.
 */

#ifndef __ssize_t_defined
#define __ssize_t_defined
#if defined(__LP64__) || defined(__x86_64__) || defined(__aarch64__) || \
    (defined(__riscv) && __riscv_xlen == 64)
typedef int64_t     ssize_t;
#else
typedef int32_t     ssize_t;
#endif
#endif /* __ssize_t_defined */

/* ========================================================================= */
/* Time Types                                                                */
/* ========================================================================= */

/** Time in seconds since epoch */
typedef int64_t     time_t;

/** Clock identifier for clock_gettime et al. */
typedef int32_t     clockid_t;

/** Time specification with nanosecond precision */
struct timespec {
    time_t  tv_sec;     /* Seconds */
    long    tv_nsec;    /* Nanoseconds [0, 999999999] */
};

/** Time value with microsecond precision (legacy) */
struct timeval {
    time_t  tv_sec;     /* Seconds */
    long    tv_usec;    /* Microseconds [0, 999999] */
};

/* ========================================================================= */
/* Clock Identifiers                                                         */
/* ========================================================================= */

#define CLOCK_REALTIME          0
#define CLOCK_MONOTONIC         1
#define CLOCK_PROCESS_CPUTIME   2
#define CLOCK_THREAD_CPUTIME    3

/* ========================================================================= */
/* Miscellaneous Types                                                       */
/* ========================================================================= */

/** Capability token (matches kernel 64-bit capability) */
typedef uint64_t    cap_t;

/** Thread identifier */
typedef uint64_t    tid_t;

/** IPC endpoint identifier */
typedef uint64_t    endpoint_t;

#ifdef __cplusplus
}
#endif

#endif /* VERIDIAN_TYPES_H */

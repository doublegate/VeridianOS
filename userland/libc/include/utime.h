/*
 * VeridianOS libc -- <utime.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * File access and modification time manipulation.
 */

#ifndef _UTIME_H
#define _UTIME_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

struct utimbuf {
    time_t actime;    /* access time */
    time_t modtime;   /* modification time */
};

/** Set file access and modification times. */
int utime(const char *filename, const struct utimbuf *times);

#ifdef __cplusplus
}
#endif

#endif /* _UTIME_H */

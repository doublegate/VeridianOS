/*
 * VeridianOS libc -- <sys/times.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Process times.
 */

#ifndef _SYS_TIMES_H
#define _SYS_TIMES_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef long clock_t;

struct tms {
    clock_t tms_utime;   /* user time */
    clock_t tms_stime;   /* system time */
    clock_t tms_cutime;  /* user time of children */
    clock_t tms_cstime;  /* system time of children */
};

clock_t times(struct tms *buf);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_TIMES_H */

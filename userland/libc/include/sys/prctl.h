/*
 * VeridianOS libc -- <sys/prctl.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Process control operations (Linux-compatible subset).
 */

#ifndef _SYS_PRCTL_H
#define _SYS_PRCTL_H

#ifdef __cplusplus
extern "C" {
#endif

/* prctl() option constants */
#define PR_SET_NAME         15  /* Set the name of the calling thread */
#define PR_GET_NAME         16  /* Get the name of the calling thread */
#define PR_SET_DUMPABLE     4   /* Set dumpable attribute */
#define PR_GET_DUMPABLE     3   /* Get dumpable attribute */
#define PR_SET_PDEATHSIG    1   /* Set parent death signal */
#define PR_GET_PDEATHSIG    2   /* Get parent death signal */
#define PR_SET_CHILD_SUBREAPER  36
#define PR_GET_CHILD_SUBREAPER  37

int prctl(int option, unsigned long arg2, unsigned long arg3,
          unsigned long arg4, unsigned long arg5);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_PRCTL_H */

/*
 * VeridianOS libc -- sys/utsname.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * POSIX utsname structure and uname() declaration.
 */

#ifndef _SYS_UTSNAME_H
#define _SYS_UTSNAME_H

#ifdef __cplusplus
extern "C" {
#endif

#define _UTSNAME_LENGTH 65

struct utsname {
    char sysname[_UTSNAME_LENGTH];    /* Operating system name */
    char nodename[_UTSNAME_LENGTH];   /* Network node hostname */
    char release[_UTSNAME_LENGTH];    /* OS release */
    char version[_UTSNAME_LENGTH];    /* OS version */
    char machine[_UTSNAME_LENGTH];    /* Hardware identifier */
};

/**
 * Get system identification.
 * Fills buf with information about the running system.
 * Returns 0 on success, -1 on error (with errno set).
 */
int uname(struct utsname *buf);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_UTSNAME_H */

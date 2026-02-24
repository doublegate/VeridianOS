/*
 * VeridianOS libc -- <utmp.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * User accounting (stub).
 */

#ifndef _UTMP_H
#define _UTMP_H

#include <sys/types.h>
#include <sys/time.h>

#ifdef __cplusplus
extern "C" {
#endif

#define UT_LINESIZE     32
#define UT_NAMESIZE     32
#define UT_HOSTSIZE     256

/* utmp entry types */
#define EMPTY           0
#define RUN_LVL         1
#define BOOT_TIME       2
#define NEW_TIME        3
#define OLD_TIME        4
#define INIT_PROCESS    5
#define LOGIN_PROCESS   6
#define USER_PROCESS    7
#define DEAD_PROCESS    8
#define ACCOUNTING      9

struct utmp {
    short   ut_type;
    pid_t   ut_pid;
    char    ut_line[UT_LINESIZE];
    char    ut_id[4];
    char    ut_user[UT_NAMESIZE];
    char    ut_host[UT_HOSTSIZE];
    struct  timeval ut_tv;
};

/* utmpx is the same as utmp on VeridianOS */
#define utmpx utmp

void setutent(void);
struct utmp *getutent(void);
void endutent(void);
struct utmp *getutid(const struct utmp *ut);
struct utmp *getutline(const struct utmp *ut);
struct utmp *pututline(const struct utmp *ut);
void utmpname(const char *file);

#ifdef __cplusplus
}
#endif

#endif /* _UTMP_H */

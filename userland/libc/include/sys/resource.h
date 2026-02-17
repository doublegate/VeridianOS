/*
 * VeridianOS libc -- <sys/resource.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Resource usage and limits.  Stub implementations — getrlimit returns
 * RLIM_INFINITY for all resources, setrlimit is a no-op.
 */

#ifndef _SYS_RESOURCE_H
#define _SYS_RESOURCE_H

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Types                                                                     */
/* ========================================================================= */

typedef unsigned long rlim_t;

#define RLIM_INFINITY   ((rlim_t)-1)

struct rlimit {
    rlim_t rlim_cur;    /* Soft limit */
    rlim_t rlim_max;    /* Hard limit */
};

/* ========================================================================= */
/* Resource identifiers                                                      */
/* ========================================================================= */

#define RLIMIT_CPU      0   /* CPU time per process (seconds) */
#define RLIMIT_FSIZE    1   /* Max file size */
#define RLIMIT_DATA     2   /* Max data segment size */
#define RLIMIT_STACK    3   /* Max stack size */
#define RLIMIT_CORE     4   /* Max core file size */
#define RLIMIT_RSS      5   /* Max resident set size */
#define RLIMIT_NPROC    6   /* Max number of processes */
#define RLIMIT_NOFILE   7   /* Max number of open files */
#define RLIMIT_MEMLOCK  8   /* Max locked-in-memory address space */
#define RLIMIT_AS       9   /* Max address space size */

/* ========================================================================= */
/* Resource usage                                                            */
/* ========================================================================= */

#define RUSAGE_SELF     0
#define RUSAGE_CHILDREN (-1)

struct rusage {
    struct timeval ru_utime;    /* User CPU time used */
    struct timeval ru_stime;    /* System CPU time used */
    long ru_maxrss;             /* Max resident set size (KB) */
    long ru_ixrss;
    long ru_idrss;
    long ru_isrss;
    long ru_minflt;
    long ru_majflt;
    long ru_nswap;
    long ru_inblock;
    long ru_oublock;
    long ru_msgsnd;
    long ru_msgrcv;
    long ru_nsignals;
    long ru_nvcsw;
    long ru_nivcsw;
};

/* ========================================================================= */
/* Functions                                                                 */
/* ========================================================================= */

/** Get resource limits. */
int getrlimit(int resource, struct rlimit *rlp);

/** Set resource limits. */
int setrlimit(int resource, const struct rlimit *rlp);

/** Get resource usage. */
int getrusage(int who, struct rusage *usage);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_RESOURCE_H */

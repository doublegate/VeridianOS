/*
 * VeridianOS libc -- <sys/resource.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Resource limits, usage and priorities, as the kernel keeps them (Linux
 * layout and numbering).
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
#define RLIM_SAVED_CUR  RLIM_INFINITY
#define RLIM_SAVED_MAX  RLIM_INFINITY

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
#define RLIMIT_LOCKS    10  /* Max file locks */
#define RLIMIT_SIGPENDING 11 /* Max queued signals */
#define RLIMIT_MSGQUEUE 12  /* Max bytes in POSIX message queues */
#define RLIMIT_NICE     13  /* Ceiling for raising the nice value (20 - nice) */
#define RLIMIT_RTPRIO   14  /* Max real-time priority */
#define RLIMIT_RTTIME   15  /* Real-time CPU time without blocking (us) */
#define RLIMIT_NLIMITS  16
#define RLIM_NLIMITS    RLIMIT_NLIMITS

/* ========================================================================= */
/* Resource usage                                                            */
/* ========================================================================= */

#define RUSAGE_SELF     0
#define RUSAGE_CHILDREN (-1)
#define RUSAGE_THREAD   1

/* ========================================================================= */
/* Priorities                                                                */
/* ========================================================================= */

#define PRIO_PROCESS    0
#define PRIO_PGRP       1
#define PRIO_USER       2

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

/** Get and/or set a resource limit of process `pid` (0: the caller). */
int prlimit(pid_t pid, int resource, const struct rlimit *new_limit,
            struct rlimit *old_limit);

/** Get resource usage. */
int getrusage(int who, struct rusage *usage);

/** The lowest nice value of the processes named. */
int getpriority(int which, id_t who);

/** Set the nice value of the processes named. */
int setpriority(int which, id_t who, int prio);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_RESOURCE_H */

/*
 * VeridianOS libc -- <sched.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Scheduling interfaces.
 */

#ifndef _SCHED_H
#define _SCHED_H

#include <sys/types.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Scheduling policies */
#define SCHED_OTHER     0
#define SCHED_FIFO      1
#define SCHED_RR        2
#define SCHED_BATCH     3
#define SCHED_IDLE      5

/* CPU set for sched_getaffinity / sched_setaffinity */
#define CPU_SETSIZE     1024

typedef struct {
    unsigned long __bits[CPU_SETSIZE / (8 * sizeof(unsigned long))];
} cpu_set_t;

#define CPU_ZERO(set)       __builtin_memset((set), 0, sizeof(cpu_set_t))
#define CPU_SET(cpu, set)   ((set)->__bits[(cpu) / (8 * sizeof(unsigned long))] |= \
                            (1UL << ((cpu) % (8 * sizeof(unsigned long)))))
#define CPU_CLR(cpu, set)   ((set)->__bits[(cpu) / (8 * sizeof(unsigned long))] &= \
                            ~(1UL << ((cpu) % (8 * sizeof(unsigned long)))))
#define CPU_ISSET(cpu, set) (((set)->__bits[(cpu) / (8 * sizeof(unsigned long))] >> \
                            ((cpu) % (8 * sizeof(unsigned long)))) & 1)
#define CPU_COUNT(set)      __sched_cpu_count(sizeof(cpu_set_t), (set))

static inline int __sched_cpu_count(size_t setsize, const cpu_set_t *set)
{
    int count = 0;
    size_t i;
    for (i = 0; i < setsize / sizeof(unsigned long); i++) {
        unsigned long v = set->__bits[i];
        while (v) { count++; v &= v - 1; }
    }
    return count;
}

struct sched_param {
    int sched_priority;
};

/* Scheduling functions */
int sched_yield(void);
int sched_getaffinity(pid_t pid, size_t cpusetsize, cpu_set_t *mask);
int sched_setaffinity(pid_t pid, size_t cpusetsize, const cpu_set_t *mask);
int sched_getscheduler(pid_t pid);
int sched_setscheduler(pid_t pid, int policy, const struct sched_param *param);
int sched_getparam(pid_t pid, struct sched_param *param);
int sched_setparam(pid_t pid, const struct sched_param *param);
int sched_get_priority_max(int policy);
int sched_get_priority_min(int policy);

#ifdef __cplusplus
}
#endif

#endif /* _SCHED_H */

/*
 * VeridianOS libc -- <sys/sysinfo.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#ifndef _SYS_SYSINFO_H
#define _SYS_SYSINFO_H

#ifdef __cplusplus
extern "C" {
#endif

struct sysinfo {
    long uptime;            /* Seconds since boot */
    unsigned long loads[3]; /* 1, 5, and 15 minute load averages */
    unsigned long totalram; /* Total usable main memory */
    unsigned long freeram;  /* Available memory */
    unsigned long sharedram;/* Amount of shared memory */
    unsigned long bufferram;/* Memory used by buffers */
    unsigned long totalswap;/* Total swap space */
    unsigned long freeswap; /* Swap space still available */
    unsigned short procs;   /* Number of current processes */
    unsigned long totalhigh;/* Total high memory size */
    unsigned long freehigh; /* Available high memory size */
    unsigned int mem_unit;  /* Memory unit size in bytes */
};

int sysinfo(struct sysinfo *info);
int get_nprocs(void);
int get_nprocs_conf(void);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_SYSINFO_H */

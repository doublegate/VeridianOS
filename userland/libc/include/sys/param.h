/*
 * VeridianOS libc -- <sys/param.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * System parameters and limits.
 */

#ifndef _SYS_PARAM_H
#define _SYS_PARAM_H

#include <limits.h>

/* Page size */
#ifndef PAGE_SIZE
#define PAGE_SIZE       4096
#endif

#ifndef PAGESIZE
#define PAGESIZE        PAGE_SIZE
#endif

/* Maximum hostname length */
#ifndef MAXHOSTNAMELEN
#define MAXHOSTNAMELEN  64
#endif

/* Maximum path components */
#ifndef MAXPATHLEN
#define MAXPATHLEN      4096
#endif

#ifndef MAXNAMLEN
#define MAXNAMLEN       255
#endif

/* Bit manipulation macros */
#ifndef howmany
#define howmany(x, y)   (((x) + ((y) - 1)) / (y))
#endif

#ifndef roundup
#define roundup(x, y)   ((((x) + ((y) - 1)) / (y)) * (y))
#endif

#ifndef rounddown
#define rounddown(x, y) (((x) / (y)) * (y))
#endif

#ifndef powerof2
#define powerof2(x)     ((((x) - 1) & (x)) == 0)
#endif

/* MIN/MAX macros */
#ifndef MIN
#define MIN(a, b)       (((a) < (b)) ? (a) : (b))
#endif

#ifndef MAX
#define MAX(a, b)       (((a) > (b)) ? (a) : (b))
#endif

/* Number of bits per byte */
#ifndef NBBY
#define NBBY            8
#endif

/* setbit/clrbit/isset macros (bit arrays) */
#ifndef setbit
#define setbit(a, i)    (((unsigned char *)(a))[(i)/NBBY] |= 1 << ((i) % NBBY))
#endif

#ifndef clrbit
#define clrbit(a, i)    (((unsigned char *)(a))[(i)/NBBY] &= ~(1 << ((i) % NBBY)))
#endif

#ifndef isset
#define isset(a, i)     (((const unsigned char *)(a))[(i)/NBBY] & (1 << ((i) % NBBY)))
#endif

#ifndef isclr
#define isclr(a, i)     ((((const unsigned char *)(a))[(i)/NBBY] & (1 << ((i) % NBBY))) == 0)
#endif

#endif /* _SYS_PARAM_H */

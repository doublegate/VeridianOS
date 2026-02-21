/*
 * VeridianOS C Library -- <glob.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#ifndef _GLOB_H
#define _GLOB_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

#define GLOB_ERR       0x0001
#define GLOB_MARK      0x0002
#define GLOB_NOSORT    0x0004
#define GLOB_DOOFFS    0x0008
#define GLOB_NOCHECK   0x0010
#define GLOB_APPEND    0x0020
#define GLOB_NOESCAPE  0x0040

#define GLOB_NOSPACE   1
#define GLOB_ABORTED   2
#define GLOB_NOMATCH   3

typedef struct {
    size_t  gl_pathc;
    char  **gl_pathv;
    size_t  gl_offs;
} glob_t;

int  glob(const char *pattern, int flags,
          int (*errfunc)(const char *, int), glob_t *pglob);
void globfree(glob_t *pglob);

#ifdef __cplusplus
}
#endif

#endif /* _GLOB_H */

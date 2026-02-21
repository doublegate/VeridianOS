/*
 * VeridianOS C Library -- <strings.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#ifndef _STRINGS_H
#define _STRINGS_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

int    bcmp(const void *s1, const void *s2, size_t n);
void   bcopy(const void *src, void *dest, size_t n);
void   bzero(void *s, size_t n);
int    ffs(int i);
int    strcasecmp(const char *s1, const char *s2);
int    strncasecmp(const char *s1, const char *s2, size_t n);

#ifdef __cplusplus
}
#endif

#endif /* _STRINGS_H */

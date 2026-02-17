/*
 * VeridianOS libc -- <stddef.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Standard definitions using GCC/Clang predefined macros.
 */

#ifndef _STDDEF_H
#define _STDDEF_H

typedef __SIZE_TYPE__    size_t;
typedef __PTRDIFF_TYPE__ ptrdiff_t;

#ifndef __cplusplus
typedef __WCHAR_TYPE__   wchar_t;
#endif

#ifndef NULL
#ifdef __cplusplus
#define NULL nullptr
#else
#define NULL ((void *)0)
#endif
#endif

#define offsetof(type, member) __builtin_offsetof(type, member)

/* max_align_t: type with the maximum alignment */
typedef struct {
    long long   __ll __attribute__((aligned(__alignof__(long long))));
    long double __ld __attribute__((aligned(__alignof__(long double))));
} max_align_t;

#endif /* _STDDEF_H */

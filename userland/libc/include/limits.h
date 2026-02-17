/*
 * VeridianOS libc -- <limits.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Implementation limits for the VeridianOS C library.
 */

#ifndef _LIMITS_H
#define _LIMITS_H

/* Sizes of integral types (C11 5.2.4.2.1) */

#define CHAR_BIT    __CHAR_BIT__

#ifdef __CHAR_UNSIGNED__
#define CHAR_MIN    0
#define CHAR_MAX    __SCHAR_MAX__ * 2 + 1
#else
#define CHAR_MIN    (-__SCHAR_MAX__ - 1)
#define CHAR_MAX    __SCHAR_MAX__
#endif

#define SCHAR_MIN   (-__SCHAR_MAX__ - 1)
#define SCHAR_MAX   __SCHAR_MAX__
#define UCHAR_MAX   (__SCHAR_MAX__ * 2 + 1)

#define SHRT_MIN    (-__SHRT_MAX__ - 1)
#define SHRT_MAX    __SHRT_MAX__
#define USHRT_MAX   (__SHRT_MAX__ * 2 + 1)

#define INT_MIN     (-__INT_MAX__ - 1)
#define INT_MAX     __INT_MAX__
#define UINT_MAX    (__INT_MAX__ * 2U + 1U)

#define LONG_MIN    (-__LONG_MAX__ - 1L)
#define LONG_MAX    __LONG_MAX__
#define ULONG_MAX   (__LONG_MAX__ * 2UL + 1UL)

#define LLONG_MIN   (-__LONG_LONG_MAX__ - 1LL)
#define LLONG_MAX   __LONG_LONG_MAX__
#define ULLONG_MAX  (__LONG_LONG_MAX__ * 2ULL + 1ULL)

/* POSIX limits */
#define PATH_MAX    4096
#define NAME_MAX    255
#define PIPE_BUF    4096
#define OPEN_MAX    256
#define ARG_MAX     131072
#define LINE_MAX    2048

/* POSIX ssize_t limit */
#define SSIZE_MAX   __LONG_MAX__

#endif /* _LIMITS_H */

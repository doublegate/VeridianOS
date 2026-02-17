/*
 * VeridianOS libc -- <stdint.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Fixed-width integer types using GCC/Clang predefined macros.
 * This header is the "system" stdint.h that GCC's wrapper chains
 * to via #include_next when building libgcc and other target libs.
 */

#ifndef _STDINT_H
#define _STDINT_H

/* ========================================================================= */
/* Exact-width integer types                                                 */
/* ========================================================================= */

typedef __INT8_TYPE__   int8_t;
typedef __INT16_TYPE__  int16_t;
typedef __INT32_TYPE__  int32_t;
typedef __INT64_TYPE__  int64_t;

typedef __UINT8_TYPE__  uint8_t;
typedef __UINT16_TYPE__ uint16_t;
typedef __UINT32_TYPE__ uint32_t;
typedef __UINT64_TYPE__ uint64_t;

/* ========================================================================= */
/* Minimum-width integer types                                               */
/* ========================================================================= */

typedef __INT_LEAST8_TYPE__   int_least8_t;
typedef __INT_LEAST16_TYPE__  int_least16_t;
typedef __INT_LEAST32_TYPE__  int_least32_t;
typedef __INT_LEAST64_TYPE__  int_least64_t;

typedef __UINT_LEAST8_TYPE__  uint_least8_t;
typedef __UINT_LEAST16_TYPE__ uint_least16_t;
typedef __UINT_LEAST32_TYPE__ uint_least32_t;
typedef __UINT_LEAST64_TYPE__ uint_least64_t;

/* ========================================================================= */
/* Fastest minimum-width integer types                                       */
/* ========================================================================= */

typedef __INT_FAST8_TYPE__   int_fast8_t;
typedef __INT_FAST16_TYPE__  int_fast16_t;
typedef __INT_FAST32_TYPE__  int_fast32_t;
typedef __INT_FAST64_TYPE__  int_fast64_t;

typedef __UINT_FAST8_TYPE__  uint_fast8_t;
typedef __UINT_FAST16_TYPE__ uint_fast16_t;
typedef __UINT_FAST32_TYPE__ uint_fast32_t;
typedef __UINT_FAST64_TYPE__ uint_fast64_t;

/* ========================================================================= */
/* Pointer-width integer types                                               */
/* ========================================================================= */

typedef __INTPTR_TYPE__  intptr_t;
typedef __UINTPTR_TYPE__ uintptr_t;

/* ========================================================================= */
/* Greatest-width integer types                                              */
/* ========================================================================= */

typedef __INTMAX_TYPE__  intmax_t;
typedef __UINTMAX_TYPE__ uintmax_t;

/* ========================================================================= */
/* Limits of exact-width integer types                                       */
/* ========================================================================= */

#define INT8_MIN    (-__INT8_MAX__ - 1)
#define INT8_MAX    __INT8_MAX__
#define UINT8_MAX   __UINT8_MAX__

#define INT16_MIN   (-__INT16_MAX__ - 1)
#define INT16_MAX   __INT16_MAX__
#define UINT16_MAX  __UINT16_MAX__

#define INT32_MIN   (-__INT32_MAX__ - 1)
#define INT32_MAX   __INT32_MAX__
#define UINT32_MAX  __UINT32_MAX__

#define INT64_MIN   (-__INT64_MAX__ - 1)
#define INT64_MAX   __INT64_MAX__
#define UINT64_MAX  __UINT64_MAX__

/* ========================================================================= */
/* Limits of pointer-width integer types                                     */
/* ========================================================================= */

#define INTPTR_MIN  (-__INTPTR_MAX__ - 1)
#define INTPTR_MAX  __INTPTR_MAX__
#define UINTPTR_MAX __UINTPTR_MAX__

/* ========================================================================= */
/* Limits of greatest-width integer types                                    */
/* ========================================================================= */

#define INTMAX_MIN  (-__INTMAX_MAX__ - 1)
#define INTMAX_MAX  __INTMAX_MAX__
#define UINTMAX_MAX __UINTMAX_MAX__

/* ========================================================================= */
/* Other limits                                                              */
/* ========================================================================= */

#define PTRDIFF_MIN (-__PTRDIFF_MAX__ - 1)
#define PTRDIFF_MAX __PTRDIFF_MAX__
#define SIZE_MAX    __SIZE_MAX__
#define SIG_ATOMIC_MIN (-__SIG_ATOMIC_MAX__ - 1)
#define SIG_ATOMIC_MAX __SIG_ATOMIC_MAX__

/* ========================================================================= */
/* Macros for integer constant expressions                                   */
/* ========================================================================= */

#define INT8_C(c)   __INT8_C(c)
#define INT16_C(c)  __INT16_C(c)
#define INT32_C(c)  __INT32_C(c)
#define INT64_C(c)  __INT64_C(c)

#define UINT8_C(c)  __UINT8_C(c)
#define UINT16_C(c) __UINT16_C(c)
#define UINT32_C(c) __UINT32_C(c)
#define UINT64_C(c) __UINT64_C(c)

#define INTMAX_C(c)  __INTMAX_C(c)
#define UINTMAX_C(c) __UINTMAX_C(c)

#endif /* _STDINT_H */

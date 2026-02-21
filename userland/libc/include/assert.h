/*
 * VeridianOS C Library -- <assert.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#ifndef _ASSERT_H
#define _ASSERT_H

#ifdef __cplusplus
extern "C" {
#endif

void __assert_fail(const char *expr, const char *file,
                   unsigned int line, const char *func);

#ifdef NDEBUG
#define assert(expression) ((void)0)
#else
#define assert(expression) \
    ((expression) ? (void)0 : \
     __assert_fail(#expression, __FILE__, __LINE__, __func__))
#endif

/* C11 static_assert */
#ifndef __cplusplus
#if __STDC_VERSION__ >= 201112L
#define static_assert _Static_assert
#endif
#endif

#ifdef __cplusplus
}
#endif

#endif /* _ASSERT_H */

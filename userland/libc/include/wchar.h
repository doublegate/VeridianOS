/*
 * VeridianOS libc -- <wchar.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal wide character support: wint_t, mbstate_t, WEOF.
 * Full wide string functions are not provided.
 */

#ifndef _WCHAR_H
#define _WCHAR_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* wint_t: integer type that can hold any wchar_t value plus WEOF */
#ifndef _WINT_T_DEFINED
#define _WINT_T_DEFINED
typedef unsigned int wint_t;
#endif

/* WEOF: wide end-of-file indicator */
#ifndef WEOF
#define WEOF ((wint_t)-1)
#endif

/* mbstate_t: multi-byte conversion state */
typedef struct {
    int __fill[6];
} mbstate_t;

/* Minimal mbsinit: check if state is initial */
static inline int mbsinit(const mbstate_t *ps) {
    if (ps == 0) return 1;
    /* Check if all state bytes are zero */
    const unsigned char *p = (const unsigned char *)ps;
    for (unsigned i = 0; i < sizeof(mbstate_t); i++)
        if (p[i]) return 0;
    return 1;
}

/* Multibyte-to-wide and wide-to-multibyte conversion functions */
size_t mbrtowc(wchar_t *pwc, const char *s, size_t n, mbstate_t *ps);
size_t wcrtomb(char *s, wchar_t wc, mbstate_t *ps);
size_t mbrlen(const char *s, size_t n, mbstate_t *ps);

/* Wide string I/O */
int swprintf(wchar_t *s, size_t n, const wchar_t *format, ...);
int vswprintf(wchar_t *s, size_t n, const wchar_t *format, __builtin_va_list ap);

/* Wide string conversion */
long wcstol(const wchar_t *nptr, wchar_t **endptr, int base);
unsigned long wcstoul(const wchar_t *nptr, wchar_t **endptr, int base);

/* Minimal wide memory functions (stubs) */
static inline wchar_t *wmemcpy(wchar_t *dest, const wchar_t *src, size_t n) {
    for (size_t i = 0; i < n; i++) dest[i] = src[i];
    return dest;
}

static inline wchar_t *wmemmove(wchar_t *dest, const wchar_t *src, size_t n) {
    if (dest < src) {
        for (size_t i = 0; i < n; i++) dest[i] = src[i];
    } else if (dest > src) {
        for (size_t i = n; i > 0; i--) dest[i-1] = src[i-1];
    }
    return dest;
}

static inline wchar_t *wmemset(wchar_t *dest, wchar_t c, size_t n) {
    for (size_t i = 0; i < n; i++) dest[i] = c;
    return dest;
}

static inline int wmemcmp(const wchar_t *s1, const wchar_t *s2, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (s1[i] < s2[i]) return -1;
        if (s1[i] > s2[i]) return 1;
    }
    return 0;
}

static inline const wchar_t *wmemchr(const wchar_t *s, wchar_t c, size_t n) {
    for (size_t i = 0; i < n; i++)
        if (s[i] == c) return &s[i];
    return 0;
}

static inline size_t wcslen(const wchar_t *s) {
    size_t len = 0;
    while (s[len]) len++;
    return len;
}

#ifdef __cplusplus
}
#endif

#endif /* _WCHAR_H */

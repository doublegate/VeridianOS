/*
 * VeridianOS libc -- <wctype.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Wide character classification and mapping (implemented in
 * src/posix_stubs2.c).
 */

#ifndef _WCTYPE_H
#define _WCTYPE_H

#include <wchar.h>

typedef unsigned long wctype_t;
typedef const int *wctrans_t;

int iswalpha(wint_t wc);
int iswdigit(wint_t wc);
int iswspace(wint_t wc);
int iswupper(wint_t wc);
int iswlower(wint_t wc);
int iswalnum(wint_t wc);
int iswprint(wint_t wc);
int iswcntrl(wint_t wc);
int iswpunct(wint_t wc);
int iswxdigit(wint_t wc);
int iswgraph(wint_t wc);
int iswblank(wint_t wc);
wint_t towupper(wint_t wc);
wint_t towlower(wint_t wc);
wctype_t wctype(const char *name);
int iswctype(wint_t wc, wctype_t desc);
wctrans_t wctrans(const char *name);
wint_t towctrans(wint_t wc, wctrans_t desc);

#endif /* _WCTYPE_H */

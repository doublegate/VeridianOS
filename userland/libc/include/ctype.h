/*
 * VeridianOS libc -- <ctype.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Character classification and conversion functions.
 * Only the basic C/POSIX locale is supported (ASCII).
 */

#ifndef _CTYPE_H
#define _CTYPE_H

#ifdef __cplusplus
extern "C" {
#endif

int isalpha(int c);
int isdigit(int c);
int isalnum(int c);
int isspace(int c);
int isupper(int c);
int islower(int c);
int isprint(int c);
int isgraph(int c);
int iscntrl(int c);
int ispunct(int c);
int isxdigit(int c);
int isascii(int c);

int toupper(int c);
int tolower(int c);
int toascii(int c);

#ifdef __cplusplus
}
#endif

#endif /* _CTYPE_H */

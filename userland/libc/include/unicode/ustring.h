/*
 * VeridianOS libc -- unicode/ustring.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * ICU 75.x UTF-16 string operations.
 */

#ifndef _UNICODE_USTRING_H
#define _UNICODE_USTRING_H

#include "utypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/** Get length of a UChar string. */
int32_t u_strlen(const UChar *s);

/** Copy UChar strings. */
UChar *u_strcpy(UChar *dst, const UChar *src);

/** Copy UChar strings with length limit. */
UChar *u_strncpy(UChar *dst, const UChar *src, int32_t n);

/** Concatenate UChar strings. */
UChar *u_strcat(UChar *dst, const UChar *src);

/** Compare UChar strings. */
int32_t u_strcmp(const UChar *s1, const UChar *s2);

/** Compare UChar strings with length limit. */
int32_t u_strncmp(const UChar *s1, const UChar *s2, int32_t n);

/** Case-insensitive UChar string comparison. */
int32_t u_strcasecmp(const UChar *s1, const UChar *s2, uint32_t options);

/** Find a character in a UChar string. */
UChar *u_strchr(const UChar *s, UChar c);

/** Find a substring in a UChar string. */
UChar *u_strstr(const UChar *s, const UChar *substring);

/** Convert UTF-8 to UChar (UTF-16). */
UChar *u_strFromUTF8(UChar *dest, int32_t destCapacity,
                     int32_t *pDestLength,
                     const char *src, int32_t srcLength,
                     UErrorCode *pErrorCode);

/** Convert UChar (UTF-16) to UTF-8. */
char *u_strToUTF8(char *dest, int32_t destCapacity,
                  int32_t *pDestLength,
                  const UChar *src, int32_t srcLength,
                  UErrorCode *pErrorCode);

#ifdef __cplusplus
}
#endif

#endif /* _UNICODE_USTRING_H */

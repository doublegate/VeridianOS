/*
 * VeridianOS libc -- <string.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * String and memory manipulation functions.
 */

#ifndef _STRING_H
#define _STRING_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Memory operations                                                         */
/* ========================================================================= */

/** Copy n bytes from src to dest (must not overlap). */
void *memcpy(void *dest, const void *src, size_t n);

/** Copy n bytes from src to dest (may overlap). */
void *memmove(void *dest, const void *src, size_t n);

/** Fill n bytes of dest with byte c. */
void *memset(void *dest, int c, size_t n);

/** Compare n bytes of s1 and s2. */
int memcmp(const void *s1, const void *s2, size_t n);

/** Find first occurrence of byte c in n bytes of s. */
void *memchr(const void *s, int c, size_t n);

/* ========================================================================= */
/* String operations                                                         */
/* ========================================================================= */

/** Return the length of s (not counting the terminating NUL). */
size_t strlen(const char *s);

/** Return the length of s, but at most maxlen. */
size_t strnlen(const char *s, size_t maxlen);

/** Compare two strings. */
int strcmp(const char *s1, const char *s2);

/** Compare at most n characters of two strings. */
int strncmp(const char *s1, const char *s2, size_t n);

/** Copy src to dest (including terminating NUL). */
char *strcpy(char *dest, const char *src);

/** Copy at most n characters from src to dest (NUL-padded). */
char *strncpy(char *dest, const char *src, size_t n);

/** Append src to dest. */
char *strcat(char *dest, const char *src);

/** Append at most n characters from src to dest. */
char *strncat(char *dest, const char *src, size_t n);

/** Find first occurrence of c in s. */
char *strchr(const char *s, int c);

/** Find last occurrence of c in s. */
char *strrchr(const char *s, int c);

/** Find first occurrence of needle in haystack. */
char *strstr(const char *haystack, const char *needle);

/** Return the length of the initial segment of s consisting of bytes in accept. */
size_t strspn(const char *s, const char *accept);

/** Return the length of the initial segment of s consisting of bytes NOT in reject. */
size_t strcspn(const char *s, const char *reject);

/** Duplicate a string (malloc-allocated). */
char *strdup(const char *s);

/** Return a string describing the given errno value. */
char *strerror(int errnum);

/* ========================================================================= */
/* Tokenization                                                              */
/* ========================================================================= */

/** Split a string into tokens (not thread-safe). */
char *strtok(char *str, const char *delim);

#ifdef __cplusplus
}
#endif

#endif /* _STRING_H */

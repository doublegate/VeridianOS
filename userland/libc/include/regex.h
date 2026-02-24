/*
 * VeridianOS libc -- regex.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * POSIX regular expression support (BRE and ERE).
 * Provides types, constants, and function declarations.
 * Implementation in libc/src/regex.c (recursive backtracking NFA).
 */

#ifndef _REGEX_H
#define _REGEX_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Error codes */
#define REG_NOMATCH     1
#define REG_BADPAT      2
#define REG_ECOLLATE    3
#define REG_ECTYPE      4
#define REG_EESCAPE     5
#define REG_ESUBREG     6
#define REG_EBRACK      7
#define REG_EPAREN      8
#define REG_EBRACE      9
#define REG_BADBR       10
#define REG_ERANGE      11
#define REG_ESPACE      12
#define REG_BADRPT      13
#define REG_NOSYS       17

/* Compile flags */
#define REG_EXTENDED    1
#define REG_ICASE       2
#define REG_NOSUB       4
#define REG_NEWLINE     8

/* Execute flags */
#define REG_NOTBOL      1
#define REG_NOTEOL      2

typedef struct {
    size_t re_nsub;       /* Number of parenthesized subexpressions */
    void  *__internal;    /* Opaque internal state */
} regex_t;

typedef struct {
    int rm_so;  /* Byte offset from start of string to start of substring */
    int rm_eo;  /* Byte offset from start of string to end of substring */
} regmatch_t;

/** Compile a regular expression. */
int regcomp(regex_t *preg, const char *pattern, int cflags);

/** Execute a compiled regular expression. */
int regexec(const regex_t *preg, const char *string,
            size_t nmatch, regmatch_t pmatch[], int eflags);

/** Free compiled regular expression. */
void regfree(regex_t *preg);

/** Get error message for regex error code. */
size_t regerror(int errcode, const regex_t *preg,
                char *errbuf, size_t errbuf_size);

#ifdef __cplusplus
}
#endif

#endif /* _REGEX_H */

/*
 * VeridianOS libc -- <locale.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Locale definitions.  VeridianOS only supports the "C" / "POSIX" locale.
 */

#ifndef _LOCALE_H
#define _LOCALE_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Locale category constants                                                 */
/* ========================================================================= */

#define LC_CTYPE        0
#define LC_NUMERIC      1
#define LC_TIME         2
#define LC_COLLATE      3
#define LC_MONETARY     4
#define LC_MESSAGES     5
#define LC_ALL          6

/* ========================================================================= */
/* struct lconv                                                              */
/* ========================================================================= */

/**
 * Numeric and monetary formatting parameters.
 *
 * In the "C" locale most strings are empty and most char values are
 * CHAR_MAX, indicating that the information is not available.
 */
struct lconv {
    /* Numeric (non-monetary) */
    char *decimal_point;    /* "."  */
    char *thousands_sep;    /* ""   */
    char *grouping;         /* ""   */

    /* Monetary */
    char *int_curr_symbol;  /* ""   */
    char *currency_symbol;  /* ""   */
    char *mon_decimal_point;/* ""   */
    char *mon_thousands_sep;/* ""   */
    char *mon_grouping;     /* ""   */
    char *positive_sign;    /* ""   */
    char *negative_sign;    /* ""   */

    char  int_frac_digits;  /* CHAR_MAX */
    char  frac_digits;      /* CHAR_MAX */
    char  p_cs_precedes;    /* CHAR_MAX */
    char  p_sep_by_space;   /* CHAR_MAX */
    char  n_cs_precedes;    /* CHAR_MAX */
    char  n_sep_by_space;   /* CHAR_MAX */
    char  p_sign_posn;      /* CHAR_MAX */
    char  n_sign_posn;      /* CHAR_MAX */
};

/* ========================================================================= */
/* Function declarations                                                     */
/* ========================================================================= */

/**
 * Set the program's locale.
 *
 * VeridianOS stub: always returns "C" regardless of arguments.
 *
 * @param category  One of the LC_* constants.
 * @param locale    Desired locale string, or NULL to query.
 * @return "C" on success, NULL on failure.
 */
char *setlocale(int category, const char *locale);

/**
 * Get numeric formatting parameters for the current locale.
 *
 * @return Pointer to a static struct lconv with C locale defaults.
 */
struct lconv *localeconv(void);

#ifdef __cplusplus
}
#endif

#endif /* _LOCALE_H */

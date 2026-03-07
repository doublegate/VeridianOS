/*
 * VeridianOS C Library -- <langinfo.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Language information constants and nl_langinfo() declaration.
 * Required by Qt 6 for locale-dependent formatting.
 */

#ifndef _LANGINFO_H
#define _LANGINFO_H

#include <locale.h>

#ifdef __cplusplus
extern "C" {
#endif

/* nl_langinfo item constants */
typedef int nl_item;

/* Codeset */
#define CODESET         0

/* Date/time formats */
#define D_T_FMT         1   /* Date and time format (same as %c) */
#define D_FMT           2   /* Date format (same as %x) */
#define T_FMT           3   /* Time format (same as %X) */
#define T_FMT_AMPM      4   /* 12-hour time format */
#define AM_STR          5   /* AM string */
#define PM_STR          6   /* PM string */

/* Abbreviated day names */
#define DAY_1           7   /* Sunday */
#define DAY_2           8
#define DAY_3           9
#define DAY_4           10
#define DAY_5           11
#define DAY_6           12
#define DAY_7           13

/* Full day names */
#define ABDAY_1         14  /* Sun */
#define ABDAY_2         15
#define ABDAY_3         16
#define ABDAY_4         17
#define ABDAY_5         18
#define ABDAY_6         19
#define ABDAY_7         20

/* Full month names */
#define MON_1           21  /* January */
#define MON_2           22
#define MON_3           23
#define MON_4           24
#define MON_5           25
#define MON_6           26
#define MON_7           27
#define MON_8           28
#define MON_9           29
#define MON_10          30
#define MON_11          31
#define MON_12          32

/* Abbreviated month names */
#define ABMON_1         33  /* Jan */
#define ABMON_2         34
#define ABMON_3         35
#define ABMON_4         36
#define ABMON_5         37
#define ABMON_6         38
#define ABMON_7         39
#define ABMON_8         40
#define ABMON_9         41
#define ABMON_10        42
#define ABMON_11        43
#define ABMON_12        44

/* Numeric formatting */
#define RADIXCHAR       45  /* Decimal point character */
#define THOUSEP          46  /* Thousands separator */

/* Yes/No strings */
#define YESEXPR         47  /* Affirmative response regex */
#define NOEXPR          48  /* Negative response regex */

/* Currency */
#define CRNCYSTR        49  /* Currency symbol */

/* Era-based time (empty in C locale) */
#define ERA             50
#define ERA_D_FMT       51
#define ERA_D_T_FMT     52
#define ERA_T_FMT       53
#define ALT_DIGITS      54

/**
 * Return locale-specific information string for the given item.
 *
 * In the C/POSIX locale, returns standard POSIX defaults.
 *
 * @param item  One of the nl_item constants defined above.
 * @return Static string (must not be freed), or "" if item is unknown.
 */
char *nl_langinfo(nl_item item);

#ifdef __cplusplus
}
#endif

#endif /* _LANGINFO_H */

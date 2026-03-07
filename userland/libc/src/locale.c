/*
 * VeridianOS libc -- locale.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Locale implementation.  VeridianOS only supports the "C" / "POSIX"
 * locale, but we track per-category state so that setlocale() queries
 * return the correct string.  Includes nl_langinfo() for Qt 6.
 */

#include <locale.h>
#include <langinfo.h>

/* ========================================================================= */
/* Internal locale state                                                     */
/* ========================================================================= */

/* Number of individual locale categories (LC_CTYPE .. LC_MESSAGES) */
#define __LC_COUNT  6

/*
 * Per-category locale names.  Only "C" and "POSIX" are accepted;
 * everything else silently maps to "C".
 */
static char *__locale_names[__LC_COUNT] = {
    "C", "C", "C", "C", "C", "C"
};

/* Composite string returned for LC_ALL when all categories match. */
static char __lc_all_buf[128] = "C";

/* ========================================================================= */
/* setlocale                                                                 */
/* ========================================================================= */

/*
 * Accept "C", "POSIX", and "" (meaning "use the environment / default").
 * All resolve to "C".  Any other string is also silently accepted as "C"
 * rather than returning NULL, matching the behaviour of many embedded C
 * libraries.
 *
 * NULL locale means "query current" -- return without changing.
 */
char *setlocale(int category, const char *locale)
{
    if (category < 0 || category > LC_ALL)
        return (char *)0;

    /* Query mode: return current setting. */
    if (locale == (const char *)0) {
        if (category == LC_ALL)
            return __lc_all_buf;
        return __locale_names[category];
    }

    /* Set mode: everything maps to "C". */
    if (category == LC_ALL) {
        int i;
        for (i = 0; i < __LC_COUNT; i++)
            __locale_names[i] = "C";
    } else {
        __locale_names[category] = "C";
    }

    /* Rebuild LC_ALL composite string. */
    /* Since all categories are always "C", this is trivially "C". */
    __lc_all_buf[0] = 'C';
    __lc_all_buf[1] = '\0';

    if (category == LC_ALL)
        return __lc_all_buf;
    return __locale_names[category];
}

/* ========================================================================= */
/* localeconv                                                                */
/* ========================================================================= */

/*
 * Return a pointer to a static struct lconv initialised to the C locale
 * defaults.  CHAR_MAX (127 for signed char) indicates "not available".
 */
struct lconv *localeconv(void)
{
    static struct lconv c_locale = {
        .decimal_point      = ".",
        .thousands_sep      = "",
        .grouping           = "",
        .int_curr_symbol    = "",
        .currency_symbol    = "",
        .mon_decimal_point  = "",
        .mon_thousands_sep  = "",
        .mon_grouping       = "",
        .positive_sign      = "",
        .negative_sign      = "",
        .int_frac_digits    = 127,
        .frac_digits        = 127,
        .p_cs_precedes      = 127,
        .p_sep_by_space     = 127,
        .n_cs_precedes      = 127,
        .n_sep_by_space     = 127,
        .p_sign_posn        = 127,
        .n_sign_posn        = 127,
    };

    return &c_locale;
}

/* ========================================================================= */
/* nl_langinfo                                                               */
/* ========================================================================= */

/*
 * Return locale-specific information for the C/POSIX locale.
 * Qt 6 uses this for CODESET, date/time formats, and numeric formatting.
 */

static const char *__day_full[] = {
    "Sunday", "Monday", "Tuesday", "Wednesday",
    "Thursday", "Friday", "Saturday"
};

static const char *__day_abbr[] = {
    "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"
};

static const char *__mon_full[] = {
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December"
};

static const char *__mon_abbr[] = {
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"
};

char *nl_langinfo(nl_item item)
{
    switch (item) {
    case CODESET:       return "UTF-8";
    case D_T_FMT:       return "%a %b %e %H:%M:%S %Y";
    case D_FMT:         return "%m/%d/%y";
    case T_FMT:         return "%H:%M:%S";
    case T_FMT_AMPM:    return "%I:%M:%S %p";
    case AM_STR:        return "AM";
    case PM_STR:        return "PM";

    /* Full day names (DAY_1 = Sunday) */
    case DAY_1: case DAY_2: case DAY_3: case DAY_4:
    case DAY_5: case DAY_6: case DAY_7:
        return (char *)__day_full[item - DAY_1];

    /* Abbreviated day names */
    case ABDAY_1: case ABDAY_2: case ABDAY_3: case ABDAY_4:
    case ABDAY_5: case ABDAY_6: case ABDAY_7:
        return (char *)__day_abbr[item - ABDAY_1];

    /* Full month names */
    case MON_1:  case MON_2:  case MON_3:  case MON_4:
    case MON_5:  case MON_6:  case MON_7:  case MON_8:
    case MON_9:  case MON_10: case MON_11: case MON_12:
        return (char *)__mon_full[item - MON_1];

    /* Abbreviated month names */
    case ABMON_1:  case ABMON_2:  case ABMON_3:  case ABMON_4:
    case ABMON_5:  case ABMON_6:  case ABMON_7:  case ABMON_8:
    case ABMON_9:  case ABMON_10: case ABMON_11: case ABMON_12:
        return (char *)__mon_abbr[item - ABMON_1];

    /* Numeric formatting */
    case RADIXCHAR:     return ".";
    case THOUSEP:       return "";

    /* Yes/No expressions */
    case YESEXPR:       return "^[yY]";
    case NOEXPR:        return "^[nN]";

    /* Currency */
    case CRNCYSTR:      return "";

    /* Era-based (empty in C locale) */
    case ERA:           return "";
    case ERA_D_FMT:     return "";
    case ERA_D_T_FMT:   return "";
    case ERA_T_FMT:     return "";
    case ALT_DIGITS:    return "";

    default:            return "";
    }
}

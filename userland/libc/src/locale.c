/*
 * VeridianOS libc -- locale.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Locale stubs.  VeridianOS only supports the "C" / "POSIX" locale.
 * These are provided so programs that call setlocale(LC_ALL, "") or
 * localeconv() at startup can link and run without modification.
 */

#include <locale.h>

/* ========================================================================= */
/* setlocale                                                                 */
/* ========================================================================= */

/*
 * We always return "C".  Any request for a locale other than "C", "POSIX",
 * or "" (meaning "use the default") also returns "C" -- we silently ignore
 * the request rather than returning NULL.  This matches the behaviour of
 * many embedded C libraries.
 */
char *setlocale(int category, const char *locale)
{
    (void)category;
    (void)locale;
    return "C";
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

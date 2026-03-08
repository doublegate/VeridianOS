/*
 * VeridianOS libc -- unicode/uchar.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * ICU 75.x Unicode character properties API.
 */

#ifndef _UNICODE_UCHAR_H
#define _UNICODE_UCHAR_H

#include "utypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* General category values                                                   */
/* ========================================================================= */

typedef enum {
    U_UNASSIGNED              = 0,
    U_GENERAL_OTHER_TYPES     = 0,
    U_UPPERCASE_LETTER        = 1,
    U_LOWERCASE_LETTER        = 2,
    U_TITLECASE_LETTER        = 3,
    U_MODIFIER_LETTER         = 4,
    U_OTHER_LETTER            = 5,
    U_NON_SPACING_MARK        = 6,
    U_ENCLOSING_MARK          = 7,
    U_COMBINING_SPACING_MARK  = 8,
    U_DECIMAL_DIGIT_NUMBER    = 9,
    U_LETTER_NUMBER           = 10,
    U_OTHER_NUMBER            = 11,
    U_SPACE_SEPARATOR         = 12,
    U_LINE_SEPARATOR          = 13,
    U_PARAGRAPH_SEPARATOR     = 14,
    U_CONTROL_CHAR            = 15,
    U_FORMAT_CHAR             = 16,
    U_PRIVATE_USE_CHAR        = 17,
    U_SURROGATE               = 18,
    U_DASH_PUNCTUATION        = 19,
    U_START_PUNCTUATION       = 20,
    U_END_PUNCTUATION         = 21,
    U_CONNECTOR_PUNCTUATION   = 22,
    U_OTHER_PUNCTUATION       = 23,
    U_MATH_SYMBOL             = 24,
    U_CURRENCY_SYMBOL         = 25,
    U_MODIFIER_SYMBOL         = 26,
    U_OTHER_SYMBOL            = 27,
    U_INITIAL_PUNCTUATION     = 28,
    U_FINAL_PUNCTUATION       = 29,
    U_CHAR_CATEGORY_COUNT     = 30
} UCharCategory;

/* ========================================================================= */
/* Character property functions                                              */
/* ========================================================================= */

/** Get the general category of a code point. */
int8_t u_charType(UChar32 c);

/** Check if a code point is alphabetic. */
UBool u_isalpha(UChar32 c);

/** Check if a code point is a digit. */
UBool u_isdigit(UChar32 c);

/** Check if a code point is alphanumeric. */
UBool u_isalnum(UChar32 c);

/** Check if a code point is whitespace. */
UBool u_isspace(UChar32 c);

/** Check if a code point is a whitespace in Unicode sense. */
UBool u_isWhitespace(UChar32 c);

/** Check if a code point is uppercase. */
UBool u_isupper(UChar32 c);

/** Check if a code point is lowercase. */
UBool u_islower(UChar32 c);

/** Check if a code point is a title case letter. */
UBool u_istitle(UChar32 c);

/** Check if a code point is a control character. */
UBool u_iscntrl(UChar32 c);

/** Check if a code point is printable. */
UBool u_isprint(UChar32 c);

/** Convert to lowercase. */
UChar32 u_tolower(UChar32 c);

/** Convert to uppercase. */
UChar32 u_toupper(UChar32 c);

/** Convert to title case. */
UChar32 u_totitle(UChar32 c);

/** Get the numeric value of a digit character. */
int32_t u_charDigitValue(UChar32 c);

/** Check if a code point is defined (assigned). */
UBool u_isdefined(UChar32 c);

/** Get the Unicode block of a code point. */
int32_t ublock_getCode(UChar32 c);

#ifdef __cplusplus
}
#endif

#endif /* _UNICODE_UCHAR_H */

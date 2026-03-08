/*
 * VeridianOS libc -- harfbuzz/hb-unicode.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz Unicode property API.
 */

#ifndef _HARFBUZZ_HB_UNICODE_H
#define _HARFBUZZ_HB_UNICODE_H

#include <harfbuzz/hb-common.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct hb_unicode_funcs_t hb_unicode_funcs_t;

typedef enum {
    HB_UNICODE_GENERAL_CATEGORY_CONTROL,
    HB_UNICODE_GENERAL_CATEGORY_FORMAT,
    HB_UNICODE_GENERAL_CATEGORY_UNASSIGNED,
    HB_UNICODE_GENERAL_CATEGORY_PRIVATE_USE,
    HB_UNICODE_GENERAL_CATEGORY_SURROGATE,
    HB_UNICODE_GENERAL_CATEGORY_LOWERCASE_LETTER,
    HB_UNICODE_GENERAL_CATEGORY_MODIFIER_LETTER,
    HB_UNICODE_GENERAL_CATEGORY_OTHER_LETTER,
    HB_UNICODE_GENERAL_CATEGORY_TITLECASE_LETTER,
    HB_UNICODE_GENERAL_CATEGORY_UPPERCASE_LETTER,
    HB_UNICODE_GENERAL_CATEGORY_SPACING_MARK,
    HB_UNICODE_GENERAL_CATEGORY_ENCLOSING_MARK,
    HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK,
    HB_UNICODE_GENERAL_CATEGORY_DECIMAL_NUMBER,
    HB_UNICODE_GENERAL_CATEGORY_LETTER_NUMBER,
    HB_UNICODE_GENERAL_CATEGORY_OTHER_NUMBER,
    HB_UNICODE_GENERAL_CATEGORY_CONNECT_PUNCTUATION,
    HB_UNICODE_GENERAL_CATEGORY_DASH_PUNCTUATION,
    HB_UNICODE_GENERAL_CATEGORY_CLOSE_PUNCTUATION,
    HB_UNICODE_GENERAL_CATEGORY_FINAL_PUNCTUATION,
    HB_UNICODE_GENERAL_CATEGORY_INITIAL_PUNCTUATION,
    HB_UNICODE_GENERAL_CATEGORY_OTHER_PUNCTUATION,
    HB_UNICODE_GENERAL_CATEGORY_OPEN_PUNCTUATION,
    HB_UNICODE_GENERAL_CATEGORY_CURRENCY_SYMBOL,
    HB_UNICODE_GENERAL_CATEGORY_MODIFIER_SYMBOL,
    HB_UNICODE_GENERAL_CATEGORY_MATH_SYMBOL,
    HB_UNICODE_GENERAL_CATEGORY_OTHER_SYMBOL,
    HB_UNICODE_GENERAL_CATEGORY_LINE_SEPARATOR,
    HB_UNICODE_GENERAL_CATEGORY_PARAGRAPH_SEPARATOR,
    HB_UNICODE_GENERAL_CATEGORY_SPACE_SEPARATOR
} hb_unicode_general_category_t;

typedef enum {
    HB_UNICODE_COMBINING_CLASS_NOT_REORDERED = 0,
    HB_UNICODE_COMBINING_CLASS_ABOVE         = 230,
    HB_UNICODE_COMBINING_CLASS_BELOW         = 220,
    HB_UNICODE_COMBINING_CLASS_INVALID       = 255
} hb_unicode_combining_class_t;

hb_unicode_funcs_t *hb_unicode_funcs_get_default(void);
hb_unicode_funcs_t *hb_unicode_funcs_create(hb_unicode_funcs_t *parent);
hb_unicode_funcs_t *hb_unicode_funcs_get_empty(void);
hb_unicode_funcs_t *hb_unicode_funcs_reference(hb_unicode_funcs_t *ufuncs);
void                hb_unicode_funcs_destroy(hb_unicode_funcs_t *ufuncs);
void                hb_unicode_funcs_make_immutable(hb_unicode_funcs_t *ufuncs);
hb_bool_t           hb_unicode_funcs_is_immutable(hb_unicode_funcs_t *ufuncs);

hb_unicode_combining_class_t hb_unicode_combining_class(
    hb_unicode_funcs_t *ufuncs, hb_codepoint_t unicode);
hb_unicode_general_category_t hb_unicode_general_category(
    hb_unicode_funcs_t *ufuncs, hb_codepoint_t unicode);
hb_codepoint_t hb_unicode_mirroring(hb_unicode_funcs_t *ufuncs,
                                       hb_codepoint_t unicode);
hb_script_t hb_unicode_script(hb_unicode_funcs_t *ufuncs,
                                hb_codepoint_t unicode);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_UNICODE_H */

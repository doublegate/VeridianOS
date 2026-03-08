/*
 * VeridianOS libc -- unicode/ubrk.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * ICU 75.x break iterator (text segmentation) API.
 */

#ifndef _UNICODE_UBRK_H
#define _UNICODE_UBRK_H

#include "utypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/** Break iterator types. */
typedef enum {
    UBRK_CHARACTER = 0,
    UBRK_WORD      = 1,
    UBRK_LINE      = 2,
    UBRK_SENTENCE  = 3,
    UBRK_TITLE     = 4,
    UBRK_COUNT     = 5
} UBreakIteratorType;

/** Sentinel value for end of iteration. */
#define UBRK_DONE ((int32_t)-1)

/** Opaque break iterator type. */
typedef struct UBreakIterator UBreakIterator;

/** Open a break iterator. */
UBreakIterator *ubrk_open(UBreakIteratorType type, const char *locale,
                           const UChar *text, int32_t textLength,
                           UErrorCode *status);

/** Close a break iterator. */
void ubrk_close(UBreakIterator *bi);

/** Set the text to iterate over. */
void ubrk_setText(UBreakIterator *bi, const UChar *text,
                  int32_t textLength, UErrorCode *status);

/** Move to the first boundary. */
int32_t ubrk_first(UBreakIterator *bi);

/** Move to the last boundary. */
int32_t ubrk_last(UBreakIterator *bi);

/** Move to the next boundary. */
int32_t ubrk_next(UBreakIterator *bi);

/** Move to the previous boundary. */
int32_t ubrk_previous(UBreakIterator *bi);

/** Move to the boundary at or after the given offset. */
int32_t ubrk_following(UBreakIterator *bi, int32_t offset);

/** Move to the boundary at or before the given offset. */
int32_t ubrk_preceding(UBreakIterator *bi, int32_t offset);

/** Get the current position. */
int32_t ubrk_current(const UBreakIterator *bi);

#ifdef __cplusplus
}
#endif

#endif /* _UNICODE_UBRK_H */

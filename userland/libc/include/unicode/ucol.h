/*
 * VeridianOS libc -- unicode/ucol.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * ICU 75.x collation (locale-aware string comparison) API.
 */

#ifndef _UNICODE_UCOL_H
#define _UNICODE_UCOL_H

#include "utypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/** Opaque collator type. */
typedef struct UCollator UCollator;

/** Collation result. */
typedef enum {
    UCOL_EQUAL   = 0,
    UCOL_GREATER = 1,
    UCOL_LESS    = -1
} UCollationResult;

/** Collation strength levels. */
typedef enum {
    UCOL_DEFAULT   = -1,
    UCOL_PRIMARY   = 0,
    UCOL_SECONDARY = 1,
    UCOL_TERTIARY  = 2,
    UCOL_QUATERNARY = 3,
    UCOL_IDENTICAL = 15
} UCollationStrength;

/** Open a collator for a locale. */
UCollator *ucol_open(const char *loc, UErrorCode *status);

/** Close a collator. */
void ucol_close(UCollator *coll);

/** Compare two UChar strings using the collator. */
UCollationResult ucol_strcoll(const UCollator *coll,
                              const UChar *source, int32_t sourceLength,
                              const UChar *target, int32_t targetLength);

/** Get the collation strength. */
UCollationStrength ucol_getStrength(const UCollator *coll);

/** Set the collation strength. */
void ucol_setStrength(UCollator *coll, UCollationStrength newStrength);

/** Get a sort key for a string. */
int32_t ucol_getSortKey(const UCollator *coll,
                        const UChar *source, int32_t sourceLength,
                        uint8_t *result, int32_t resultLength);

#ifdef __cplusplus
}
#endif

#endif /* _UNICODE_UCOL_H */

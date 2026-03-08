/*
 * VeridianOS libc -- unicode/ucnv.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * ICU 75.x charset converter API.
 */

#ifndef _UNICODE_UCNV_H
#define _UNICODE_UCNV_H

#include "utypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/** Opaque converter type. */
typedef struct UConverter UConverter;

/** Open a converter by name (e.g., "UTF-8", "ISO-8859-1"). */
UConverter *ucnv_open(const char *converterName, UErrorCode *err);

/** Close a converter. */
void ucnv_close(UConverter *converter);

/** Get the name of a converter. */
const char *ucnv_getName(const UConverter *converter, UErrorCode *err);

/** Convert from a charset to UChar (UTF-16). */
void ucnv_toUChars(UConverter *cnv, UChar *dest, int32_t destCapacity,
                   const char *src, int32_t srcLength,
                   UErrorCode *pErrorCode);

/** Convert from UChar (UTF-16) to a charset. */
void ucnv_fromUChars(UConverter *cnv, char *dest, int32_t destCapacity,
                     const UChar *src, int32_t srcLength,
                     UErrorCode *pErrorCode);

/** Get the maximum number of bytes per char for this converter. */
int8_t ucnv_getMaxCharSize(const UConverter *converter);

/** Get the minimum number of bytes per char for this converter. */
int8_t ucnv_getMinCharSize(const UConverter *converter);

/** Get the number of available converter names. */
int32_t ucnv_countAvailable(void);

/** Get the name of an available converter by index. */
const char *ucnv_getAvailableName(int32_t n);

#ifdef __cplusplus
}
#endif

#endif /* _UNICODE_UCNV_H */

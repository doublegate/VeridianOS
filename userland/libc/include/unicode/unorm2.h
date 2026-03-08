/*
 * VeridianOS libc -- unicode/unorm2.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * ICU 75.x Unicode normalization API.
 */

#ifndef _UNICODE_UNORM2_H
#define _UNICODE_UNORM2_H

#include "utypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/** Opaque normalizer type. */
typedef struct UNormalizer2 UNormalizer2;

/** Normalization check results. */
typedef enum {
    UNORM_NO,
    UNORM_YES,
    UNORM_MAYBE
} UNormalizationCheckResult;

/** Get the NFC normalizer singleton. */
const UNormalizer2 *unorm2_getNFCInstance(UErrorCode *pErrorCode);

/** Get the NFD normalizer singleton. */
const UNormalizer2 *unorm2_getNFDInstance(UErrorCode *pErrorCode);

/** Get the NFKC normalizer singleton. */
const UNormalizer2 *unorm2_getNFKCInstance(UErrorCode *pErrorCode);

/** Get the NFKD normalizer singleton. */
const UNormalizer2 *unorm2_getNFKDInstance(UErrorCode *pErrorCode);

/** Normalize a string. */
int32_t unorm2_normalize(const UNormalizer2 *norm2,
                         const UChar *src, int32_t length,
                         UChar *dest, int32_t capacity,
                         UErrorCode *pErrorCode);

/** Check if a string is normalized. */
UNormalizationCheckResult unorm2_quickCheck(const UNormalizer2 *norm2,
                                            const UChar *s, int32_t length,
                                            UErrorCode *pErrorCode);

/** Check if a string is normalized (full check). */
UBool unorm2_isNormalized(const UNormalizer2 *norm2,
                          const UChar *s, int32_t length,
                          UErrorCode *pErrorCode);

#ifdef __cplusplus
}
#endif

#endif /* _UNICODE_UNORM2_H */

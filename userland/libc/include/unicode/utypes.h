/*
 * VeridianOS libc -- unicode/utypes.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * ICU 75.x base types and error codes.
 */

#ifndef _UNICODE_UTYPES_H
#define _UNICODE_UTYPES_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Basic types                                                               */
/* ========================================================================= */

typedef int8_t   UBool;
typedef uint16_t UChar;
typedef int32_t  UChar32;

#define U_SENTINEL (-1)

#ifndef TRUE
#define TRUE  1
#endif
#ifndef FALSE
#define FALSE 0
#endif

/* ========================================================================= */
/* Error codes                                                               */
/* ========================================================================= */

typedef enum UErrorCode {
    U_ZERO_ERROR               =  0,
    U_ILLEGAL_ARGUMENT_ERROR   =  1,
    U_MISSING_RESOURCE_ERROR   =  2,
    U_INVALID_FORMAT_ERROR     =  3,
    U_FILE_ACCESS_ERROR        =  4,
    U_INTERNAL_PROGRAM_ERROR   =  5,
    U_MEMORY_ALLOCATION_ERROR  =  7,
    U_INDEX_OUTOFBOUNDS_ERROR  =  8,
    U_PARSE_ERROR              =  9,
    U_INVALID_CHAR_FOUND       = 10,
    U_TRUNCATED_CHAR_FOUND     = 11,
    U_ILLEGAL_CHAR_FOUND       = 12,
    U_INVALID_TABLE_FORMAT     = 13,
    U_INVALID_TABLE_FILE       = 14,
    U_BUFFER_OVERFLOW_ERROR    = 15,
    U_UNSUPPORTED_ERROR        = 16,
    U_RESOURCE_TYPE_MISMATCH   = 17,
    U_ILLEGAL_ESCAPE_SEQUENCE  = 18,
    U_UNSUPPORTED_ESCAPE_SEQUENCE = 19,
    U_ERROR_LIMIT              = 20
} UErrorCode;

#define U_SUCCESS(x) ((x) <= U_ZERO_ERROR)
#define U_FAILURE(x) ((x) > U_ZERO_ERROR)

/* ========================================================================= */
/* ICU version info                                                          */
/* ========================================================================= */

#include "uversion.h"

#ifdef __cplusplus
}
#endif

#endif /* _UNICODE_UTYPES_H */

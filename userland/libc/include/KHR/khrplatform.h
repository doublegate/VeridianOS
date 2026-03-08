/*
 * VeridianOS libc -- <KHR/khrplatform.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Khronos platform definitions.
 * Defines portable integer types used by EGL and OpenGL ES headers.
 */

#ifndef _KHR_KHRPLATFORM_H
#define _KHR_KHRPLATFORM_H

/* Also define the standard Khronos guard to prevent double-inclusion
 * if a system khrplatform.h is on the include path. */
#ifndef __khrplatform_h_
#define __khrplatform_h_
#endif

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>

/* ========================================================================= */
/* Basic type definitions                                                    */
/* ========================================================================= */

typedef int32_t  khronos_int32_t;
typedef uint32_t khronos_uint32_t;
typedef int64_t  khronos_int64_t;
typedef uint64_t khronos_uint64_t;
typedef int8_t   khronos_int8_t;
typedef uint8_t  khronos_uint8_t;
typedef int16_t  khronos_int16_t;
typedef uint16_t khronos_uint16_t;
typedef float    khronos_float_t;

/** Signed size type (same size as a pointer) */
typedef intptr_t  khronos_intptr_t;
typedef uintptr_t khronos_uintptr_t;
typedef intptr_t  khronos_ssize_t;
typedef uintptr_t khronos_usize_t;

/* ========================================================================= */
/* Time types                                                                */
/* ========================================================================= */

/** Time in nanoseconds, stored as a signed 64-bit integer. */
typedef khronos_int64_t khronos_utime_nanoseconds_t;
typedef khronos_int64_t khronos_stime_nanoseconds_t;

/* ========================================================================= */
/* Boolean                                                                   */
/* ========================================================================= */

typedef enum {
    KHRONOS_FALSE = 0,
    KHRONOS_TRUE  = 1,
    KHRONOS_BOOLEAN_ENUM_FORCE_SIZE = 0x7FFFFFFF
} khronos_boolean_enum_t;

/* ========================================================================= */
/* Calling convention macros                                                 */
/* ========================================================================= */

#ifndef KHRONOS_APICALL
#define KHRONOS_APICALL
#endif

#ifndef KHRONOS_APIENTRY
#define KHRONOS_APIENTRY
#endif

#ifndef KHRONOS_APIATTRIBUTES
#define KHRONOS_APIATTRIBUTES
#endif

/* ========================================================================= */
/* Maximum values                                                            */
/* ========================================================================= */

#define KHRONOS_MAX_ENUM 0x7FFFFFFF

#ifdef __cplusplus
}
#endif

#endif /* _KHR_KHRPLATFORM_H */

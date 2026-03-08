/*
 * VeridianOS libc -- unicode/uversion.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * ICU 75.x version definitions.
 */

#ifndef _UNICODE_UVERSION_H
#define _UNICODE_UVERSION_H

#include <stdint.h>

#define U_ICU_VERSION_MAJOR_NUM 75
#define U_ICU_VERSION_MINOR_NUM 1
#define U_ICU_VERSION_PATCHLEVEL_NUM 0
#define U_ICU_VERSION "75.1"
#define U_ICU_VERSION_SHORT "75"

#define U_MAX_VERSION_LENGTH 4

typedef uint8_t UVersionInfo[U_MAX_VERSION_LENGTH];

#endif /* _UNICODE_UVERSION_H */

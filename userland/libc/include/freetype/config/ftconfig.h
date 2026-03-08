/*
 * VeridianOS libc -- freetype/config/ftconfig.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 build configuration.
 */

#ifndef _FREETYPE_CONFIG_FTCONFIG_H
#define _FREETYPE_CONFIG_FTCONFIG_H

#include <stdint.h>
#include <stddef.h>

/* Integer types */
typedef int32_t   FT_Int32;
typedef uint32_t  FT_UInt32;
typedef int64_t   FT_Int64;
typedef uint64_t  FT_UInt64;

/* Sizeof macros */
#define FT_SIZEOF_INT   4
#define FT_SIZEOF_LONG  8

/* Alignment */
#define FT_ALIGNMENT  8

#endif /* _FREETYPE_CONFIG_FTCONFIG_H */

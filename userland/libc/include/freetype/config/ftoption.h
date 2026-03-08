/*
 * VeridianOS libc -- freetype/config/ftoption.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 build options.
 */

#ifndef _FREETYPE_CONFIG_FTOPTION_H
#define _FREETYPE_CONFIG_FTOPTION_H

/* Enable TrueType hinting */
#define TT_CONFIG_OPTION_BYTECODE_INTERPRETER  1

/* Enable subpixel rendering */
#define FT_CONFIG_OPTION_SUBPIXEL_RENDERING    1

/* Enable glyph names */
#define FT_CONFIG_OPTION_POSTSCRIPT_NAMES      1

/* Enable incremental loading */
#define FT_CONFIG_OPTION_INCREMENTAL           1

/* Enable zlib for compressed fonts */
#define FT_CONFIG_OPTION_USE_ZLIB              1

/* Enable libpng for PNG-in-OpenType */
#define FT_CONFIG_OPTION_USE_PNG               1

#endif /* _FREETYPE_CONFIG_FTOPTION_H */

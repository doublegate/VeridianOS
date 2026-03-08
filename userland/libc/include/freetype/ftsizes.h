/*
 * VeridianOS libc -- freetype/ftsizes.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 size management API.
 */

#ifndef _FREETYPE_FTSIZES_H
#define _FREETYPE_FTSIZES_H

#include <freetype/freetype.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* API functions                                                             */
/* ========================================================================= */

FT_Error  FT_New_Size(FT_Face face, FT_Size *size);
FT_Error  FT_Done_Size(FT_Size size);
FT_Error  FT_Activate_Size(FT_Size size);

#ifdef __cplusplus
}
#endif

#endif /* _FREETYPE_FTSIZES_H */

/*
 * VeridianOS libc -- freetype/ftbitmap.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 bitmap manipulation API.
 */

#ifndef _FREETYPE_FTBITMAP_H
#define _FREETYPE_FTBITMAP_H

#include <freetype/freetype.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Color type for FT_Bitmap_Blend */
typedef struct FT_Color_ {
    FT_Byte  blue;
    FT_Byte  green;
    FT_Byte  red;
    FT_Byte  alpha;
} FT_Color;

/* Bitmap functions are declared in freetype.h:
 *   FT_Bitmap_Init, FT_Bitmap_Copy, FT_Bitmap_Convert, FT_Bitmap_Done
 *
 * Additional bitmap operations:
 */

FT_Error  FT_Bitmap_Embolden(FT_Library library, FT_Bitmap *bitmap,
                               FT_Pos xStrength, FT_Pos yStrength);
FT_Error  FT_Bitmap_Blend(FT_Library library, const FT_Bitmap *source,
                            const FT_Vector source_offset,
                            FT_Bitmap *target, FT_Vector *atarget_offset,
                            FT_Color color);

#ifdef __cplusplus
}
#endif

#endif /* _FREETYPE_FTBITMAP_H */

/*
 * VeridianOS libc -- freetype/ftstroke.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 stroker API for outline path stroking.
 */

#ifndef _FREETYPE_FTSTROKE_H
#define _FREETYPE_FTSTROKE_H

#include <freetype/freetype.h>
#include <freetype/ftglyph.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Stroker types                                                             */
/* ========================================================================= */

typedef struct FT_StrokerRec_ *FT_Stroker;

typedef enum FT_Stroker_LineCap_ {
    FT_STROKER_LINECAP_BUTT   = 0,
    FT_STROKER_LINECAP_ROUND  = 1,
    FT_STROKER_LINECAP_SQUARE = 2
} FT_Stroker_LineCap;

typedef enum FT_Stroker_LineJoin_ {
    FT_STROKER_LINEJOIN_ROUND          = 0,
    FT_STROKER_LINEJOIN_BEVEL          = 1,
    FT_STROKER_LINEJOIN_MITER_VARIABLE = 2,
    FT_STROKER_LINEJOIN_MITER          = 2,
    FT_STROKER_LINEJOIN_MITER_FIXED    = 3
} FT_Stroker_LineJoin;

typedef enum FT_StrokerBorder_ {
    FT_STROKER_BORDER_LEFT  = 0,
    FT_STROKER_BORDER_RIGHT = 1
} FT_StrokerBorder;

/* ========================================================================= */
/* API functions                                                             */
/* ========================================================================= */

FT_Error  FT_Stroker_New(FT_Library library, FT_Stroker *astroker);
void      FT_Stroker_Set(FT_Stroker stroker, FT_Fixed radius,
                          FT_Stroker_LineCap line_cap,
                          FT_Stroker_LineJoin line_join,
                          FT_Fixed miter_limit);
FT_Error  FT_Stroker_ParseOutline(FT_Stroker stroker,
                                    FT_Outline *outline, FT_Bool opened);
FT_Error  FT_Stroker_GetBorderCounts(FT_Stroker stroker,
                                       FT_StrokerBorder border,
                                       FT_UInt *anum_points,
                                       FT_UInt *anum_contours);
FT_Error  FT_Stroker_GetCounts(FT_Stroker stroker,
                                 FT_UInt *anum_points,
                                 FT_UInt *anum_contours);
void      FT_Stroker_ExportBorder(FT_Stroker stroker,
                                    FT_StrokerBorder border,
                                    FT_Outline *outline);
void      FT_Stroker_Export(FT_Stroker stroker, FT_Outline *outline);
FT_Error  FT_Glyph_Stroke(FT_Glyph *pglyph, FT_Stroker stroker,
                            FT_Bool destroy);
FT_Error  FT_Glyph_StrokeBorder(FT_Glyph *pglyph, FT_Stroker stroker,
                                  FT_Bool inside, FT_Bool destroy);
void      FT_Stroker_Done(FT_Stroker stroker);

#ifdef __cplusplus
}
#endif

#endif /* _FREETYPE_FTSTROKE_H */

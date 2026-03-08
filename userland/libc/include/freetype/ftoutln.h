/*
 * VeridianOS libc -- freetype/ftoutln.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 outline processing API.
 */

#ifndef _FREETYPE_FTOUTLN_H
#define _FREETYPE_FTOUTLN_H

#include <freetype/freetype.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Outline funcs (callback interface)                                        */
/* ========================================================================= */

typedef int (*FT_Outline_MoveToFunc)(const FT_Vector *to, void *user);
typedef int (*FT_Outline_LineToFunc)(const FT_Vector *to, void *user);
typedef int (*FT_Outline_ConicToFunc)(const FT_Vector *control,
                                       const FT_Vector *to, void *user);
typedef int (*FT_Outline_CubicToFunc)(const FT_Vector *control1,
                                       const FT_Vector *control2,
                                       const FT_Vector *to, void *user);

typedef struct FT_Outline_Funcs_ {
    FT_Outline_MoveToFunc   move_to;
    FT_Outline_LineToFunc   line_to;
    FT_Outline_ConicToFunc  conic_to;
    FT_Outline_CubicToFunc  cubic_to;
    int                     shift;
    FT_Pos                  delta;
} FT_Outline_Funcs;

/* ========================================================================= */
/* API functions                                                             */
/* ========================================================================= */

FT_Error  FT_Outline_New(FT_Library library, FT_UInt numPoints,
                           FT_Int numContours, FT_Outline *anoutline);
FT_Error  FT_Outline_Done(FT_Library library, FT_Outline *outline);
FT_Error  FT_Outline_Copy(const FT_Outline *source, FT_Outline *target);
void      FT_Outline_Translate(const FT_Outline *outline,
                                FT_Pos xOffset, FT_Pos yOffset);
void      FT_Outline_Transform(const FT_Outline *outline,
                                const FT_Matrix *matrix);
FT_Error  FT_Outline_Embolden(FT_Outline *outline, FT_Pos strength);
FT_Error  FT_Outline_EmboldenXY(FT_Outline *outline,
                                  FT_Pos xstrength, FT_Pos ystrength);
void      FT_Outline_Reverse(FT_Outline *outline);
FT_Error  FT_Outline_Check(FT_Outline *outline);
void      FT_Outline_Get_CBox(const FT_Outline *outline, FT_BBox *acbox);
FT_Error  FT_Outline_Get_Bitmap(FT_Library library,
                                  FT_Outline *outline, FT_Bitmap *bitmap);
FT_Error  FT_Outline_Render(FT_Library library, FT_Outline *outline,
                              FT_Raster_Params *params);
FT_Error  FT_Outline_Decompose(FT_Outline *outline,
                                 const FT_Outline_Funcs *func_interface,
                                 void *user);

/* Orientation enum */
typedef enum FT_Orientation_ {
    FT_ORIENTATION_TRUETYPE   = 0,
    FT_ORIENTATION_POSTSCRIPT = 1,
    FT_ORIENTATION_NONE
} FT_Orientation;

FT_Orientation FT_Outline_Get_Orientation(FT_Outline *outline);

#ifdef __cplusplus
}
#endif

#endif /* _FREETYPE_FTOUTLN_H */

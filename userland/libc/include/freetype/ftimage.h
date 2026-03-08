/*
 * VeridianOS libc -- freetype/ftimage.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 image/bitmap and outline types.
 */

#ifndef _FREETYPE_FTIMAGE_H
#define _FREETYPE_FTIMAGE_H

#include <freetype/fttypes.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Vector / Matrix                                                           */
/* ========================================================================= */

typedef struct FT_Vector_ {
    FT_Pos  x;
    FT_Pos  y;
} FT_Vector;

typedef struct FT_Matrix_ {
    FT_Fixed  xx, xy;
    FT_Fixed  yx, yy;
} FT_Matrix;

typedef struct FT_BBox_ {
    FT_Pos  xMin, yMin;
    FT_Pos  xMax, yMax;
} FT_BBox;

/* ========================================================================= */
/* Bitmap                                                                    */
/* ========================================================================= */

typedef enum FT_Pixel_Mode_ {
    FT_PIXEL_MODE_NONE  = 0,
    FT_PIXEL_MODE_MONO  = 1,
    FT_PIXEL_MODE_GRAY  = 2,
    FT_PIXEL_MODE_GRAY2 = 3,
    FT_PIXEL_MODE_GRAY4 = 4,
    FT_PIXEL_MODE_LCD   = 5,
    FT_PIXEL_MODE_LCD_V = 6,
    FT_PIXEL_MODE_BGRA  = 7,
    FT_PIXEL_MODE_MAX
} FT_Pixel_Mode;

typedef struct FT_Bitmap_ {
    unsigned int    rows;
    unsigned int    width;
    int             pitch;
    unsigned char  *buffer;
    unsigned short  num_grays;
    unsigned char   pixel_mode;
    unsigned char   palette_mode;
    void           *palette;
} FT_Bitmap;

/* ========================================================================= */
/* Outline                                                                   */
/* ========================================================================= */

#define FT_OUTLINE_NONE           0x0
#define FT_OUTLINE_OWNER          0x1
#define FT_OUTLINE_EVEN_ODD_FILL  0x2
#define FT_OUTLINE_REVERSE_FILL   0x4
#define FT_OUTLINE_IGNORE_DROPOUTS 0x8
#define FT_OUTLINE_SMART_DROPOUTS 0x10
#define FT_OUTLINE_INCLUDE_STUBS  0x20
#define FT_OUTLINE_HIGH_PRECISION 0x100
#define FT_OUTLINE_SINGLE_PASS    0x200

#define FT_CURVE_TAG_ON           0x01
#define FT_CURVE_TAG_CONIC        0x00
#define FT_CURVE_TAG_CUBIC        0x02

typedef struct FT_Outline_ {
    short         n_contours;
    short         n_points;
    FT_Vector    *points;
    char         *tags;
    short        *contours;
    int           flags;
} FT_Outline;

/* ========================================================================= */
/* Glyph format                                                              */
/* ========================================================================= */

typedef enum FT_Glyph_Format_ {
    FT_GLYPH_FORMAT_NONE      = 0,
    FT_GLYPH_FORMAT_COMPOSITE = 0x636F6D70,  /* 'comp' */
    FT_GLYPH_FORMAT_BITMAP    = 0x62697473,  /* 'bits' */
    FT_GLYPH_FORMAT_OUTLINE   = 0x6F75746C,  /* 'outl' */
    FT_GLYPH_FORMAT_PLOTTER   = 0x706C6F74,  /* 'plot' */
    FT_GLYPH_FORMAT_SVG       = 0x53564720   /* 'SVG ' */
} FT_Glyph_Format;

/* ========================================================================= */
/* Render mode                                                               */
/* ========================================================================= */

typedef enum FT_Render_Mode_ {
    FT_RENDER_MODE_NORMAL = 0,
    FT_RENDER_MODE_LIGHT  = 1,
    FT_RENDER_MODE_MONO   = 2,
    FT_RENDER_MODE_LCD    = 3,
    FT_RENDER_MODE_LCD_V  = 4,
    FT_RENDER_MODE_SDF    = 5,
    FT_RENDER_MODE_MAX
} FT_Render_Mode;

/* ========================================================================= */
/* Raster types                                                              */
/* ========================================================================= */

typedef struct FT_Span_ {
    short          x;
    unsigned short len;
    unsigned char  coverage;
} FT_Span;

typedef void (*FT_SpanFunc)(int y, int count, const FT_Span *spans,
                            void *user);

typedef struct FT_Raster_Params_ {
    const FT_Bitmap  *target;
    const void       *source;
    int               flags;
    FT_SpanFunc       gray_spans;
    FT_SpanFunc       black_spans; /* unused */
    void (* bit_test)(int y, int x, void *user);
    void (* bit_set)(int y, int x, void *user);
    void             *user;
    FT_BBox           clip_box;
} FT_Raster_Params;

#define FT_RASTER_FLAG_DEFAULT  0x0
#define FT_RASTER_FLAG_AA       0x1
#define FT_RASTER_FLAG_DIRECT   0x2
#define FT_RASTER_FLAG_CLIP     0x4
#define FT_RASTER_FLAG_SDF      0x8

#ifdef __cplusplus
}
#endif

#endif /* _FREETYPE_FTIMAGE_H */

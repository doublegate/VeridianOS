/*
 * VeridianOS libc -- freetype/freetype.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2 core API declarations: library lifecycle, face loading,
 * glyph rendering, charmap selection, kerning, and metrics.
 */

#ifndef _FREETYPE_FREETYPE_H
#define _FREETYPE_FREETYPE_H

#include <freetype/config/ftconfig.h>
#include <freetype/fttypes.h>
#include <freetype/ftimage.h>
#include <freetype/fterrors.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Version                                                                   */
/* ========================================================================= */

#define FREETYPE_MAJOR  2
#define FREETYPE_MINOR  13
#define FREETYPE_PATCH  3

/* ========================================================================= */
/* Encoding                                                                  */
/* ========================================================================= */

typedef enum FT_Encoding_ {
    FT_ENCODING_NONE           = 0,
    FT_ENCODING_MS_SYMBOL      = 0x73796D62,  /* 'symb' */
    FT_ENCODING_UNICODE        = 0x756E6963,  /* 'unic' */
    FT_ENCODING_SJIS           = 0x736A6973,  /* 'sjis' */
    FT_ENCODING_PRC            = 0x67622020,  /* 'gb  ' */
    FT_ENCODING_BIG5           = 0x62696735,  /* 'big5' */
    FT_ENCODING_WANSUNG        = 0x77616E73,  /* 'wans' */
    FT_ENCODING_JOHAB          = 0x6A6F6861,  /* 'joha' */
    FT_ENCODING_ADOBE_STANDARD = 0x41444F42,  /* 'ADOB' */
    FT_ENCODING_ADOBE_EXPERT   = 0x41444245,  /* 'ADBE' */
    FT_ENCODING_ADOBE_CUSTOM   = 0x41444243,  /* 'ADBC' */
    FT_ENCODING_ADOBE_LATIN_1  = 0x6C617431,  /* 'lat1' */
    FT_ENCODING_OLD_LATIN_2    = 0x6C617432,  /* 'lat2' */
    FT_ENCODING_APPLE_ROMAN    = 0x61726D6E   /* 'armn' */
} FT_Encoding;

#define ft_encoding_none           FT_ENCODING_NONE
#define ft_encoding_unicode        FT_ENCODING_UNICODE
#define ft_encoding_symbol         FT_ENCODING_MS_SYMBOL
#define ft_encoding_latin_1        FT_ENCODING_ADOBE_LATIN_1
#define ft_encoding_apple_roman    FT_ENCODING_APPLE_ROMAN

/* ========================================================================= */
/* Face flags                                                                */
/* ========================================================================= */

#define FT_FACE_FLAG_SCALABLE          (1L << 0)
#define FT_FACE_FLAG_FIXED_SIZES       (1L << 1)
#define FT_FACE_FLAG_FIXED_WIDTH       (1L << 2)
#define FT_FACE_FLAG_SFNT              (1L << 3)
#define FT_FACE_FLAG_HORIZONTAL        (1L << 4)
#define FT_FACE_FLAG_VERTICAL          (1L << 5)
#define FT_FACE_FLAG_KERNING           (1L << 6)
#define FT_FACE_FLAG_FAST_GLYPHS       (1L << 7)
#define FT_FACE_FLAG_MULTIPLE_MASTERS  (1L << 8)
#define FT_FACE_FLAG_GLYPH_NAMES      (1L << 9)
#define FT_FACE_FLAG_EXTERNAL_STREAM  (1L << 10)
#define FT_FACE_FLAG_HINTER           (1L << 11)
#define FT_FACE_FLAG_CID_KEYED        (1L << 12)
#define FT_FACE_FLAG_TRICKY           (1L << 13)
#define FT_FACE_FLAG_COLOR            (1L << 14)
#define FT_FACE_FLAG_VARIATION        (1L << 15)
#define FT_FACE_FLAG_SVG              (1L << 16)
#define FT_FACE_FLAG_SBIX             (1L << 17)
#define FT_FACE_FLAG_SBIX_OVERLAY     (1L << 18)

#define FT_HAS_HORIZONTAL(face)  ((face)->face_flags & FT_FACE_FLAG_HORIZONTAL)
#define FT_HAS_VERTICAL(face)    ((face)->face_flags & FT_FACE_FLAG_VERTICAL)
#define FT_HAS_KERNING(face)     ((face)->face_flags & FT_FACE_FLAG_KERNING)
#define FT_IS_SCALABLE(face)     ((face)->face_flags & FT_FACE_FLAG_SCALABLE)
#define FT_IS_SFNT(face)         ((face)->face_flags & FT_FACE_FLAG_SFNT)
#define FT_IS_FIXED_WIDTH(face)  ((face)->face_flags & FT_FACE_FLAG_FIXED_WIDTH)
#define FT_HAS_GLYPH_NAMES(face) ((face)->face_flags & FT_FACE_FLAG_GLYPH_NAMES)
#define FT_HAS_COLOR(face)       ((face)->face_flags & FT_FACE_FLAG_COLOR)

/* ========================================================================= */
/* Style flags                                                               */
/* ========================================================================= */

#define FT_STYLE_FLAG_ITALIC  (1 << 0)
#define FT_STYLE_FLAG_BOLD    (1 << 1)

/* ========================================================================= */
/* Load flags                                                                */
/* ========================================================================= */

#define FT_LOAD_DEFAULT                     0x0
#define FT_LOAD_NO_SCALE                    (1L << 0)
#define FT_LOAD_NO_HINTING                  (1L << 1)
#define FT_LOAD_RENDER                      (1L << 2)
#define FT_LOAD_NO_BITMAP                   (1L << 3)
#define FT_LOAD_VERTICAL_LAYOUT             (1L << 4)
#define FT_LOAD_FORCE_AUTOHINT              (1L << 5)
#define FT_LOAD_CROP_BITMAP                 (1L << 6)
#define FT_LOAD_PEDANTIC                    (1L << 7)
#define FT_LOAD_IGNORE_GLOBAL_ADVANCE_WIDTH (1L << 9)
#define FT_LOAD_NO_RECURSE                  (1L << 10)
#define FT_LOAD_IGNORE_TRANSFORM            (1L << 11)
#define FT_LOAD_MONOCHROME                  (1L << 12)
#define FT_LOAD_LINEAR_DESIGN               (1L << 13)
#define FT_LOAD_NO_AUTOHINT                 (1L << 15)
#define FT_LOAD_COLOR                       (1L << 20)
#define FT_LOAD_COMPUTE_METRICS             (1L << 21)
#define FT_LOAD_BITMAP_METRICS_ONLY         (1L << 22)
#define FT_LOAD_NO_SVG                      (1L << 24)

#define FT_LOAD_TARGET_NORMAL   (FT_RENDER_MODE_NORMAL << 16)
#define FT_LOAD_TARGET_LIGHT    (FT_RENDER_MODE_LIGHT  << 16)
#define FT_LOAD_TARGET_MONO     (FT_RENDER_MODE_MONO   << 16)
#define FT_LOAD_TARGET_LCD      (FT_RENDER_MODE_LCD    << 16)
#define FT_LOAD_TARGET_LCD_V    (FT_RENDER_MODE_LCD_V  << 16)

/* ========================================================================= */
/* Kerning mode                                                              */
/* ========================================================================= */

typedef enum FT_Kerning_Mode_ {
    FT_KERNING_DEFAULT  = 0,
    FT_KERNING_UNFITTED = 1,
    FT_KERNING_UNSCALED = 2
} FT_Kerning_Mode;

/* ========================================================================= */
/* Size metrics                                                              */
/* ========================================================================= */

typedef struct FT_Size_Metrics_ {
    FT_UShort  x_ppem;
    FT_UShort  y_ppem;
    FT_Fixed   x_scale;
    FT_Fixed   y_scale;
    FT_Pos     ascender;
    FT_Pos     descender;
    FT_Pos     height;
    FT_Pos     max_advance;
} FT_Size_Metrics;

/* ========================================================================= */
/* Glyph metrics                                                             */
/* ========================================================================= */

typedef struct FT_Glyph_Metrics_ {
    FT_Pos  width;
    FT_Pos  height;
    FT_Pos  horiBearingX;
    FT_Pos  horiBearingY;
    FT_Pos  horiAdvance;
    FT_Pos  vertBearingX;
    FT_Pos  vertBearingY;
    FT_Pos  vertAdvance;
} FT_Glyph_Metrics;

/* ========================================================================= */
/* Forward declarations                                                      */
/* ========================================================================= */

typedef struct FT_LibraryRec_  *FT_Library;
typedef struct FT_FaceRec_     *FT_Face;
typedef struct FT_SizeRec_     *FT_Size;
typedef struct FT_GlyphSlotRec_ *FT_GlyphSlot;
typedef struct FT_CharMapRec_  *FT_CharMap;

/* ========================================================================= */
/* CharMap                                                                   */
/* ========================================================================= */

typedef struct FT_CharMapRec_ {
    FT_Face       face;
    FT_Encoding   encoding;
    FT_UShort     platform_id;
    FT_UShort     encoding_id;
} FT_CharMapRec;

/* ========================================================================= */
/* GlyphSlot                                                                 */
/* ========================================================================= */

typedef struct FT_GlyphSlotRec_ {
    FT_Library         library;
    FT_Face            face;
    FT_GlyphSlot       next;
    FT_UInt            glyph_index;
    FT_Generic         generic;

    FT_Glyph_Metrics   metrics;
    FT_Fixed           linearHoriAdvance;
    FT_Fixed           linearVertAdvance;
    FT_Vector          advance;

    FT_Glyph_Format    format;
    FT_Bitmap          bitmap;
    FT_Int             bitmap_left;
    FT_Int             bitmap_top;

    FT_Outline         outline;

    FT_UInt            num_subglyphs;
    void              *subglyphs;

    void              *control_data;
    long               control_len;

    FT_Pos             lsb_delta;
    FT_Pos             rsb_delta;

    void              *other;
    void              *internal;
} FT_GlyphSlotRec;

/* ========================================================================= */
/* Size                                                                      */
/* ========================================================================= */

typedef struct FT_SizeRec_ {
    FT_Face           face;
    FT_Generic        generic;
    FT_Size_Metrics   metrics;
    void             *internal;
} FT_SizeRec;

/* ========================================================================= */
/* Bitmap size (for fixed-size fonts)                                        */
/* ========================================================================= */

typedef struct FT_Bitmap_Size_ {
    FT_Short  height;
    FT_Short  width;
    FT_Pos    size;
    FT_Pos    x_ppem;
    FT_Pos    y_ppem;
} FT_Bitmap_Size;

/* ========================================================================= */
/* Face                                                                      */
/* ========================================================================= */

typedef struct FT_FaceRec_ {
    FT_Long            num_faces;
    FT_Long            face_index;

    FT_Long            face_flags;
    FT_Long            style_flags;

    FT_Long            num_glyphs;

    FT_String         *family_name;
    FT_String         *style_name;

    FT_Int             num_fixed_sizes;
    FT_Bitmap_Size    *available_sizes;

    FT_Int             num_charmaps;
    FT_CharMap        *charmaps;

    FT_Generic         generic;

    FT_BBox            bbox;

    FT_UShort          units_per_EM;
    FT_Short           ascender;
    FT_Short           descender;
    FT_Short           height;

    FT_Short           max_advance_width;
    FT_Short           max_advance_height;

    FT_Short           underline_position;
    FT_Short           underline_thickness;

    FT_GlyphSlot       glyph;
    FT_Size            size;
    FT_CharMap         charmap;

    /* Private fields */
    void              *driver;
    FT_Memory          memory;
    void              *stream;
    FT_ListRec         sizes_list;
    FT_Generic         autohint;
    void              *extensions;
    void              *internal;
} FT_FaceRec;

/* ========================================================================= */
/* Open args                                                                 */
/* ========================================================================= */

#define FT_OPEN_MEMORY    0x1
#define FT_OPEN_STREAM    0x2
#define FT_OPEN_PATHNAME  0x4
#define FT_OPEN_DRIVER    0x8
#define FT_OPEN_PARAMS    0x10

typedef struct FT_Parameter_ {
    FT_ULong   tag;
    FT_Pointer data;
} FT_Parameter;

typedef struct FT_Open_Args_ {
    FT_UInt         flags;
    const FT_Byte  *memory_base;
    FT_Long         memory_size;
    FT_String      *pathname;
    void           *stream;
    void           *driver;
    FT_Int          num_params;
    FT_Parameter   *params;
} FT_Open_Args;

/* ========================================================================= */
/* Core API functions                                                        */
/* ========================================================================= */

/* Library lifecycle */
FT_Error  FT_Init_FreeType(FT_Library *alibrary);
FT_Error  FT_Done_FreeType(FT_Library library);

/* Version query */
void      FT_Library_Version(FT_Library library,
                              FT_Int *amajor, FT_Int *aminor,
                              FT_Int *apatch);

/* Face loading */
FT_Error  FT_New_Face(FT_Library library, const char *filepathname,
                       FT_Long face_index, FT_Face *aface);
FT_Error  FT_New_Memory_Face(FT_Library library, const FT_Byte *file_base,
                              FT_Long file_size, FT_Long face_index,
                              FT_Face *aface);
FT_Error  FT_Open_Face(FT_Library library, const FT_Open_Args *args,
                        FT_Long face_index, FT_Face *aface);
FT_Error  FT_Done_Face(FT_Face face);
FT_Error  FT_Reference_Face(FT_Face face);

/* Size selection */
FT_Error  FT_Set_Char_Size(FT_Face face, FT_F26Dot6 char_width,
                            FT_F26Dot6 char_height,
                            FT_UInt horz_resolution,
                            FT_UInt vert_resolution);
FT_Error  FT_Set_Pixel_Sizes(FT_Face face, FT_UInt pixel_width,
                              FT_UInt pixel_height);

/* Glyph loading and rendering */
FT_Error  FT_Load_Glyph(FT_Face face, FT_UInt glyph_index,
                          FT_Int32 load_flags);
FT_Error  FT_Load_Char(FT_Face face, FT_ULong char_code,
                         FT_Int32 load_flags);
FT_Error  FT_Render_Glyph(FT_GlyphSlot slot, FT_Render_Mode render_mode);

/* Character mapping */
FT_UInt   FT_Get_Char_Index(FT_Face face, FT_ULong charcode);
FT_Error  FT_Select_Charmap(FT_Face face, FT_Encoding encoding);
FT_Error  FT_Set_Charmap(FT_Face face, FT_CharMap charmap);
FT_ULong  FT_Get_First_Char(FT_Face face, FT_UInt *agindex);
FT_ULong  FT_Get_Next_Char(FT_Face face, FT_ULong char_code,
                             FT_UInt *agindex);
FT_Int    FT_Get_Charmap_Index(FT_CharMap charmap);
FT_UInt   FT_Get_Name_Index(FT_Face face, const FT_String *glyph_name);
FT_Error  FT_Get_Glyph_Name(FT_Face face, FT_UInt glyph_index,
                              FT_Pointer buffer, FT_UInt buffer_max);
const char *FT_Get_Postscript_Name(FT_Face face);

/* Kerning */
FT_Error  FT_Get_Kerning(FT_Face face, FT_UInt left_glyph,
                           FT_UInt right_glyph, FT_UInt kern_mode,
                           FT_Vector *akerning);
FT_Error  FT_Get_Track_Kerning(FT_Face face, FT_Fixed point_size,
                                FT_Int degree, FT_Fixed *akerning);

/* Transform */
void      FT_Set_Transform(FT_Face face, FT_Matrix *matrix,
                            FT_Vector *delta);
void      FT_Get_Transform(FT_Face face, FT_Matrix *matrix,
                            FT_Vector *delta);

/* Face properties */
FT_Long   FT_Get_Sfnt_Name_Count(FT_Face face);
FT_Error  FT_Get_FSType_Flags(FT_Face face, FT_UShort *flags);

/* Bitmap operations */
void      FT_Bitmap_Init(FT_Bitmap *bitmap);
FT_Error  FT_Bitmap_Copy(FT_Library library, const FT_Bitmap *source,
                           FT_Bitmap *target);
FT_Error  FT_Bitmap_Convert(FT_Library library, const FT_Bitmap *source,
                              FT_Bitmap *target, FT_Int alignment);
FT_Error  FT_Bitmap_Done(FT_Library library, FT_Bitmap *bitmap);

#ifdef __cplusplus
}
#endif

#endif /* _FREETYPE_FREETYPE_H */

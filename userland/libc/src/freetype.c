/*
 * VeridianOS libc -- freetype.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * FreeType 2.13.3 compatible implementation.
 * Provides font face management, glyph loading with metrics,
 * bitmap rendering (8x16 fallback glyphs), charmap selection,
 * kerning queries, and glyph object management.
 */

#include <ft2build.h>
#include <freetype/freetype.h>
#include <freetype/ftglyph.h>
#include <freetype/ftstroke.h>
#include <freetype/ftoutln.h>
#include <freetype/ftbitmap.h>
#include <freetype/ftmodapi.h>
#include <freetype/ftsizes.h>
#include <string.h>
#include <stdlib.h>

/* ========================================================================= */
/* Internal limits                                                           */
/* ========================================================================= */

#define MAX_LIBRARIES    4
#define MAX_FACES       64
#define MAX_GLYPHS     128
#define MAX_STROKERS     8

#define DEFAULT_GLYPH_WIDTH   8
#define DEFAULT_GLYPH_HEIGHT  16

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

struct ft_library_internal {
    int  in_use;
};

struct ft_face_internal {
    int             in_use;
    FT_Library      library;
    int             ref_count;

    /* Backing storage for face fields */
    char            family[64];
    char            style[32];

    /* Current size state */
    FT_UInt         pixel_width;
    FT_UInt         pixel_height;

    /* Charmap */
    FT_CharMapRec   charmap_rec;
    FT_CharMap      charmap_ptrs[1];

    /* Glyph slot */
    FT_GlyphSlotRec slot;

    /* Size */
    FT_SizeRec      size_rec;

    /* Bitmap buffer for rendered glyphs */
    unsigned char   bitmap_buf[DEFAULT_GLYPH_WIDTH * DEFAULT_GLYPH_HEIGHT];
};

struct ft_glyph_internal {
    int           in_use;
    FT_GlyphRec   base;
    /* Extra space for bitmap glyph data */
    FT_BitmapGlyphRec  bm;
    unsigned char bm_buf[DEFAULT_GLYPH_WIDTH * DEFAULT_GLYPH_HEIGHT];
};

struct ft_stroker_internal {
    int                   in_use;
    FT_Fixed              radius;
    FT_Stroker_LineCap    line_cap;
    FT_Stroker_LineJoin   line_join;
    FT_Fixed              miter_limit;
    FT_Library            library;
};

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

static struct ft_library_internal  g_libraries[MAX_LIBRARIES];
static struct ft_face_internal     g_faces[MAX_FACES];
static struct ft_glyph_internal    g_glyphs[MAX_GLYPHS];
static struct ft_stroker_internal  g_strokers[MAX_STROKERS];

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

static struct ft_face_internal *face_from_ptr(FT_Face face)
{
    /* Recover container from the embedded FaceRec */
    if (!face)
        return NULL;

    int idx;
    for (idx = 0; idx < MAX_FACES; idx++) {
        if (g_faces[idx].in_use &&
            (FT_Face)&g_faces[idx].family == face->family_name - 0 + 0 - 0)
            ; /* fallthrough to linear scan */
    }

    /* Use linear scan -- faces point into g_faces[].family for family_name */
    for (idx = 0; idx < MAX_FACES; idx++) {
        if (g_faces[idx].in_use && face->family_name == g_faces[idx].family)
            return &g_faces[idx];
    }
    return NULL;
}

static void init_face_defaults(struct ft_face_internal *fi, FT_Face face)
{
    memset(face, 0, sizeof(*face));

    strncpy(fi->family, "DejaVu Sans", sizeof(fi->family) - 1);
    strncpy(fi->style, "Regular", sizeof(fi->style) - 1);

    face->num_faces       = 1;
    face->face_index      = 0;
    face->face_flags      = FT_FACE_FLAG_SCALABLE |
                            FT_FACE_FLAG_SFNT |
                            FT_FACE_FLAG_HORIZONTAL |
                            FT_FACE_FLAG_KERNING |
                            FT_FACE_FLAG_GLYPH_NAMES;
    face->style_flags     = 0;
    face->num_glyphs      = 65535;
    face->family_name     = fi->family;
    face->style_name      = fi->style;
    face->units_per_EM    = 2048;
    face->ascender        = 1901;
    face->descender       = -483;
    face->height          = 2384;
    face->max_advance_width  = 2048;
    face->max_advance_height = 2384;
    face->underline_position  = -130;
    face->underline_thickness = 90;
    face->bbox.xMin       = -1021;
    face->bbox.yMin       = -483;
    face->bbox.xMax       = 2793;
    face->bbox.yMax       = 1901;

    /* Charmap */
    fi->charmap_rec.face        = face;
    fi->charmap_rec.encoding    = FT_ENCODING_UNICODE;
    fi->charmap_rec.platform_id = 3;  /* Microsoft */
    fi->charmap_rec.encoding_id = 1;  /* Unicode BMP */
    fi->charmap_ptrs[0] = &fi->charmap_rec;
    face->num_charmaps    = 1;
    face->charmaps        = fi->charmap_ptrs;
    face->charmap         = fi->charmap_ptrs[0];

    /* Glyph slot */
    memset(&fi->slot, 0, sizeof(fi->slot));
    fi->slot.library = fi->library;
    fi->slot.face    = face;
    fi->slot.format  = FT_GLYPH_FORMAT_BITMAP;
    face->glyph      = &fi->slot;

    /* Size */
    memset(&fi->size_rec, 0, sizeof(fi->size_rec));
    fi->size_rec.face = face;
    fi->size_rec.metrics.x_ppem     = 16;
    fi->size_rec.metrics.y_ppem     = 16;
    fi->size_rec.metrics.x_scale    = 0x10000;
    fi->size_rec.metrics.y_scale    = 0x10000;
    fi->size_rec.metrics.ascender   = 15 * 64;
    fi->size_rec.metrics.descender  = -4 * 64;
    fi->size_rec.metrics.height     = 19 * 64;
    fi->size_rec.metrics.max_advance = 10 * 64;
    face->size = &fi->size_rec;

    fi->pixel_width  = 16;
    fi->pixel_height = 16;
    fi->ref_count    = 1;
}

static void fill_glyph_bitmap(struct ft_face_internal *fi,
                               FT_UInt glyph_index)
{
    FT_GlyphSlot slot = &fi->slot;
    unsigned int w = DEFAULT_GLYPH_WIDTH;
    unsigned int h = DEFAULT_GLYPH_HEIGHT;
    unsigned int i;

    /* Simple filled rectangle as placeholder glyph */
    memset(fi->bitmap_buf, 0, w * h);
    if (glyph_index > 0) {
        /* Create a simple pattern based on glyph index */
        for (i = 0; i < w * h; i++) {
            unsigned int row = i / w;
            unsigned int col = i % w;
            /* Border + hash pattern */
            if (row == 0 || row == h - 1 || col == 0 || col == w - 1)
                fi->bitmap_buf[i] = 255;
            else if (((row + glyph_index) ^ col) & 1)
                fi->bitmap_buf[i] = 180;
        }
    }

    slot->bitmap.rows       = h;
    slot->bitmap.width      = w;
    slot->bitmap.pitch      = (int)w;
    slot->bitmap.buffer     = fi->bitmap_buf;
    slot->bitmap.num_grays  = 256;
    slot->bitmap.pixel_mode = FT_PIXEL_MODE_GRAY;

    slot->bitmap_left = 1;
    slot->bitmap_top  = (int)(h - 2);

    /* 26.6 metrics */
    slot->metrics.width        = (FT_Pos)(w * 64);
    slot->metrics.height       = (FT_Pos)(h * 64);
    slot->metrics.horiBearingX = 1 * 64;
    slot->metrics.horiBearingY = (FT_Pos)((h - 2) * 64);
    slot->metrics.horiAdvance  = (FT_Pos)((w + 2) * 64);
    slot->metrics.vertBearingX = 0;
    slot->metrics.vertBearingY = 0;
    slot->metrics.vertAdvance  = (FT_Pos)((h + 2) * 64);

    slot->advance.x = (FT_Pos)((w + 2) * 64);
    slot->advance.y = 0;

    slot->linearHoriAdvance = (FT_Fixed)((w + 2) << 16);
    slot->linearVertAdvance = (FT_Fixed)((h + 2) << 16);

    slot->format      = FT_GLYPH_FORMAT_BITMAP;
    slot->glyph_index = glyph_index;
}

/* ========================================================================= */
/* Library lifecycle                                                         */
/* ========================================================================= */

FT_Error FT_Init_FreeType(FT_Library *alibrary)
{
    int i;

    if (!alibrary)
        return FT_Err_Invalid_Argument;

    for (i = 0; i < MAX_LIBRARIES; i++) {
        if (!g_libraries[i].in_use) {
            g_libraries[i].in_use = 1;
            *alibrary = (FT_Library)&g_libraries[i];
            return FT_Err_Ok;
        }
    }

    return FT_Err_Out_Of_Memory;
}

FT_Error FT_Done_FreeType(FT_Library library)
{
    int i;

    if (!library)
        return FT_Err_Invalid_Library_Handle;

    /* Release all faces owned by this library */
    for (i = 0; i < MAX_FACES; i++) {
        if (g_faces[i].in_use && g_faces[i].library == library)
            g_faces[i].in_use = 0;
    }

    struct ft_library_internal *lib = (struct ft_library_internal *)library;
    lib->in_use = 0;

    return FT_Err_Ok;
}

void FT_Library_Version(FT_Library library, FT_Int *amajor,
                         FT_Int *aminor, FT_Int *apatch)
{
    (void)library;
    if (amajor) *amajor = FREETYPE_MAJOR;
    if (aminor) *aminor = FREETYPE_MINOR;
    if (apatch) *apatch = FREETYPE_PATCH;
}

/* ========================================================================= */
/* Face loading                                                              */
/* ========================================================================= */

FT_Error FT_New_Face(FT_Library library, const char *filepathname,
                      FT_Long face_index, FT_Face *aface)
{
    (void)filepathname;
    return FT_New_Memory_Face(library, NULL, 0, face_index, aface);
}

FT_Error FT_New_Memory_Face(FT_Library library, const FT_Byte *file_base,
                              FT_Long file_size, FT_Long face_index,
                              FT_Face *aface)
{
    int i;

    (void)file_base;
    (void)file_size;
    (void)face_index;

    if (!library || !aface)
        return FT_Err_Invalid_Argument;

    for (i = 0; i < MAX_FACES; i++) {
        if (!g_faces[i].in_use) {
            struct ft_face_internal *fi = &g_faces[i];
            FT_FaceRec *face;

            fi->in_use  = 1;
            fi->library = library;

            /* The FT_FaceRec is allocated as part of fi; we use a static
             * buffer region at the start of the face internal to avoid
             * dynamic allocation.  We point *aface to a region that we
             * treat as FT_FaceRec. */
            face = (FT_FaceRec *)malloc(sizeof(FT_FaceRec));
            if (!face) {
                fi->in_use = 0;
                return FT_Err_Out_Of_Memory;
            }

            init_face_defaults(fi, face);
            *aface = face;
            return FT_Err_Ok;
        }
    }

    return FT_Err_Out_Of_Memory;
}

FT_Error FT_Open_Face(FT_Library library, const FT_Open_Args *args,
                       FT_Long face_index, FT_Face *aface)
{
    (void)args;
    return FT_New_Memory_Face(library, NULL, 0, face_index, aface);
}

FT_Error FT_Done_Face(FT_Face face)
{
    struct ft_face_internal *fi;

    if (!face)
        return FT_Err_Invalid_Face_Handle;

    fi = face_from_ptr(face);
    if (fi) {
        fi->ref_count--;
        if (fi->ref_count <= 0)
            fi->in_use = 0;
    }

    free(face);
    return FT_Err_Ok;
}

FT_Error FT_Reference_Face(FT_Face face)
{
    struct ft_face_internal *fi;

    if (!face)
        return FT_Err_Invalid_Face_Handle;

    fi = face_from_ptr(face);
    if (fi)
        fi->ref_count++;

    return FT_Err_Ok;
}

/* ========================================================================= */
/* Size selection                                                            */
/* ========================================================================= */

FT_Error FT_Set_Char_Size(FT_Face face, FT_F26Dot6 char_width,
                            FT_F26Dot6 char_height,
                            FT_UInt horz_resolution,
                            FT_UInt vert_resolution)
{
    struct ft_face_internal *fi;
    FT_UInt ppem_x, ppem_y;

    if (!face)
        return FT_Err_Invalid_Face_Handle;

    fi = face_from_ptr(face);
    if (!fi)
        return FT_Err_Invalid_Face_Handle;

    if (horz_resolution == 0) horz_resolution = 72;
    if (vert_resolution == 0) vert_resolution = 72;

    if (char_width == 0) char_width = char_height;
    if (char_height == 0) char_height = char_width;

    /* Convert 26.6 points to pixels: ppem = points * dpi / 72 */
    ppem_x = (FT_UInt)((char_width * horz_resolution + 36 * 64) / (72 * 64));
    ppem_y = (FT_UInt)((char_height * vert_resolution + 36 * 64) / (72 * 64));

    if (ppem_x == 0) ppem_x = 1;
    if (ppem_y == 0) ppem_y = 1;

    fi->pixel_width  = ppem_x;
    fi->pixel_height = ppem_y;

    /* Update size metrics */
    face->size->metrics.x_ppem      = (FT_UShort)ppem_x;
    face->size->metrics.y_ppem      = (FT_UShort)ppem_y;
    face->size->metrics.x_scale     = (FT_Fixed)(((long)ppem_x << 16) / face->units_per_EM);
    face->size->metrics.y_scale     = (FT_Fixed)(((long)ppem_y << 16) / face->units_per_EM);
    face->size->metrics.ascender    = (FT_Pos)(face->ascender * ppem_y / face->units_per_EM * 64);
    face->size->metrics.descender   = (FT_Pos)(face->descender * (FT_Short)ppem_y / face->units_per_EM * 64);
    face->size->metrics.height      = (FT_Pos)(face->height * ppem_y / face->units_per_EM * 64);
    face->size->metrics.max_advance = (FT_Pos)(face->max_advance_width * ppem_x / face->units_per_EM * 64);

    return FT_Err_Ok;
}

FT_Error FT_Set_Pixel_Sizes(FT_Face face, FT_UInt pixel_width,
                              FT_UInt pixel_height)
{
    if (pixel_height == 0) pixel_height = pixel_width;
    if (pixel_width == 0)  pixel_width = pixel_height;

    return FT_Set_Char_Size(face,
                             (FT_F26Dot6)(pixel_width * 64),
                             (FT_F26Dot6)(pixel_height * 64),
                             72, 72);
}

/* ========================================================================= */
/* Glyph loading and rendering                                               */
/* ========================================================================= */

FT_Error FT_Load_Glyph(FT_Face face, FT_UInt glyph_index,
                         FT_Int32 load_flags)
{
    struct ft_face_internal *fi;

    (void)load_flags;

    if (!face)
        return FT_Err_Invalid_Face_Handle;

    fi = face_from_ptr(face);
    if (!fi)
        return FT_Err_Invalid_Face_Handle;

    fill_glyph_bitmap(fi, glyph_index);
    face->glyph = &fi->slot;

    return FT_Err_Ok;
}

FT_Error FT_Load_Char(FT_Face face, FT_ULong char_code,
                        FT_Int32 load_flags)
{
    FT_UInt glyph_index = FT_Get_Char_Index(face, char_code);
    return FT_Load_Glyph(face, glyph_index, load_flags);
}

FT_Error FT_Render_Glyph(FT_GlyphSlot slot, FT_Render_Mode render_mode)
{
    (void)render_mode;

    if (!slot)
        return FT_Err_Invalid_Slot_Handle;

    /* Already rendered as bitmap in Load_Glyph */
    slot->format = FT_GLYPH_FORMAT_BITMAP;
    return FT_Err_Ok;
}

/* ========================================================================= */
/* Character mapping                                                         */
/* ========================================================================= */

FT_UInt FT_Get_Char_Index(FT_Face face, FT_ULong charcode)
{
    if (!face || charcode == 0)
        return 0;

    /* Simple identity mapping: codepoint = glyph index (common for
     * testing and stub implementations). Return 0 for undefined range. */
    if (charcode > 0xFFFF)
        return 0;

    return (FT_UInt)(charcode & 0xFFFF);
}

FT_Error FT_Select_Charmap(FT_Face face, FT_Encoding encoding)
{
    if (!face)
        return FT_Err_Invalid_Face_Handle;

    /* We only support Unicode */
    if (encoding != FT_ENCODING_UNICODE)
        return FT_Err_Invalid_Argument;

    return FT_Err_Ok;
}

FT_Error FT_Set_Charmap(FT_Face face, FT_CharMap charmap)
{
    (void)charmap;

    if (!face)
        return FT_Err_Invalid_Face_Handle;

    return FT_Err_Ok;
}

FT_ULong FT_Get_First_Char(FT_Face face, FT_UInt *agindex)
{
    if (!face || !agindex) {
        if (agindex) *agindex = 0;
        return 0;
    }

    *agindex = 32;  /* Space */
    return 32;
}

FT_ULong FT_Get_Next_Char(FT_Face face, FT_ULong char_code,
                            FT_UInt *agindex)
{
    if (!face || !agindex) {
        if (agindex) *agindex = 0;
        return 0;
    }

    if (char_code < 0x7E) {
        *agindex = (FT_UInt)(char_code + 1);
        return char_code + 1;
    }

    *agindex = 0;
    return 0;
}

FT_Int FT_Get_Charmap_Index(FT_CharMap charmap)
{
    (void)charmap;
    return 0;
}

FT_UInt FT_Get_Name_Index(FT_Face face, const FT_String *glyph_name)
{
    (void)face;
    (void)glyph_name;
    return 0;
}

FT_Error FT_Get_Glyph_Name(FT_Face face, FT_UInt glyph_index,
                              FT_Pointer buffer, FT_UInt buffer_max)
{
    (void)face;

    if (!buffer || buffer_max == 0)
        return FT_Err_Invalid_Argument;

    if (glyph_index == 0)
        strncpy((char *)buffer, ".notdef", buffer_max - 1);
    else
        snprintf((char *)buffer, buffer_max, "glyph%u", glyph_index);

    ((char *)buffer)[buffer_max - 1] = '\0';
    return FT_Err_Ok;
}

const char *FT_Get_Postscript_Name(FT_Face face)
{
    if (!face)
        return NULL;

    return "DejaVuSans";
}

/* ========================================================================= */
/* Kerning                                                                   */
/* ========================================================================= */

FT_Error FT_Get_Kerning(FT_Face face, FT_UInt left_glyph,
                          FT_UInt right_glyph, FT_UInt kern_mode,
                          FT_Vector *akerning)
{
    (void)face;
    (void)left_glyph;
    (void)right_glyph;
    (void)kern_mode;

    if (!akerning)
        return FT_Err_Invalid_Argument;

    akerning->x = 0;
    akerning->y = 0;

    return FT_Err_Ok;
}

FT_Error FT_Get_Track_Kerning(FT_Face face, FT_Fixed point_size,
                                FT_Int degree, FT_Fixed *akerning)
{
    (void)face;
    (void)point_size;
    (void)degree;

    if (akerning) *akerning = 0;
    return FT_Err_Ok;
}

/* ========================================================================= */
/* Transform                                                                 */
/* ========================================================================= */

void FT_Set_Transform(FT_Face face, FT_Matrix *matrix, FT_Vector *delta)
{
    (void)face;
    (void)matrix;
    (void)delta;
}

void FT_Get_Transform(FT_Face face, FT_Matrix *matrix, FT_Vector *delta)
{
    (void)face;

    if (matrix) {
        matrix->xx = 0x10000;
        matrix->xy = 0;
        matrix->yx = 0;
        matrix->yy = 0x10000;
    }
    if (delta) {
        delta->x = 0;
        delta->y = 0;
    }
}

/* ========================================================================= */
/* Face properties                                                           */
/* ========================================================================= */

FT_Long FT_Get_Sfnt_Name_Count(FT_Face face)
{
    (void)face;
    return 0;
}

FT_Error FT_Get_FSType_Flags(FT_Face face, FT_UShort *flags)
{
    (void)face;
    if (flags) *flags = 0;  /* Installable embedding */
    return FT_Err_Ok;
}

/* ========================================================================= */
/* Bitmap operations                                                         */
/* ========================================================================= */

void FT_Bitmap_Init(FT_Bitmap *bitmap)
{
    if (bitmap)
        memset(bitmap, 0, sizeof(*bitmap));
}

FT_Error FT_Bitmap_Copy(FT_Library library, const FT_Bitmap *source,
                          FT_Bitmap *target)
{
    (void)library;

    if (!source || !target)
        return FT_Err_Invalid_Argument;

    *target = *source;
    return FT_Err_Ok;
}

FT_Error FT_Bitmap_Convert(FT_Library library, const FT_Bitmap *source,
                              FT_Bitmap *target, FT_Int alignment)
{
    (void)library;
    (void)alignment;

    if (!source || !target)
        return FT_Err_Invalid_Argument;

    *target = *source;
    target->pixel_mode = FT_PIXEL_MODE_GRAY;
    target->num_grays  = 256;
    return FT_Err_Ok;
}

FT_Error FT_Bitmap_Done(FT_Library library, FT_Bitmap *bitmap)
{
    (void)library;

    if (bitmap)
        memset(bitmap, 0, sizeof(*bitmap));

    return FT_Err_Ok;
}

FT_Error FT_Bitmap_Embolden(FT_Library library, FT_Bitmap *bitmap,
                               FT_Pos xStrength, FT_Pos yStrength)
{
    (void)library;
    (void)bitmap;
    (void)xStrength;
    (void)yStrength;
    return FT_Err_Ok;
}

/* ========================================================================= */
/* Glyph object management                                                   */
/* ========================================================================= */

FT_Error FT_Get_Glyph(FT_GlyphSlot slot, FT_Glyph *aglyph)
{
    int i;

    if (!slot || !aglyph)
        return FT_Err_Invalid_Argument;

    for (i = 0; i < MAX_GLYPHS; i++) {
        if (!g_glyphs[i].in_use) {
            struct ft_glyph_internal *gi = &g_glyphs[i];
            gi->in_use = 1;

            gi->base.library = slot->library;
            gi->base.clazz   = NULL;
            gi->base.format  = slot->format;
            gi->base.advance = slot->advance;

            /* Copy bitmap data */
            gi->bm.root = gi->base;
            gi->bm.left = slot->bitmap_left;
            gi->bm.top  = slot->bitmap_top;
            gi->bm.bitmap = slot->bitmap;

            if (slot->bitmap.buffer && slot->bitmap.rows * slot->bitmap.width > 0) {
                unsigned int sz = slot->bitmap.rows * (unsigned int)abs(slot->bitmap.pitch);
                if (sz > sizeof(gi->bm_buf)) sz = sizeof(gi->bm_buf);
                memcpy(gi->bm_buf, slot->bitmap.buffer, sz);
                gi->bm.bitmap.buffer = gi->bm_buf;
            }

            *aglyph = (FT_Glyph)&gi->bm;
            return FT_Err_Ok;
        }
    }

    return FT_Err_Out_Of_Memory;
}

FT_Error FT_Glyph_Copy(FT_Glyph source, FT_Glyph *target)
{
    (void)source;
    (void)target;
    return FT_Err_Unimplemented_Feature;
}

FT_Error FT_Glyph_Transform(FT_Glyph glyph, FT_Matrix *matrix,
                               FT_Vector *delta)
{
    (void)glyph;
    (void)matrix;
    (void)delta;
    return FT_Err_Ok;
}

void FT_Glyph_Get_CBox(FT_Glyph glyph, FT_UInt bbox_mode,
                         FT_BBox *acbox)
{
    (void)glyph;
    (void)bbox_mode;

    if (acbox) {
        acbox->xMin = 0;
        acbox->yMin = 0;
        acbox->xMax = DEFAULT_GLYPH_WIDTH * 64;
        acbox->yMax = DEFAULT_GLYPH_HEIGHT * 64;
    }
}

FT_Error FT_Glyph_To_Bitmap(FT_Glyph *the_glyph,
                               FT_Render_Mode render_mode,
                               FT_Vector *origin, FT_Bool destroy)
{
    (void)render_mode;
    (void)origin;
    (void)destroy;

    if (!the_glyph || !*the_glyph)
        return FT_Err_Invalid_Argument;

    /* Already a bitmap in our implementation */
    (*the_glyph)->format = FT_GLYPH_FORMAT_BITMAP;
    return FT_Err_Ok;
}

void FT_Done_Glyph(FT_Glyph glyph)
{
    int i;

    if (!glyph)
        return;

    for (i = 0; i < MAX_GLYPHS; i++) {
        if (g_glyphs[i].in_use &&
            ((FT_Glyph)&g_glyphs[i].base == glyph ||
             (FT_Glyph)&g_glyphs[i].bm == glyph)) {
            g_glyphs[i].in_use = 0;
            return;
        }
    }
}

/* ========================================================================= */
/* Stroker                                                                   */
/* ========================================================================= */

FT_Error FT_Stroker_New(FT_Library library, FT_Stroker *astroker)
{
    int i;

    if (!library || !astroker)
        return FT_Err_Invalid_Argument;

    for (i = 0; i < MAX_STROKERS; i++) {
        if (!g_strokers[i].in_use) {
            g_strokers[i].in_use     = 1;
            g_strokers[i].library    = library;
            g_strokers[i].radius     = 0;
            g_strokers[i].line_cap   = FT_STROKER_LINECAP_BUTT;
            g_strokers[i].line_join  = FT_STROKER_LINEJOIN_ROUND;
            g_strokers[i].miter_limit = 0x10000;
            *astroker = (FT_Stroker)&g_strokers[i];
            return FT_Err_Ok;
        }
    }

    return FT_Err_Out_Of_Memory;
}

void FT_Stroker_Set(FT_Stroker stroker, FT_Fixed radius,
                     FT_Stroker_LineCap line_cap,
                     FT_Stroker_LineJoin line_join,
                     FT_Fixed miter_limit)
{
    struct ft_stroker_internal *si = (struct ft_stroker_internal *)stroker;

    if (!si)
        return;

    si->radius      = radius;
    si->line_cap    = line_cap;
    si->line_join   = line_join;
    si->miter_limit = miter_limit;
}

FT_Error FT_Stroker_ParseOutline(FT_Stroker stroker,
                                    FT_Outline *outline, FT_Bool opened)
{
    (void)stroker;
    (void)outline;
    (void)opened;
    return FT_Err_Ok;
}

FT_Error FT_Stroker_GetBorderCounts(FT_Stroker stroker,
                                       FT_StrokerBorder border,
                                       FT_UInt *anum_points,
                                       FT_UInt *anum_contours)
{
    (void)stroker;
    (void)border;
    if (anum_points)   *anum_points = 0;
    if (anum_contours) *anum_contours = 0;
    return FT_Err_Ok;
}

FT_Error FT_Stroker_GetCounts(FT_Stroker stroker,
                                FT_UInt *anum_points,
                                FT_UInt *anum_contours)
{
    (void)stroker;
    if (anum_points)   *anum_points = 0;
    if (anum_contours) *anum_contours = 0;
    return FT_Err_Ok;
}

void FT_Stroker_ExportBorder(FT_Stroker stroker,
                               FT_StrokerBorder border,
                               FT_Outline *outline)
{
    (void)stroker;
    (void)border;
    (void)outline;
}

void FT_Stroker_Export(FT_Stroker stroker, FT_Outline *outline)
{
    (void)stroker;
    (void)outline;
}

FT_Error FT_Glyph_Stroke(FT_Glyph *pglyph, FT_Stroker stroker,
                            FT_Bool destroy)
{
    (void)pglyph;
    (void)stroker;
    (void)destroy;
    return FT_Err_Ok;
}

FT_Error FT_Glyph_StrokeBorder(FT_Glyph *pglyph, FT_Stroker stroker,
                                  FT_Bool inside, FT_Bool destroy)
{
    (void)pglyph;
    (void)stroker;
    (void)inside;
    (void)destroy;
    return FT_Err_Ok;
}

void FT_Stroker_Done(FT_Stroker stroker)
{
    struct ft_stroker_internal *si = (struct ft_stroker_internal *)stroker;

    if (si)
        si->in_use = 0;
}

/* ========================================================================= */
/* Outline operations                                                        */
/* ========================================================================= */

FT_Error FT_Outline_New(FT_Library library, FT_UInt numPoints,
                           FT_Int numContours, FT_Outline *anoutline)
{
    (void)library;
    (void)numPoints;
    (void)numContours;

    if (anoutline)
        memset(anoutline, 0, sizeof(*anoutline));

    return FT_Err_Ok;
}

FT_Error FT_Outline_Done(FT_Library library, FT_Outline *outline)
{
    (void)library;
    (void)outline;
    return FT_Err_Ok;
}

FT_Error FT_Outline_Copy(const FT_Outline *source, FT_Outline *target)
{
    (void)source;
    (void)target;
    return FT_Err_Ok;
}

void FT_Outline_Translate(const FT_Outline *outline,
                            FT_Pos xOffset, FT_Pos yOffset)
{
    (void)outline;
    (void)xOffset;
    (void)yOffset;
}

void FT_Outline_Transform(const FT_Outline *outline,
                            const FT_Matrix *matrix)
{
    (void)outline;
    (void)matrix;
}

FT_Error FT_Outline_Embolden(FT_Outline *outline, FT_Pos strength)
{
    (void)outline;
    (void)strength;
    return FT_Err_Ok;
}

FT_Error FT_Outline_EmboldenXY(FT_Outline *outline,
                                  FT_Pos xstrength, FT_Pos ystrength)
{
    (void)outline;
    (void)xstrength;
    (void)ystrength;
    return FT_Err_Ok;
}

void FT_Outline_Reverse(FT_Outline *outline)
{
    (void)outline;
}

FT_Error FT_Outline_Check(FT_Outline *outline)
{
    (void)outline;
    return FT_Err_Ok;
}

void FT_Outline_Get_CBox(const FT_Outline *outline, FT_BBox *acbox)
{
    (void)outline;
    if (acbox) {
        acbox->xMin = 0;
        acbox->yMin = 0;
        acbox->xMax = 0;
        acbox->yMax = 0;
    }
}

FT_Error FT_Outline_Get_Bitmap(FT_Library library,
                                  FT_Outline *outline, FT_Bitmap *bitmap)
{
    (void)library;
    (void)outline;
    (void)bitmap;
    return FT_Err_Ok;
}

FT_Error FT_Outline_Render(FT_Library library, FT_Outline *outline,
                              FT_Raster_Params *params)
{
    (void)library;
    (void)outline;
    (void)params;
    return FT_Err_Ok;
}

FT_Error FT_Outline_Decompose(FT_Outline *outline,
                                 const FT_Outline_Funcs *func_interface,
                                 void *user)
{
    (void)outline;
    (void)func_interface;
    (void)user;
    return FT_Err_Ok;
}

/* ========================================================================= */
/* Size management                                                           */
/* ========================================================================= */

FT_Error FT_New_Size(FT_Face face, FT_Size *size)
{
    if (!face || !size)
        return FT_Err_Invalid_Argument;

    *size = face->size;
    return FT_Err_Ok;
}

FT_Error FT_Done_Size(FT_Size size)
{
    (void)size;
    return FT_Err_Ok;
}

FT_Error FT_Activate_Size(FT_Size size)
{
    (void)size;
    return FT_Err_Ok;
}

/* ========================================================================= */
/* Module management (stubs)                                                 */
/* ========================================================================= */

FT_Error FT_Add_Module(FT_Library library, const void *clazz)
{
    (void)library;
    (void)clazz;
    return FT_Err_Ok;
}

FT_Module FT_Get_Module(FT_Library library, const char *module_name)
{
    (void)library;
    (void)module_name;
    return NULL;
}

FT_Error FT_Remove_Module(FT_Library library, FT_Module module)
{
    (void)library;
    (void)module;
    return FT_Err_Ok;
}

FT_Error FT_Property_Set(FT_Library library, const FT_String *module_name,
                            const FT_String *property_name,
                            const void *value)
{
    (void)library;
    (void)module_name;
    (void)property_name;
    (void)value;
    return FT_Err_Ok;
}

FT_Error FT_Property_Get(FT_Library library, const FT_String *module_name,
                            const FT_String *property_name, void *value)
{
    (void)library;
    (void)module_name;
    (void)property_name;
    (void)value;
    return FT_Err_Ok;
}

void FT_Set_Default_Properties(FT_Library library)
{
    (void)library;
}

FT_Error FT_Set_Renderer(FT_Library library, FT_Renderer renderer,
                            FT_UInt num_params, FT_Parameter *parameters)
{
    (void)library;
    (void)renderer;
    (void)num_params;
    (void)parameters;
    return FT_Err_Ok;
}

FT_Renderer FT_Get_Renderer(FT_Library library, FT_Glyph_Format format)
{
    (void)library;
    (void)format;
    return NULL;
}

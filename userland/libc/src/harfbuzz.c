/*
 * VeridianOS libc -- harfbuzz.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz 9.0.0 compatible implementation.
 * Provides text shaping with 1:1 Latin glyph mapping, buffer
 * management, font/face lifecycle, blob storage, FreeType
 * integration, and OpenType layout stubs.
 */

#include <harfbuzz/hb.h>
#include <harfbuzz/hb-ft.h>
#include <string.h>
#include <stdlib.h>

/* ========================================================================= */
/* Internal limits                                                           */
/* ========================================================================= */

#define MAX_BUFFERS      256
#define MAX_BUFFER_GLYPHS 4096
#define MAX_FONTS         64
#define MAX_FACES         64
#define MAX_BLOBS         64
#define MAX_SETS          32
#define MAX_MAPS          16

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

struct hb_blob_internal {
    int               in_use;
    int               ref_count;
    const char       *data;
    unsigned int      length;
    hb_memory_mode_t  mode;
    void             *user_data;
    hb_destroy_func_t destroy;
    int               immutable;
};

struct hb_face_internal {
    int               in_use;
    int               ref_count;
    unsigned int      index;
    unsigned int      upem;
    unsigned int      glyph_count;
    int               immutable;
    struct hb_blob_internal *blob;
};

struct hb_font_internal {
    int               in_use;
    int               ref_count;
    struct hb_face_internal *face;
    int               x_scale;
    int               y_scale;
    unsigned int      x_ppem;
    unsigned int      y_ppem;
    float             ptem;
    int               immutable;
    FT_Face           ft_face;         /* For hb-ft integration */
    int               ft_load_flags;
};

struct hb_buffer_internal {
    int                    in_use;
    int                    ref_count;
    hb_glyph_info_t        infos[MAX_BUFFER_GLYPHS];
    hb_glyph_position_t    positions[MAX_BUFFER_GLYPHS];
    unsigned int            length;
    hb_direction_t          direction;
    hb_script_t             script;
    hb_language_t           language;
    hb_buffer_flags_t       flags;
    hb_buffer_cluster_level_t cluster_level;
    hb_buffer_content_type_t  content_type;
    int                     has_positions;
};

struct hb_set_internal {
    int            in_use;
    int            ref_count;
    /* Bitmap for codepoints 0-1023 (sufficient for basic use) */
    uint32_t       bits[32];  /* 1024 bits */
    unsigned int   population;
};

struct hb_map_internal {
    int            in_use;
    int            ref_count;
    /* Simple linear array for small maps */
    hb_codepoint_t keys[256];
    hb_codepoint_t values[256];
    unsigned int   count;
};

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

static struct hb_blob_internal   g_blobs[MAX_BLOBS];
static struct hb_face_internal   g_faces[MAX_FACES];
static struct hb_font_internal   g_fonts[MAX_FONTS];
static struct hb_buffer_internal g_buffers[MAX_BUFFERS];
static struct hb_set_internal    g_sets[MAX_SETS];
static struct hb_map_internal    g_maps[MAX_MAPS];

/* Language singleton */
static const char g_default_lang[] = "en";
struct hb_language_impl_t {
    const char *s;
};
static struct hb_language_impl_t g_lang_en = { "en" };
static struct hb_language_impl_t g_lang_custom[16];
static int g_lang_count = 0;

/* Unicode funcs singleton */
static struct { int dummy; } g_unicode_funcs_default;

/* Shaper list */
static const char *g_shapers[] = { "ot", "fallback", NULL };

/* ========================================================================= */
/* Version                                                                   */
/* ========================================================================= */

void hb_version(unsigned int *major, unsigned int *minor,
                  unsigned int *micro)
{
    if (major) *major = HB_VERSION_MAJOR;
    if (minor) *minor = HB_VERSION_MINOR;
    if (micro) *micro = HB_VERSION_MICRO;
}

const char *hb_version_string(void)
{
    return HB_VERSION_STRING;
}

hb_bool_t hb_version_atleast(unsigned int major, unsigned int minor,
                                unsigned int micro)
{
    return HB_VERSION_ATLEAST(major, minor, micro);
}

/* ========================================================================= */
/* Direction                                                                 */
/* ========================================================================= */

hb_direction_t hb_direction_from_string(const char *str, int len)
{
    if (!str || len == 0)
        return HB_DIRECTION_INVALID;

    switch (str[0]) {
    case 'l': case 'L': return HB_DIRECTION_LTR;
    case 'r': case 'R': return HB_DIRECTION_RTL;
    case 't': case 'T': return HB_DIRECTION_TTB;
    case 'b': case 'B': return HB_DIRECTION_BTT;
    default:            return HB_DIRECTION_INVALID;
    }
}

const char *hb_direction_to_string(hb_direction_t direction)
{
    switch (direction) {
    case HB_DIRECTION_LTR: return "ltr";
    case HB_DIRECTION_RTL: return "rtl";
    case HB_DIRECTION_TTB: return "ttb";
    case HB_DIRECTION_BTT: return "btt";
    default:               return "invalid";
    }
}

/* ========================================================================= */
/* Script                                                                    */
/* ========================================================================= */

hb_script_t hb_script_from_iso15924_tag(hb_tag_t tag)
{
    return (hb_script_t)tag;
}

hb_script_t hb_script_from_string(const char *str, int len)
{
    if (!str || len == 0)
        return HB_SCRIPT_UNKNOWN;

    if (len < 0)
        len = (int)strlen(str);

    if (len >= 4)
        return (hb_script_t)HB_TAG(str[0], str[1], str[2], str[3]);

    return HB_SCRIPT_UNKNOWN;
}

hb_tag_t hb_script_to_iso15924_tag(hb_script_t script)
{
    return (hb_tag_t)script;
}

hb_direction_t hb_script_get_horizontal_direction(hb_script_t script)
{
    switch ((uint32_t)script) {
    case HB_SCRIPT_ARABIC:
    case HB_SCRIPT_HEBREW:
    case HB_SCRIPT_SYRIAC:
    case HB_SCRIPT_THAANA:
        return HB_DIRECTION_RTL;
    default:
        return HB_DIRECTION_LTR;
    }
}

/* ========================================================================= */
/* Language                                                                  */
/* ========================================================================= */

hb_language_t hb_language_from_string(const char *str, int len)
{
    int i;

    if (!str || len == 0)
        return (hb_language_t)&g_lang_en;

    if (len < 0)
        len = (int)strlen(str);

    /* Check existing languages */
    if (len == 2 && str[0] == 'e' && str[1] == 'n')
        return (hb_language_t)&g_lang_en;

    for (i = 0; i < g_lang_count; i++) {
        if (strncmp(g_lang_custom[i].s, str, (size_t)len) == 0 &&
            g_lang_custom[i].s[len] == '\0')
            return (hb_language_t)&g_lang_custom[i];
    }

    /* Create new (up to limit) */
    if (g_lang_count < 16) {
        char *s = (char *)malloc((size_t)len + 1);
        if (s) {
            memcpy(s, str, (size_t)len);
            s[len] = '\0';
            g_lang_custom[g_lang_count].s = s;
            return (hb_language_t)&g_lang_custom[g_lang_count++];
        }
    }

    return (hb_language_t)&g_lang_en;
}

const char *hb_language_to_string(hb_language_t language)
{
    if (!language)
        return NULL;

    return language->s;
}

hb_language_t hb_language_get_default(void)
{
    return (hb_language_t)&g_lang_en;
}

hb_bool_t hb_language_matches(hb_language_t language,
                                hb_language_t specific)
{
    if (!language || !specific)
        return 0;

    return strcmp(language->s, specific->s) == 0;
}

/* ========================================================================= */
/* Feature                                                                   */
/* ========================================================================= */

hb_bool_t hb_feature_from_string(const char *str, int len,
                                    hb_feature_t *feature)
{
    (void)str;
    (void)len;

    if (feature) {
        feature->tag   = HB_TAG_NONE;
        feature->value = 0;
        feature->start = HB_FEATURE_GLOBAL_START;
        feature->end   = HB_FEATURE_GLOBAL_END;
    }
    return 0;
}

void hb_feature_to_string(hb_feature_t *feature, char *buf,
                             unsigned int size)
{
    (void)feature;
    if (buf && size > 0)
        buf[0] = '\0';
}

hb_bool_t hb_variation_from_string(const char *str, int len,
                                      hb_variation_t *variation)
{
    (void)str;
    (void)len;
    (void)variation;
    return 0;
}

void hb_variation_to_string(hb_variation_t *variation, char *buf,
                               unsigned int size)
{
    (void)variation;
    if (buf && size > 0)
        buf[0] = '\0';
}

/* ========================================================================= */
/* Blob                                                                      */
/* ========================================================================= */

hb_blob_t *hb_blob_create(const char *data, unsigned int length,
                             hb_memory_mode_t mode, void *user_data,
                             hb_destroy_func_t destroy)
{
    int i;

    for (i = 0; i < MAX_BLOBS; i++) {
        if (!g_blobs[i].in_use) {
            g_blobs[i].in_use    = 1;
            g_blobs[i].ref_count = 1;
            g_blobs[i].data      = data;
            g_blobs[i].length    = length;
            g_blobs[i].mode      = mode;
            g_blobs[i].user_data = user_data;
            g_blobs[i].destroy   = destroy;
            g_blobs[i].immutable = 0;
            return (hb_blob_t *)&g_blobs[i];
        }
    }
    return hb_blob_get_empty();
}

hb_blob_t *hb_blob_create_from_file(const char *file_name)
{
    (void)file_name;
    return hb_blob_get_empty();
}

hb_blob_t *hb_blob_create_from_file_or_fail(const char *file_name)
{
    (void)file_name;
    return NULL;
}

hb_blob_t *hb_blob_create_sub_blob(hb_blob_t *parent,
                                      unsigned int offset,
                                      unsigned int length)
{
    struct hb_blob_internal *p = (struct hb_blob_internal *)parent;

    if (!p || !p->in_use || offset >= p->length)
        return hb_blob_get_empty();

    if (offset + length > p->length)
        length = p->length - offset;

    return hb_blob_create(p->data + offset, length,
                            HB_MEMORY_MODE_READONLY, NULL, NULL);
}

hb_blob_t *hb_blob_copy_writable_or_fail(hb_blob_t *blob)
{
    (void)blob;
    return NULL;
}

static struct hb_blob_internal g_empty_blob = {
    1, 999999, NULL, 0, HB_MEMORY_MODE_READONLY, NULL, NULL, 1
};

hb_blob_t *hb_blob_get_empty(void)
{
    return (hb_blob_t *)&g_empty_blob;
}

hb_blob_t *hb_blob_reference(hb_blob_t *blob)
{
    struct hb_blob_internal *b = (struct hb_blob_internal *)blob;
    if (b && b != &g_empty_blob)
        b->ref_count++;
    return blob;
}

void hb_blob_destroy(hb_blob_t *blob)
{
    struct hb_blob_internal *b = (struct hb_blob_internal *)blob;

    if (!b || b == &g_empty_blob)
        return;

    if (--b->ref_count <= 0) {
        if (b->destroy && b->user_data)
            b->destroy(b->user_data);
        b->in_use = 0;
    }
}

void hb_blob_make_immutable(hb_blob_t *blob)
{
    struct hb_blob_internal *b = (struct hb_blob_internal *)blob;
    if (b) b->immutable = 1;
}

hb_bool_t hb_blob_is_immutable(hb_blob_t *blob)
{
    struct hb_blob_internal *b = (struct hb_blob_internal *)blob;
    return b ? b->immutable : 1;
}

unsigned int hb_blob_get_length(hb_blob_t *blob)
{
    struct hb_blob_internal *b = (struct hb_blob_internal *)blob;
    return b ? b->length : 0;
}

const char *hb_blob_get_data(hb_blob_t *blob, unsigned int *length)
{
    struct hb_blob_internal *b = (struct hb_blob_internal *)blob;
    if (!b) {
        if (length) *length = 0;
        return NULL;
    }
    if (length) *length = b->length;
    return b->data;
}

char *hb_blob_get_data_writable(hb_blob_t *blob, unsigned int *length)
{
    (void)blob;
    if (length) *length = 0;
    return NULL;
}

/* ========================================================================= */
/* Face                                                                      */
/* ========================================================================= */

hb_face_t *hb_face_create(hb_blob_t *blob, unsigned int index)
{
    int i;

    for (i = 0; i < MAX_FACES; i++) {
        if (!g_faces[i].in_use) {
            g_faces[i].in_use      = 1;
            g_faces[i].ref_count   = 1;
            g_faces[i].index       = index;
            g_faces[i].upem        = 1000;
            g_faces[i].glyph_count = 65535;
            g_faces[i].immutable   = 0;
            g_faces[i].blob        = (struct hb_blob_internal *)blob;
            return (hb_face_t *)&g_faces[i];
        }
    }
    return hb_face_get_empty();
}

hb_face_t *hb_face_create_for_tables(hb_reference_table_func_t func,
                                        void *user_data,
                                        hb_destroy_func_t destroy)
{
    (void)func;
    (void)user_data;
    (void)destroy;
    return hb_face_create(hb_blob_get_empty(), 0);
}

static struct hb_face_internal g_empty_face = {
    1, 999999, 0, 1000, 0, 1, NULL
};

hb_face_t *hb_face_get_empty(void)
{
    return (hb_face_t *)&g_empty_face;
}

hb_face_t *hb_face_reference(hb_face_t *face)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    if (f && f != &g_empty_face)
        f->ref_count++;
    return face;
}

void hb_face_destroy(hb_face_t *face)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    if (!f || f == &g_empty_face)
        return;
    if (--f->ref_count <= 0)
        f->in_use = 0;
}

void hb_face_make_immutable(hb_face_t *face)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    if (f) f->immutable = 1;
}

hb_bool_t hb_face_is_immutable(hb_face_t *face)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    return f ? f->immutable : 1;
}

hb_blob_t *hb_face_reference_table(hb_face_t *face, hb_tag_t tag)
{
    (void)face;
    (void)tag;
    return hb_blob_get_empty();
}

hb_blob_t *hb_face_reference_blob(hb_face_t *face)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    if (f && f->blob)
        return hb_blob_reference((hb_blob_t *)f->blob);
    return hb_blob_get_empty();
}

void hb_face_set_index(hb_face_t *face, unsigned int index)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    if (f && !f->immutable) f->index = index;
}

unsigned int hb_face_get_index(hb_face_t *face)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    return f ? f->index : 0;
}

void hb_face_set_upem(hb_face_t *face, unsigned int upem)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    if (f && !f->immutable) f->upem = upem;
}

unsigned int hb_face_get_upem(hb_face_t *face)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    return f ? f->upem : 1000;
}

void hb_face_set_glyph_count(hb_face_t *face, unsigned int glyph_count)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    if (f && !f->immutable) f->glyph_count = glyph_count;
}

unsigned int hb_face_get_glyph_count(hb_face_t *face)
{
    struct hb_face_internal *f = (struct hb_face_internal *)face;
    return f ? f->glyph_count : 0;
}

unsigned int hb_face_get_table_tags(hb_face_t *face,
                                       unsigned int start_offset,
                                       unsigned int *table_count,
                                       hb_tag_t *table_tags)
{
    (void)face;
    (void)start_offset;
    (void)table_tags;
    if (table_count) *table_count = 0;
    return 0;
}

void hb_face_collect_unicodes(hb_face_t *face, hb_set_t *out)
{
    (void)face;
    (void)out;
}

void hb_face_collect_nominal_glyph_mapping(hb_face_t *face,
                                              hb_map_t *mapping,
                                              hb_set_t *unicodes)
{
    (void)face;
    (void)mapping;
    (void)unicodes;
}

void hb_face_collect_variation_selectors(hb_face_t *face, hb_set_t *out)
{
    (void)face;
    (void)out;
}

void hb_face_collect_variation_unicodes(hb_face_t *face,
                                           hb_codepoint_t vs,
                                           hb_set_t *out)
{
    (void)face;
    (void)vs;
    (void)out;
}

hb_face_t *hb_face_builder_create(void)
{
    return hb_face_create(hb_blob_get_empty(), 0);
}

hb_bool_t hb_face_builder_add_table(hb_face_t *face, hb_tag_t tag,
                                       hb_blob_t *blob)
{
    (void)face;
    (void)tag;
    (void)blob;
    return 1;
}

void hb_face_builder_sort_tables(hb_face_t *face, const hb_tag_t *tags)
{
    (void)face;
    (void)tags;
}

/* ========================================================================= */
/* Font                                                                      */
/* ========================================================================= */

hb_font_t *hb_font_create(hb_face_t *face)
{
    int i;

    for (i = 0; i < MAX_FONTS; i++) {
        if (!g_fonts[i].in_use) {
            struct hb_font_internal *f = &g_fonts[i];
            memset(f, 0, sizeof(*f));
            f->in_use    = 1;
            f->ref_count = 1;
            f->face      = (struct hb_face_internal *)face;
            f->x_scale   = f->face ? (int)f->face->upem : 1000;
            f->y_scale   = f->x_scale;
            f->x_ppem    = 0;
            f->y_ppem    = 0;
            f->ptem      = 0.0f;
            return (hb_font_t *)f;
        }
    }
    return hb_font_get_empty();
}

hb_font_t *hb_font_create_sub_font(hb_font_t *parent)
{
    struct hb_font_internal *p = (struct hb_font_internal *)parent;
    hb_font_t *font = hb_font_create((hb_face_t *)p->face);
    struct hb_font_internal *f = (struct hb_font_internal *)font;

    if (f && p) {
        f->x_scale = p->x_scale;
        f->y_scale = p->y_scale;
        f->x_ppem  = p->x_ppem;
        f->y_ppem  = p->y_ppem;
        f->ptem    = p->ptem;
    }
    return font;
}

static struct hb_font_internal g_empty_font = {
    1, 999999, NULL, 1000, 1000, 0, 0, 0.0f, 1, NULL, 0
};

hb_font_t *hb_font_get_empty(void)
{
    return (hb_font_t *)&g_empty_font;
}

hb_font_t *hb_font_reference(hb_font_t *font)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    if (f && f != &g_empty_font)
        f->ref_count++;
    return font;
}

void hb_font_destroy(hb_font_t *font)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    if (!f || f == &g_empty_font)
        return;
    if (--f->ref_count <= 0)
        f->in_use = 0;
}

void hb_font_make_immutable(hb_font_t *font)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    if (f) f->immutable = 1;
}

hb_bool_t hb_font_is_immutable(hb_font_t *font)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    return f ? f->immutable : 1;
}

hb_face_t *hb_font_get_face(hb_font_t *font)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    return f ? (hb_face_t *)f->face : hb_face_get_empty();
}

void hb_font_set_funcs(hb_font_t *font, hb_font_funcs_t *klass,
                          void *font_data, hb_destroy_func_t destroy)
{
    (void)font; (void)klass; (void)font_data; (void)destroy;
}

void hb_font_set_funcs_data(hb_font_t *font, void *font_data,
                               hb_destroy_func_t destroy)
{
    (void)font; (void)font_data; (void)destroy;
}

void hb_font_set_scale(hb_font_t *font, int x_scale, int y_scale)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    if (f && !f->immutable) {
        f->x_scale = x_scale;
        f->y_scale = y_scale;
    }
}

void hb_font_get_scale(hb_font_t *font, int *x_scale, int *y_scale)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    if (x_scale) *x_scale = f ? f->x_scale : 0;
    if (y_scale) *y_scale = f ? f->y_scale : 0;
}

void hb_font_set_ppem(hb_font_t *font, unsigned int x_ppem,
                         unsigned int y_ppem)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    if (f && !f->immutable) {
        f->x_ppem = x_ppem;
        f->y_ppem = y_ppem;
    }
}

void hb_font_get_ppem(hb_font_t *font, unsigned int *x_ppem,
                         unsigned int *y_ppem)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    if (x_ppem) *x_ppem = f ? f->x_ppem : 0;
    if (y_ppem) *y_ppem = f ? f->y_ppem : 0;
}

void hb_font_set_ptem(hb_font_t *font, float ptem)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    if (f && !f->immutable) f->ptem = ptem;
}

float hb_font_get_ptem(hb_font_t *font)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    return f ? f->ptem : 0.0f;
}

void hb_font_set_synthetic_bold(hb_font_t *font, float x, float y,
                                   hb_bool_t in_place)
{
    (void)font; (void)x; (void)y; (void)in_place;
}

void hb_font_set_synthetic_slant(hb_font_t *font, float slant)
{
    (void)font; (void)slant;
}

hb_bool_t hb_font_get_h_extents(hb_font_t *font,
                                    hb_font_extents_t *extents)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;

    if (!extents)
        return 0;

    memset(extents, 0, sizeof(*extents));
    if (f) {
        extents->ascender  = f->y_scale * 800 / 1000;
        extents->descender = -(f->y_scale * 200 / 1000);
        extents->line_gap  = f->y_scale * 90 / 1000;
    }
    return 1;
}

hb_bool_t hb_font_get_v_extents(hb_font_t *font,
                                    hb_font_extents_t *extents)
{
    (void)font;
    if (extents) memset(extents, 0, sizeof(*extents));
    return 0;
}

hb_bool_t hb_font_get_nominal_glyph(hb_font_t *font,
                                        hb_codepoint_t unicode,
                                        hb_codepoint_t *glyph)
{
    (void)font;
    if (glyph) *glyph = unicode;
    return 1;
}

hb_bool_t hb_font_get_variation_glyph(hb_font_t *font,
                                          hb_codepoint_t unicode,
                                          hb_codepoint_t vs,
                                          hb_codepoint_t *glyph)
{
    (void)font; (void)vs;
    if (glyph) *glyph = unicode;
    return 0;
}

hb_bool_t hb_font_get_nominal_glyphs(hb_font_t *font,
                                         unsigned int count,
                                         const hb_codepoint_t *first_unicode,
                                         unsigned int unicode_stride,
                                         hb_codepoint_t *first_glyph,
                                         unsigned int glyph_stride)
{
    unsigned int i;
    (void)font;

    for (i = 0; i < count; i++) {
        *first_glyph = *first_unicode;
        first_unicode = (const hb_codepoint_t *)((const char *)first_unicode + unicode_stride);
        first_glyph = (hb_codepoint_t *)((char *)first_glyph + glyph_stride);
    }
    return 1;
}

hb_position_t hb_font_get_glyph_h_advance(hb_font_t *font,
                                              hb_codepoint_t glyph)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    (void)glyph;
    return f ? f->x_scale * 600 / 1000 : 600;
}

hb_position_t hb_font_get_glyph_v_advance(hb_font_t *font,
                                              hb_codepoint_t glyph)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    (void)glyph;
    return f ? -(f->y_scale) : -1000;
}

hb_bool_t hb_font_get_glyph_h_origin(hb_font_t *font, hb_codepoint_t glyph,
                                         hb_position_t *x, hb_position_t *y)
{
    (void)font; (void)glyph;
    if (x) *x = 0;
    if (y) *y = 0;
    return 1;
}

hb_bool_t hb_font_get_glyph_v_origin(hb_font_t *font, hb_codepoint_t glyph,
                                         hb_position_t *x, hb_position_t *y)
{
    (void)font; (void)glyph;
    if (x) *x = 0;
    if (y) *y = 0;
    return 0;
}

hb_position_t hb_font_get_glyph_h_kerning(hb_font_t *font,
                                              hb_codepoint_t left,
                                              hb_codepoint_t right)
{
    (void)font; (void)left; (void)right;
    return 0;
}

hb_bool_t hb_font_get_glyph_extents(hb_font_t *font, hb_codepoint_t glyph,
                                        hb_glyph_extents_t *extents)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    (void)glyph;

    if (!extents) return 0;
    extents->x_bearing = 0;
    extents->y_bearing = f ? f->y_scale * 800 / 1000 : 800;
    extents->width     = f ? f->x_scale * 600 / 1000 : 600;
    extents->height    = f ? -(f->y_scale) : -1000;
    return 1;
}

hb_bool_t hb_font_get_glyph_contour_point(hb_font_t *font,
                                              hb_codepoint_t glyph,
                                              unsigned int point_index,
                                              hb_position_t *x,
                                              hb_position_t *y)
{
    (void)font; (void)glyph; (void)point_index;
    if (x) *x = 0;
    if (y) *y = 0;
    return 0;
}

hb_bool_t hb_font_get_glyph_name(hb_font_t *font, hb_codepoint_t glyph,
                                     char *name, unsigned int size)
{
    (void)font;
    if (name && size > 0)
        snprintf(name, size, "gid%u", glyph);
    return 1;
}

hb_bool_t hb_font_get_glyph_from_name(hb_font_t *font, const char *name,
                                          int len, hb_codepoint_t *glyph)
{
    (void)font; (void)name; (void)len;
    if (glyph) *glyph = 0;
    return 0;
}

void hb_font_set_variations(hb_font_t *font,
                               const hb_variation_t *variations,
                               unsigned int variations_length)
{
    (void)font; (void)variations; (void)variations_length;
}

void hb_font_set_var_coords_design(hb_font_t *font, const float *coords,
                                      unsigned int coords_length)
{
    (void)font; (void)coords; (void)coords_length;
}

void hb_font_set_var_coords_normalized(hb_font_t *font, const int *coords,
                                          unsigned int coords_length)
{
    (void)font; (void)coords; (void)coords_length;
}

/* Font funcs stubs */
static struct { int dummy; } g_empty_ffuncs;

hb_font_funcs_t *hb_font_funcs_create(void) { return (hb_font_funcs_t *)&g_empty_ffuncs; }
hb_font_funcs_t *hb_font_funcs_get_empty(void) { return (hb_font_funcs_t *)&g_empty_ffuncs; }
hb_font_funcs_t *hb_font_funcs_reference(hb_font_funcs_t *ff) { return ff; }
void hb_font_funcs_destroy(hb_font_funcs_t *ff) { (void)ff; }
void hb_font_funcs_make_immutable(hb_font_funcs_t *ff) { (void)ff; }
hb_bool_t hb_font_funcs_is_immutable(hb_font_funcs_t *ff) { (void)ff; return 1; }

/* ========================================================================= */
/* Buffer                                                                    */
/* ========================================================================= */

hb_buffer_t *hb_buffer_create(void)
{
    int i;

    for (i = 0; i < MAX_BUFFERS; i++) {
        if (!g_buffers[i].in_use) {
            struct hb_buffer_internal *b = &g_buffers[i];
            memset(b, 0, sizeof(*b));
            b->in_use        = 1;
            b->ref_count     = 1;
            b->direction     = HB_DIRECTION_LTR;
            b->script        = HB_SCRIPT_COMMON;
            b->language      = hb_language_get_default();
            b->cluster_level = HB_BUFFER_CLUSTER_LEVEL_DEFAULT;
            b->content_type  = HB_BUFFER_CONTENT_TYPE_INVALID;
            return (hb_buffer_t *)b;
        }
    }
    return hb_buffer_get_empty();
}

hb_buffer_t *hb_buffer_create_similar(const hb_buffer_t *src)
{
    (void)src;
    return hb_buffer_create();
}

static struct hb_buffer_internal g_empty_buffer = {
    1, 999999, {{0}}, {{0}}, 0, HB_DIRECTION_LTR, HB_SCRIPT_COMMON,
    NULL, HB_BUFFER_FLAG_DEFAULT, HB_BUFFER_CLUSTER_LEVEL_DEFAULT,
    HB_BUFFER_CONTENT_TYPE_INVALID, 0
};

hb_buffer_t *hb_buffer_get_empty(void)
{
    return (hb_buffer_t *)&g_empty_buffer;
}

hb_buffer_t *hb_buffer_reference(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (b && b != &g_empty_buffer)
        b->ref_count++;
    return buffer;
}

void hb_buffer_destroy(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (!b || b == &g_empty_buffer)
        return;
    if (--b->ref_count <= 0)
        b->in_use = 0;
}

void hb_buffer_reset(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (!b || b == &g_empty_buffer) return;
    b->length       = 0;
    b->direction    = HB_DIRECTION_LTR;
    b->script       = HB_SCRIPT_COMMON;
    b->language     = hb_language_get_default();
    b->content_type = HB_BUFFER_CONTENT_TYPE_INVALID;
    b->has_positions = 0;
}

void hb_buffer_clear_contents(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (!b || b == &g_empty_buffer) return;
    b->length        = 0;
    b->content_type  = HB_BUFFER_CONTENT_TYPE_INVALID;
    b->has_positions = 0;
}

hb_bool_t hb_buffer_pre_allocate(hb_buffer_t *buffer, unsigned int size)
{
    (void)buffer;
    return size <= MAX_BUFFER_GLYPHS;
}

hb_bool_t hb_buffer_allocation_successful(hb_buffer_t *buffer)
{
    (void)buffer;
    return 1;
}

void hb_buffer_add(hb_buffer_t *buffer, hb_codepoint_t codepoint,
                      unsigned int cluster)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (!b || b == &g_empty_buffer || b->length >= MAX_BUFFER_GLYPHS)
        return;

    unsigned int idx = b->length++;
    b->infos[idx].codepoint = codepoint;
    b->infos[idx].cluster   = cluster;
    b->infos[idx].mask      = 0;
    b->content_type = HB_BUFFER_CONTENT_TYPE_UNICODE;
}

void hb_buffer_add_utf8(hb_buffer_t *buffer, const char *text,
                           int text_length, unsigned int item_offset,
                           int item_length)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    unsigned int i, len;
    const unsigned char *p;

    if (!b || b == &g_empty_buffer || !text)
        return;

    if (text_length < 0)
        len = (unsigned int)strlen(text);
    else
        len = (unsigned int)text_length;

    if (item_length < 0)
        item_length = (int)(len - item_offset);

    p = (const unsigned char *)text + item_offset;
    for (i = 0; i < (unsigned int)item_length && b->length < MAX_BUFFER_GLYPHS; i++) {
        unsigned int cp;
        unsigned int cluster_idx = item_offset + i;

        if (p[i] < 0x80) {
            cp = p[i];
        } else if ((p[i] & 0xE0) == 0xC0 && i + 1 < (unsigned int)item_length) {
            cp = ((unsigned int)(p[i] & 0x1F) << 6) | (p[i + 1] & 0x3F);
            i++;
        } else if ((p[i] & 0xF0) == 0xE0 && i + 2 < (unsigned int)item_length) {
            cp = ((unsigned int)(p[i] & 0x0F) << 12) |
                 ((unsigned int)(p[i + 1] & 0x3F) << 6) |
                 (p[i + 2] & 0x3F);
            i += 2;
        } else if ((p[i] & 0xF8) == 0xF0 && i + 3 < (unsigned int)item_length) {
            cp = ((unsigned int)(p[i] & 0x07) << 18) |
                 ((unsigned int)(p[i + 1] & 0x3F) << 12) |
                 ((unsigned int)(p[i + 2] & 0x3F) << 6) |
                 (p[i + 3] & 0x3F);
            i += 3;
        } else {
            cp = 0xFFFD;  /* Replacement character */
        }

        hb_buffer_add(buffer, cp, cluster_idx);
    }
}

void hb_buffer_add_utf16(hb_buffer_t *buffer, const uint16_t *text,
                            int text_length, unsigned int item_offset,
                            int item_length)
{
    unsigned int i, len;

    if (!text) return;

    if (text_length < 0) {
        const uint16_t *p = text;
        while (*p) p++;
        len = (unsigned int)(p - text);
    } else {
        len = (unsigned int)text_length;
    }

    if (item_length < 0)
        item_length = (int)(len - item_offset);

    for (i = 0; i < (unsigned int)item_length; i++) {
        uint32_t cp = text[item_offset + i];
        if (cp >= 0xD800 && cp <= 0xDBFF && i + 1 < (unsigned int)item_length) {
            uint32_t lo = text[item_offset + i + 1];
            if (lo >= 0xDC00 && lo <= 0xDFFF) {
                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                i++;
            }
        }
        hb_buffer_add(buffer, cp, item_offset + i);
    }
}

void hb_buffer_add_utf32(hb_buffer_t *buffer, const uint32_t *text,
                            int text_length, unsigned int item_offset,
                            int item_length)
{
    unsigned int i;
    if (!text) return;
    if (text_length < 0) {
        const uint32_t *p = text;
        while (*p) p++;
        text_length = (int)(p - text);
    }
    if (item_length < 0)
        item_length = text_length - (int)item_offset;

    for (i = 0; i < (unsigned int)item_length; i++)
        hb_buffer_add(buffer, text[item_offset + i], item_offset + i);
}

void hb_buffer_add_latin1(hb_buffer_t *buffer, const uint8_t *text,
                             int text_length, unsigned int item_offset,
                             int item_length)
{
    unsigned int i;
    if (!text) return;
    if (text_length < 0) text_length = (int)strlen((const char *)text);
    if (item_length < 0) item_length = text_length - (int)item_offset;

    for (i = 0; i < (unsigned int)item_length; i++)
        hb_buffer_add(buffer, text[item_offset + i], item_offset + i);
}

void hb_buffer_add_codepoints(hb_buffer_t *buffer,
                                 const hb_codepoint_t *text,
                                 int text_length, unsigned int item_offset,
                                 int item_length)
{
    hb_buffer_add_utf32(buffer, text, text_length, item_offset, item_length);
}

void hb_buffer_append(hb_buffer_t *buffer, const hb_buffer_t *source,
                         unsigned int start, unsigned int end)
{
    (void)buffer; (void)source; (void)start; (void)end;
}

void hb_buffer_set_content_type(hb_buffer_t *buffer,
                                   hb_buffer_content_type_t ct)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (b) b->content_type = ct;
}

hb_buffer_content_type_t hb_buffer_get_content_type(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    return b ? b->content_type : HB_BUFFER_CONTENT_TYPE_INVALID;
}

void hb_buffer_set_unicode_funcs(hb_buffer_t *buffer,
                                    hb_unicode_funcs_t *uf)
{
    (void)buffer; (void)uf;
}

hb_unicode_funcs_t *hb_buffer_get_unicode_funcs(hb_buffer_t *buffer)
{
    (void)buffer;
    return hb_unicode_funcs_get_default();
}

void hb_buffer_set_direction(hb_buffer_t *buffer, hb_direction_t dir)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (b) b->direction = dir;
}

hb_direction_t hb_buffer_get_direction(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    return b ? b->direction : HB_DIRECTION_INVALID;
}

void hb_buffer_set_script(hb_buffer_t *buffer, hb_script_t script)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (b) b->script = script;
}

hb_script_t hb_buffer_get_script(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    return b ? b->script : HB_SCRIPT_UNKNOWN;
}

void hb_buffer_set_language(hb_buffer_t *buffer, hb_language_t lang)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (b) b->language = lang;
}

hb_language_t hb_buffer_get_language(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    return b ? b->language : HB_LANGUAGE_INVALID;
}

void hb_buffer_set_segment_properties(hb_buffer_t *buffer, const void *props)
{
    (void)buffer; (void)props;
}

void hb_buffer_get_segment_properties(hb_buffer_t *buffer, void *props)
{
    (void)buffer; (void)props;
}

void hb_buffer_guess_segment_properties(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (!b) return;
    if (b->direction == HB_DIRECTION_INVALID)
        b->direction = HB_DIRECTION_LTR;
    if (b->script == HB_SCRIPT_UNKNOWN || (uint32_t)b->script == 0)
        b->script = HB_SCRIPT_LATIN;
    if (!b->language)
        b->language = hb_language_get_default();
}

void hb_buffer_set_flags(hb_buffer_t *buffer, hb_buffer_flags_t flags)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (b) b->flags = flags;
}

hb_buffer_flags_t hb_buffer_get_flags(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    return b ? b->flags : HB_BUFFER_FLAG_DEFAULT;
}

void hb_buffer_set_cluster_level(hb_buffer_t *buffer,
                                    hb_buffer_cluster_level_t cl)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (b) b->cluster_level = cl;
}

hb_buffer_cluster_level_t hb_buffer_get_cluster_level(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    return b ? b->cluster_level : HB_BUFFER_CLUSTER_LEVEL_DEFAULT;
}

void hb_buffer_set_length(hb_buffer_t *buffer, unsigned int length)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (b && length <= MAX_BUFFER_GLYPHS)
        b->length = length;
}

unsigned int hb_buffer_get_length(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    return b ? b->length : 0;
}

hb_glyph_info_t *hb_buffer_get_glyph_infos(hb_buffer_t *buffer,
                                               unsigned int *length)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (!b) {
        if (length) *length = 0;
        return NULL;
    }
    if (length) *length = b->length;
    return b->infos;
}

hb_glyph_position_t *hb_buffer_get_glyph_positions(hb_buffer_t *buffer,
                                                       unsigned int *length)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    if (!b) {
        if (length) *length = 0;
        return NULL;
    }
    if (length) *length = b->length;
    return b->positions;
}

hb_bool_t hb_buffer_has_positions(hb_buffer_t *buffer)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    return b ? b->has_positions : 0;
}

void hb_buffer_normalize_glyphs(hb_buffer_t *buffer) { (void)buffer; }
void hb_buffer_reverse(hb_buffer_t *buffer) { (void)buffer; }
void hb_buffer_reverse_range(hb_buffer_t *buffer, unsigned int start,
                                unsigned int end)
{
    (void)buffer; (void)start; (void)end;
}
void hb_buffer_reverse_clusters(hb_buffer_t *buffer) { (void)buffer; }

unsigned int hb_buffer_serialize_glyphs(hb_buffer_t *buffer,
                                           unsigned int start,
                                           unsigned int end, char *buf,
                                           unsigned int buf_size,
                                           unsigned int *buf_consumed,
                                           hb_font_t *font,
                                           hb_buffer_serialize_format_t format,
                                           hb_buffer_serialize_flags_t flags)
{
    (void)buffer; (void)start; (void)end; (void)buf; (void)buf_size;
    (void)font; (void)format; (void)flags;
    if (buf_consumed) *buf_consumed = 0;
    return 0;
}

hb_bool_t hb_buffer_deserialize_glyphs(hb_buffer_t *buffer, const char *buf,
                                           int buf_len, const char **end_ptr,
                                           hb_font_t *font,
                                           hb_buffer_serialize_format_t format)
{
    (void)buffer; (void)buf; (void)buf_len; (void)font; (void)format;
    if (end_ptr) *end_ptr = buf;
    return 0;
}

hb_buffer_diff_flags_t hb_buffer_diff(hb_buffer_t *buffer,
                                         hb_buffer_t *reference,
                                         hb_codepoint_t dc,
                                         unsigned int position_fuzz)
{
    (void)buffer; (void)reference; (void)dc; (void)position_fuzz;
    return HB_BUFFER_DIFF_FLAG_EQUAL;
}

void hb_buffer_set_message_func(hb_buffer_t *buffer,
                                   hb_buffer_message_func_t func,
                                   void *user_data,
                                   hb_destroy_func_t destroy)
{
    (void)buffer; (void)func; (void)user_data; (void)destroy;
}

/* ========================================================================= */
/* Shaping                                                                   */
/* ========================================================================= */

void hb_shape(hb_font_t *font, hb_buffer_t *buffer,
                const hb_feature_t *features, unsigned int num_features)
{
    hb_shape_full(font, buffer, features, num_features, NULL);
}

hb_bool_t hb_shape_full(hb_font_t *font, hb_buffer_t *buffer,
                           const hb_feature_t *features,
                           unsigned int num_features,
                           const char * const *shaper_list)
{
    struct hb_buffer_internal *b = (struct hb_buffer_internal *)buffer;
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    unsigned int i;

    (void)features;
    (void)num_features;
    (void)shaper_list;

    if (!b || !f || b->length == 0)
        return 0;

    /* Simple 1:1 shaping: each codepoint maps to a glyph with the
     * same index, and receives a default advance. */
    int advance = f->x_scale * 600 / 1000;
    if (advance <= 0) advance = 600;

    for (i = 0; i < b->length; i++) {
        /* Codepoint stays as glyph index (identity mapping) */
        b->positions[i].x_advance = advance;
        b->positions[i].y_advance = 0;
        b->positions[i].x_offset  = 0;
        b->positions[i].y_offset  = 0;
    }

    b->content_type  = HB_BUFFER_CONTENT_TYPE_GLYPHS;
    b->has_positions = 1;

    return 1;
}

hb_bool_t hb_shape_justify(hb_font_t *font, hb_buffer_t *buffer,
                              const hb_feature_t *features,
                              unsigned int num_features,
                              const char * const *shaper_list,
                              float min_target_advance,
                              float max_target_advance,
                              float *advance, hb_tag_t *var_tag,
                              float *var_value)
{
    (void)min_target_advance;
    (void)max_target_advance;
    (void)advance;
    (void)var_tag;
    (void)var_value;
    return hb_shape_full(font, buffer, features, num_features, shaper_list);
}

const char **hb_shape_list_shapers(void)
{
    return g_shapers;
}

/* Shape plan stubs */
hb_shape_plan_t *hb_shape_plan_create(hb_face_t *f, const void *p,
                                         const hb_feature_t *feat,
                                         unsigned int nf,
                                         const char * const *sl)
{
    (void)f; (void)p; (void)feat; (void)nf; (void)sl;
    return hb_shape_plan_get_empty();
}

hb_shape_plan_t *hb_shape_plan_create_cached(hb_face_t *f, const void *p,
                                                const hb_feature_t *feat,
                                                unsigned int nf,
                                                const char * const *sl)
{
    (void)f; (void)p; (void)feat; (void)nf; (void)sl;
    return hb_shape_plan_get_empty();
}

hb_shape_plan_t *hb_shape_plan_create2(hb_face_t *f, const void *p,
                                          const hb_feature_t *feat,
                                          unsigned int nf, const int *co,
                                          unsigned int nc,
                                          const char * const *sl)
{
    (void)f; (void)p; (void)feat; (void)nf; (void)co; (void)nc; (void)sl;
    return hb_shape_plan_get_empty();
}

hb_shape_plan_t *hb_shape_plan_create_cached2(hb_face_t *f, const void *p,
                                                 const hb_feature_t *feat,
                                                 unsigned int nf,
                                                 const int *co, unsigned int nc,
                                                 const char * const *sl)
{
    (void)f; (void)p; (void)feat; (void)nf; (void)co; (void)nc; (void)sl;
    return hb_shape_plan_get_empty();
}

static int g_empty_plan;
hb_shape_plan_t *hb_shape_plan_get_empty(void)
{
    return (hb_shape_plan_t *)&g_empty_plan;
}

hb_shape_plan_t *hb_shape_plan_reference(hb_shape_plan_t *sp) { return sp; }
void hb_shape_plan_destroy(hb_shape_plan_t *sp) { (void)sp; }

hb_bool_t hb_shape_plan_execute(hb_shape_plan_t *sp, hb_font_t *font,
                                   hb_buffer_t *buffer,
                                   const hb_feature_t *features,
                                   unsigned int num_features)
{
    (void)sp;
    return hb_shape_full(font, buffer, features, num_features, NULL);
}

const char *hb_shape_plan_get_shaper(hb_shape_plan_t *sp)
{
    (void)sp;
    return "ot";
}

/* ========================================================================= */
/* Unicode funcs                                                             */
/* ========================================================================= */

hb_unicode_funcs_t *hb_unicode_funcs_get_default(void)
{
    return (hb_unicode_funcs_t *)&g_unicode_funcs_default;
}

hb_unicode_funcs_t *hb_unicode_funcs_create(hb_unicode_funcs_t *parent)
{
    (void)parent;
    return hb_unicode_funcs_get_default();
}

hb_unicode_funcs_t *hb_unicode_funcs_get_empty(void)
{
    return hb_unicode_funcs_get_default();
}

hb_unicode_funcs_t *hb_unicode_funcs_reference(hb_unicode_funcs_t *uf)
{
    return uf;
}

void hb_unicode_funcs_destroy(hb_unicode_funcs_t *uf) { (void)uf; }
void hb_unicode_funcs_make_immutable(hb_unicode_funcs_t *uf) { (void)uf; }
hb_bool_t hb_unicode_funcs_is_immutable(hb_unicode_funcs_t *uf) { (void)uf; return 1; }

hb_unicode_combining_class_t hb_unicode_combining_class(
    hb_unicode_funcs_t *uf, hb_codepoint_t unicode)
{
    (void)uf; (void)unicode;
    return HB_UNICODE_COMBINING_CLASS_NOT_REORDERED;
}

hb_unicode_general_category_t hb_unicode_general_category(
    hb_unicode_funcs_t *uf, hb_codepoint_t unicode)
{
    (void)uf;
    if (unicode >= 'a' && unicode <= 'z')
        return HB_UNICODE_GENERAL_CATEGORY_LOWERCASE_LETTER;
    if (unicode >= 'A' && unicode <= 'Z')
        return HB_UNICODE_GENERAL_CATEGORY_UPPERCASE_LETTER;
    if (unicode >= '0' && unicode <= '9')
        return HB_UNICODE_GENERAL_CATEGORY_DECIMAL_NUMBER;
    if (unicode == ' ')
        return HB_UNICODE_GENERAL_CATEGORY_SPACE_SEPARATOR;
    return HB_UNICODE_GENERAL_CATEGORY_OTHER_LETTER;
}

hb_codepoint_t hb_unicode_mirroring(hb_unicode_funcs_t *uf,
                                        hb_codepoint_t unicode)
{
    (void)uf;
    /* Common mirroring pairs */
    switch (unicode) {
    case '(': return ')';
    case ')': return '(';
    case '[': return ']';
    case ']': return '[';
    case '{': return '}';
    case '}': return '{';
    case '<': return '>';
    case '>': return '<';
    default:  return unicode;
    }
}

hb_script_t hb_unicode_script(hb_unicode_funcs_t *uf,
                                 hb_codepoint_t unicode)
{
    (void)uf;
    if (unicode < 0x0080) return HB_SCRIPT_LATIN;
    if (unicode >= 0x0400 && unicode <= 0x04FF) return HB_SCRIPT_CYRILLIC;
    if (unicode >= 0x0600 && unicode <= 0x06FF) return HB_SCRIPT_ARABIC;
    if (unicode >= 0x4E00 && unicode <= 0x9FFF) return HB_SCRIPT_HAN;
    return HB_SCRIPT_COMMON;
}

/* ========================================================================= */
/* Set                                                                       */
/* ========================================================================= */

hb_set_t *hb_set_create(void)
{
    int i;
    for (i = 0; i < MAX_SETS; i++) {
        if (!g_sets[i].in_use) {
            memset(&g_sets[i], 0, sizeof(g_sets[i]));
            g_sets[i].in_use = 1;
            g_sets[i].ref_count = 1;
            return (hb_set_t *)&g_sets[i];
        }
    }
    return hb_set_get_empty();
}

static struct hb_set_internal g_empty_set = { 1, 999999, {0}, 0 };

hb_set_t *hb_set_get_empty(void) { return (hb_set_t *)&g_empty_set; }
hb_set_t *hb_set_reference(hb_set_t *s) {
    struct hb_set_internal *si = (struct hb_set_internal *)s;
    if (si && si != &g_empty_set) si->ref_count++;
    return s;
}
void hb_set_destroy(hb_set_t *s) {
    struct hb_set_internal *si = (struct hb_set_internal *)s;
    if (!si || si == &g_empty_set) return;
    if (--si->ref_count <= 0) si->in_use = 0;
}

hb_bool_t hb_set_allocation_successful(hb_set_t *s) { (void)s; return 1; }
hb_set_t *hb_set_copy(const hb_set_t *s) { (void)s; return hb_set_create(); }
void hb_set_clear(hb_set_t *s) {
    struct hb_set_internal *si = (struct hb_set_internal *)s;
    if (si) { memset(si->bits, 0, sizeof(si->bits)); si->population = 0; }
}
hb_bool_t hb_set_is_empty(const hb_set_t *s) {
    const struct hb_set_internal *si = (const struct hb_set_internal *)s;
    return !si || si->population == 0;
}
hb_bool_t hb_set_has(const hb_set_t *s, hb_codepoint_t cp) {
    const struct hb_set_internal *si = (const struct hb_set_internal *)s;
    if (!si || cp >= 1024) return 0;
    return (si->bits[cp / 32] >> (cp % 32)) & 1;
}
void hb_set_add(hb_set_t *s, hb_codepoint_t cp) {
    struct hb_set_internal *si = (struct hb_set_internal *)s;
    if (!si || si == &g_empty_set || cp >= 1024) return;
    if (!((si->bits[cp / 32] >> (cp % 32)) & 1)) {
        si->bits[cp / 32] |= (1u << (cp % 32));
        si->population++;
    }
}
void hb_set_add_range(hb_set_t *s, hb_codepoint_t first, hb_codepoint_t last) {
    hb_codepoint_t c;
    for (c = first; c <= last && c < 1024; c++) hb_set_add(s, c);
}
void hb_set_add_sorted_array(hb_set_t *s, const hb_codepoint_t *arr, unsigned int n) {
    unsigned int i;
    for (i = 0; i < n; i++) hb_set_add(s, arr[i]);
}
void hb_set_del(hb_set_t *s, hb_codepoint_t cp) {
    struct hb_set_internal *si = (struct hb_set_internal *)s;
    if (!si || cp >= 1024) return;
    if ((si->bits[cp / 32] >> (cp % 32)) & 1) {
        si->bits[cp / 32] &= ~(1u << (cp % 32));
        si->population--;
    }
}
void hb_set_del_range(hb_set_t *s, hb_codepoint_t first, hb_codepoint_t last) {
    hb_codepoint_t c;
    for (c = first; c <= last && c < 1024; c++) hb_set_del(s, c);
}
hb_bool_t hb_set_is_equal(const hb_set_t *a, const hb_set_t *b) {
    (void)a; (void)b; return 0;
}
hb_bool_t hb_set_is_subset(const hb_set_t *s, const hb_set_t *l) {
    (void)s; (void)l; return 0;
}
unsigned int hb_set_get_population(const hb_set_t *s) {
    const struct hb_set_internal *si = (const struct hb_set_internal *)s;
    return si ? si->population : 0;
}
hb_codepoint_t hb_set_get_min(const hb_set_t *s) { (void)s; return HB_SET_VALUE_INVALID; }
hb_codepoint_t hb_set_get_max(const hb_set_t *s) { (void)s; return HB_SET_VALUE_INVALID; }
hb_bool_t hb_set_next(const hb_set_t *s, hb_codepoint_t *cp) { (void)s; (void)cp; return 0; }
hb_bool_t hb_set_previous(const hb_set_t *s, hb_codepoint_t *cp) { (void)s; (void)cp; return 0; }
hb_bool_t hb_set_next_range(const hb_set_t *s, hb_codepoint_t *f, hb_codepoint_t *l) {
    (void)s; (void)f; (void)l; return 0;
}
hb_bool_t hb_set_previous_range(const hb_set_t *s, hb_codepoint_t *f, hb_codepoint_t *l) {
    (void)s; (void)f; (void)l; return 0;
}
unsigned int hb_set_next_many(const hb_set_t *s, hb_codepoint_t cp,
                                 hb_codepoint_t *out, unsigned int size) {
    (void)s; (void)cp; (void)out; (void)size; return 0;
}
void hb_set_union(hb_set_t *s, const hb_set_t *o) { (void)s; (void)o; }
void hb_set_intersect(hb_set_t *s, const hb_set_t *o) { (void)s; (void)o; }
void hb_set_subtract(hb_set_t *s, const hb_set_t *o) { (void)s; (void)o; }
void hb_set_symmetric_difference(hb_set_t *s, const hb_set_t *o) { (void)s; (void)o; }
void hb_set_invert(hb_set_t *s) { (void)s; }

/* ========================================================================= */
/* Map                                                                       */
/* ========================================================================= */

hb_map_t *hb_map_create(void) {
    int i;
    for (i = 0; i < MAX_MAPS; i++) {
        if (!g_maps[i].in_use) {
            memset(&g_maps[i], 0, sizeof(g_maps[i]));
            g_maps[i].in_use = 1;
            g_maps[i].ref_count = 1;
            return (hb_map_t *)&g_maps[i];
        }
    }
    return hb_map_get_empty();
}

static struct hb_map_internal g_empty_map = { 1, 999999, {0}, {0}, 0 };

hb_map_t *hb_map_get_empty(void) { return (hb_map_t *)&g_empty_map; }
hb_map_t *hb_map_reference(hb_map_t *m) {
    struct hb_map_internal *mi = (struct hb_map_internal *)m;
    if (mi && mi != &g_empty_map) mi->ref_count++;
    return m;
}
void hb_map_destroy(hb_map_t *m) {
    struct hb_map_internal *mi = (struct hb_map_internal *)m;
    if (!mi || mi == &g_empty_map) return;
    if (--mi->ref_count <= 0) mi->in_use = 0;
}
hb_bool_t hb_map_allocation_successful(hb_map_t *m) { (void)m; return 1; }
hb_map_t *hb_map_copy(const hb_map_t *m) { (void)m; return hb_map_create(); }
void hb_map_clear(hb_map_t *m) {
    struct hb_map_internal *mi = (struct hb_map_internal *)m;
    if (mi) mi->count = 0;
}
hb_bool_t hb_map_is_empty(const hb_map_t *m) {
    const struct hb_map_internal *mi = (const struct hb_map_internal *)m;
    return !mi || mi->count == 0;
}
unsigned int hb_map_get_population(const hb_map_t *m) {
    const struct hb_map_internal *mi = (const struct hb_map_internal *)m;
    return mi ? mi->count : 0;
}
hb_bool_t hb_map_is_equal(const hb_map_t *a, const hb_map_t *b) { (void)a; (void)b; return 0; }
unsigned int hb_map_hash(const hb_map_t *m) { (void)m; return 0; }
void hb_map_set(hb_map_t *m, hb_codepoint_t key, hb_codepoint_t value) {
    struct hb_map_internal *mi = (struct hb_map_internal *)m;
    unsigned int i;
    if (!mi || mi == &g_empty_map) return;
    for (i = 0; i < mi->count; i++) {
        if (mi->keys[i] == key) { mi->values[i] = value; return; }
    }
    if (mi->count < 256) {
        mi->keys[mi->count] = key;
        mi->values[mi->count] = value;
        mi->count++;
    }
}
hb_codepoint_t hb_map_get(const hb_map_t *m, hb_codepoint_t key) {
    const struct hb_map_internal *mi = (const struct hb_map_internal *)m;
    unsigned int i;
    if (!mi) return HB_MAP_VALUE_INVALID;
    for (i = 0; i < mi->count; i++)
        if (mi->keys[i] == key) return mi->values[i];
    return HB_MAP_VALUE_INVALID;
}
void hb_map_del(hb_map_t *m, hb_codepoint_t key) {
    struct hb_map_internal *mi = (struct hb_map_internal *)m;
    unsigned int i;
    if (!mi) return;
    for (i = 0; i < mi->count; i++) {
        if (mi->keys[i] == key) {
            mi->keys[i] = mi->keys[mi->count - 1];
            mi->values[i] = mi->values[mi->count - 1];
            mi->count--;
            return;
        }
    }
}
hb_bool_t hb_map_has(const hb_map_t *m, hb_codepoint_t key) {
    return hb_map_get(m, key) != HB_MAP_VALUE_INVALID;
}
void hb_map_update(hb_map_t *m, const hb_map_t *o) { (void)m; (void)o; }
hb_bool_t hb_map_next(const hb_map_t *m, int *idx, hb_codepoint_t *key,
                          hb_codepoint_t *value) {
    const struct hb_map_internal *mi = (const struct hb_map_internal *)m;
    if (!mi || *idx >= (int)mi->count) return 0;
    if (key) *key = mi->keys[*idx];
    if (value) *value = mi->values[*idx];
    (*idx)++;
    return 1;
}
void hb_map_keys(const hb_map_t *m, hb_set_t *keys) { (void)m; (void)keys; }
void hb_map_values(const hb_map_t *m, hb_set_t *values) { (void)m; (void)values; }

/* ========================================================================= */
/* FreeType integration                                                      */
/* ========================================================================= */

hb_face_t *hb_ft_face_create(FT_Face ft_face, hb_destroy_func_t destroy)
{
    (void)destroy;
    hb_face_t *face = hb_face_create(hb_blob_get_empty(), 0);
    if (ft_face) {
        struct hb_face_internal *f = (struct hb_face_internal *)face;
        if (f) {
            f->upem = ft_face->units_per_EM;
            f->glyph_count = (unsigned int)ft_face->num_glyphs;
        }
    }
    return face;
}

hb_face_t *hb_ft_face_create_cached(FT_Face ft_face)
{
    return hb_ft_face_create(ft_face, NULL);
}

hb_face_t *hb_ft_face_create_referenced(FT_Face ft_face)
{
    return hb_ft_face_create(ft_face, NULL);
}

hb_font_t *hb_ft_font_create(FT_Face ft_face, hb_destroy_func_t destroy)
{
    hb_face_t *face = hb_ft_face_create(ft_face, NULL);
    hb_font_t *font = hb_font_create(face);
    struct hb_font_internal *f = (struct hb_font_internal *)font;

    (void)destroy;

    if (f && ft_face) {
        f->ft_face = ft_face;
        if (ft_face->size) {
            f->x_scale = (int)ft_face->size->metrics.x_ppem * 64;
            f->y_scale = (int)ft_face->size->metrics.y_ppem * 64;
            f->x_ppem  = ft_face->size->metrics.x_ppem;
            f->y_ppem  = ft_face->size->metrics.y_ppem;
        }
    }

    /* Face ref is now owned by font */
    hb_face_destroy(face);
    return font;
}

hb_font_t *hb_ft_font_create_referenced(FT_Face ft_face)
{
    return hb_ft_font_create(ft_face, NULL);
}

FT_Face hb_ft_font_get_face(hb_font_t *font)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    return f ? f->ft_face : NULL;
}

FT_Face hb_ft_font_lock_face(hb_font_t *font)
{
    return hb_ft_font_get_face(font);
}

void hb_ft_font_unlock_face(hb_font_t *font)
{
    (void)font;
}

void hb_ft_font_set_load_flags(hb_font_t *font, int load_flags)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    if (f) f->ft_load_flags = load_flags;
}

int hb_ft_font_get_load_flags(hb_font_t *font)
{
    struct hb_font_internal *f = (struct hb_font_internal *)font;
    return f ? f->ft_load_flags : 0;
}

void hb_ft_font_changed(hb_font_t *font) { (void)font; }
hb_bool_t hb_ft_hb_font_changed(hb_font_t *font) { (void)font; return 0; }
void hb_ft_font_set_funcs(hb_font_t *font) { (void)font; }

/* ========================================================================= */
/* OpenType layout stubs                                                     */
/* ========================================================================= */

unsigned int hb_ot_layout_table_get_script_tags(hb_face_t *face,
    hb_tag_t tt, unsigned int so, unsigned int *sc, hb_tag_t *st) {
    (void)face; (void)tt; (void)so; (void)st;
    if (sc) *sc = 0;
    return 0;
}
hb_bool_t hb_ot_layout_table_find_script(hb_face_t *face, hb_tag_t tt,
    hb_tag_t st, unsigned int *si) {
    (void)face; (void)tt; (void)st;
    if (si) *si = 0;
    return 0;
}
unsigned int hb_ot_layout_table_get_feature_tags(hb_face_t *face,
    hb_tag_t tt, unsigned int so, unsigned int *fc, hb_tag_t *ft) {
    (void)face; (void)tt; (void)so; (void)ft;
    if (fc) *fc = 0;
    return 0;
}
unsigned int hb_ot_layout_script_get_language_tags(hb_face_t *face,
    hb_tag_t tt, unsigned int si, unsigned int so, unsigned int *lc,
    hb_tag_t *lt) {
    (void)face; (void)tt; (void)si; (void)so; (void)lt;
    if (lc) *lc = 0;
    return 0;
}
hb_bool_t hb_ot_layout_has_substitution(hb_face_t *face) { (void)face; return 0; }
hb_bool_t hb_ot_layout_has_positioning(hb_face_t *face) { (void)face; return 0; }
void hb_ot_layout_collect_lookups(hb_face_t *face, hb_tag_t tt,
    const hb_tag_t *sc, const hb_tag_t *la, const hb_tag_t *fe,
    hb_set_t *li) {
    (void)face; (void)tt; (void)sc; (void)la; (void)fe; (void)li;
}
void hb_ot_layout_collect_features(hb_face_t *face, hb_tag_t tt,
    const hb_tag_t *sc, const hb_tag_t *la, const hb_tag_t *fe,
    hb_set_t *fi) {
    (void)face; (void)tt; (void)sc; (void)la; (void)fe; (void)fi;
}

/* OT Var stubs */
hb_bool_t hb_ot_var_has_data(hb_face_t *face) { (void)face; return 0; }
unsigned int hb_ot_var_get_axis_count(hb_face_t *face) { (void)face; return 0; }
unsigned int hb_ot_var_get_axis_infos(hb_face_t *face, unsigned int so,
    unsigned int *ac, hb_ot_var_axis_info_t *aa) {
    (void)face; (void)so; (void)aa;
    if (ac) *ac = 0;
    return 0;
}
hb_bool_t hb_ot_var_find_axis_info(hb_face_t *face, hb_tag_t tag,
    hb_ot_var_axis_info_t *ai) {
    (void)face; (void)tag; (void)ai;
    return 0;
}
unsigned int hb_ot_var_get_named_instance_count(hb_face_t *face) { (void)face; return 0; }
hb_tag_t hb_ot_var_named_instance_get_subfamily_name_id(hb_face_t *face,
    unsigned int ii) { (void)face; (void)ii; return HB_OT_NAME_ID_INVALID; }
unsigned int hb_ot_var_named_instance_get_design_coords(hb_face_t *face,
    unsigned int ii, unsigned int *cl, float *co) {
    (void)face; (void)ii; (void)co;
    if (cl) *cl = 0;
    return 0;
}
void hb_ot_var_normalize_variations(hb_face_t *face,
    const hb_variation_t *v, unsigned int vl, int *c, unsigned int cl) {
    (void)face; (void)v; (void)vl; (void)c; (void)cl;
}
void hb_ot_var_normalize_coords(hb_face_t *face, unsigned int cl,
    const float *dc, int *nc) {
    (void)face; (void)cl; (void)dc; (void)nc;
}

/* OT Metrics */
hb_bool_t hb_ot_metrics_get_position(hb_font_t *font,
    hb_ot_metrics_tag_t tag, hb_position_t *pos) {
    (void)font; (void)tag;
    if (pos) *pos = 0;
    return 0;
}

/* OT Name */
const hb_ot_name_entry_t *hb_ot_name_list_names(hb_face_t *face,
    unsigned int *num) {
    (void)face;
    if (num) *num = 0;
    return NULL;
}
unsigned int hb_ot_name_get_utf8(hb_face_t *face, hb_ot_name_id_t nid,
    hb_language_t lang, unsigned int *ts, char *text) {
    (void)face; (void)nid; (void)lang;
    if (ts) *ts = 0;
    if (text) text[0] = '\0';
    return 0;
}
unsigned int hb_ot_name_get_utf16(hb_face_t *face, hb_ot_name_id_t nid,
    hb_language_t lang, unsigned int *ts, uint16_t *text) {
    (void)face; (void)nid; (void)lang;
    if (ts) *ts = 0;
    if (text) text[0] = 0;
    return 0;
}

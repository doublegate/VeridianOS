/*
 * VeridianOS libc -- harfbuzz/hb-ot.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz OpenType API (combined header).
 */

#ifndef _HARFBUZZ_HB_OT_H
#define _HARFBUZZ_HB_OT_H

#include <harfbuzz/hb-common.h>
#include <harfbuzz/hb-face.h>
#include <harfbuzz/hb-font.h>
#include <harfbuzz/hb-set.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* OpenType tag constants                                                    */
/* ========================================================================= */

#define HB_OT_TAG_DEFAULT_SCRIPT   HB_TAG('D','F','L','T')
#define HB_OT_TAG_DEFAULT_LANGUAGE HB_TAG('d','f','l','t')

#define HB_OT_LAYOUT_DEFAULT_LANGUAGE_INDEX  0xFFFF
#define HB_OT_LAYOUT_NO_SCRIPT_INDEX         0xFFFF
#define HB_OT_LAYOUT_NO_FEATURE_INDEX        0xFFFF
#define HB_OT_LAYOUT_NO_VARIATIONS_INDEX     0xFFFFFFFF

/* ========================================================================= */
/* OpenType Layout                                                           */
/* ========================================================================= */

#define HB_OT_TAG_GSUB  HB_TAG('G','S','U','B')
#define HB_OT_TAG_GPOS  HB_TAG('G','P','O','S')
#define HB_OT_TAG_GDEF  HB_TAG('G','D','E','F')

unsigned int  hb_ot_layout_table_get_script_tags(hb_face_t *face,
                                                    hb_tag_t table_tag,
                                                    unsigned int start_offset,
                                                    unsigned int *script_count,
                                                    hb_tag_t *script_tags);
hb_bool_t     hb_ot_layout_table_find_script(hb_face_t *face,
                                                hb_tag_t table_tag,
                                                hb_tag_t script_tag,
                                                unsigned int *script_index);
unsigned int  hb_ot_layout_table_get_feature_tags(hb_face_t *face,
                                                     hb_tag_t table_tag,
                                                     unsigned int start_offset,
                                                     unsigned int *feature_count,
                                                     hb_tag_t *feature_tags);
unsigned int  hb_ot_layout_script_get_language_tags(hb_face_t *face,
                                                       hb_tag_t table_tag,
                                                       unsigned int script_index,
                                                       unsigned int start_offset,
                                                       unsigned int *language_count,
                                                       hb_tag_t *language_tags);
hb_bool_t     hb_ot_layout_has_substitution(hb_face_t *face);
hb_bool_t     hb_ot_layout_has_positioning(hb_face_t *face);
void          hb_ot_layout_collect_lookups(hb_face_t *face,
                                             hb_tag_t table_tag,
                                             const hb_tag_t *scripts,
                                             const hb_tag_t *languages,
                                             const hb_tag_t *features,
                                             hb_set_t *lookup_indexes);
void          hb_ot_layout_collect_features(hb_face_t *face,
                                              hb_tag_t table_tag,
                                              const hb_tag_t *scripts,
                                              const hb_tag_t *languages,
                                              const hb_tag_t *features,
                                              hb_set_t *feature_indexes);

/* ========================================================================= */
/* OpenType Font Variations                                                  */
/* ========================================================================= */

#define HB_OT_TAG_VAR_AXIS_ITALIC    HB_TAG('i','t','a','l')
#define HB_OT_TAG_VAR_AXIS_OPTICAL_SIZE HB_TAG('o','p','s','z')
#define HB_OT_TAG_VAR_AXIS_SLANT     HB_TAG('s','l','n','t')
#define HB_OT_TAG_VAR_AXIS_WEIGHT    HB_TAG('w','g','h','t')
#define HB_OT_TAG_VAR_AXIS_WIDTH     HB_TAG('w','d','t','h')

typedef struct hb_ot_var_axis_info_t {
    unsigned int  axis_index;
    hb_tag_t      tag;
    unsigned int  name_id;
    unsigned int  flags;
    float         min_value;
    float         default_value;
    float         max_value;
    unsigned int  reserved;
} hb_ot_var_axis_info_t;

hb_bool_t     hb_ot_var_has_data(hb_face_t *face);
unsigned int  hb_ot_var_get_axis_count(hb_face_t *face);
unsigned int  hb_ot_var_get_axis_infos(hb_face_t *face,
                                          unsigned int start_offset,
                                          unsigned int *axes_count,
                                          hb_ot_var_axis_info_t *axes_array);
hb_bool_t     hb_ot_var_find_axis_info(hb_face_t *face, hb_tag_t axis_tag,
                                          hb_ot_var_axis_info_t *axis_info);
unsigned int  hb_ot_var_get_named_instance_count(hb_face_t *face);
hb_tag_t      hb_ot_var_named_instance_get_subfamily_name_id(hb_face_t *face,
                                                                unsigned int instance_index);
unsigned int  hb_ot_var_named_instance_get_design_coords(hb_face_t *face,
                                                            unsigned int instance_index,
                                                            unsigned int *coords_length,
                                                            float *coords);
void          hb_ot_var_normalize_variations(hb_face_t *face,
                                               const hb_variation_t *variations,
                                               unsigned int variations_length,
                                               int *coords,
                                               unsigned int coords_length);
void          hb_ot_var_normalize_coords(hb_face_t *face,
                                           unsigned int coords_length,
                                           const float *design_coords,
                                           int *normalized_coords);

/* ========================================================================= */
/* OpenType Metrics                                                          */
/* ========================================================================= */

typedef enum {
    HB_OT_METRICS_TAG_HORIZONTAL_ASCENDER   = HB_TAG('h','a','s','c'),
    HB_OT_METRICS_TAG_HORIZONTAL_DESCENDER  = HB_TAG('h','d','s','c'),
    HB_OT_METRICS_TAG_HORIZONTAL_LINE_GAP   = HB_TAG('h','l','g','p'),
    HB_OT_METRICS_TAG_VERTICAL_ASCENDER     = HB_TAG('v','a','s','c'),
    HB_OT_METRICS_TAG_VERTICAL_DESCENDER    = HB_TAG('v','d','s','c'),
    HB_OT_METRICS_TAG_VERTICAL_LINE_GAP     = HB_TAG('v','l','g','p'),
    HB_OT_METRICS_TAG_HORIZONTAL_CARET_RISE = HB_TAG('h','c','r','s'),
    HB_OT_METRICS_TAG_HORIZONTAL_CARET_RUN  = HB_TAG('h','c','r','n'),
    HB_OT_METRICS_TAG_HORIZONTAL_CARET_OFFSET = HB_TAG('h','c','o','f'),
    HB_OT_METRICS_TAG_VERTICAL_CARET_RISE   = HB_TAG('v','c','r','s'),
    HB_OT_METRICS_TAG_VERTICAL_CARET_RUN    = HB_TAG('v','c','r','n'),
    HB_OT_METRICS_TAG_VERTICAL_CARET_OFFSET = HB_TAG('v','c','o','f'),
    HB_OT_METRICS_TAG_X_HEIGHT              = HB_TAG('x','h','g','t'),
    HB_OT_METRICS_TAG_CAP_HEIGHT            = HB_TAG('c','p','h','t'),
    HB_OT_METRICS_TAG_SUBSCRIPT_EM_X_SIZE   = HB_TAG('s','b','x','s'),
    HB_OT_METRICS_TAG_SUBSCRIPT_EM_Y_SIZE   = HB_TAG('s','b','y','s'),
    HB_OT_METRICS_TAG_SUBSCRIPT_EM_X_OFFSET = HB_TAG('s','b','x','o'),
    HB_OT_METRICS_TAG_SUBSCRIPT_EM_Y_OFFSET = HB_TAG('s','b','y','o'),
    HB_OT_METRICS_TAG_SUPERSCRIPT_EM_X_SIZE = HB_TAG('s','p','x','s'),
    HB_OT_METRICS_TAG_SUPERSCRIPT_EM_Y_SIZE = HB_TAG('s','p','y','s'),
    HB_OT_METRICS_TAG_SUPERSCRIPT_EM_X_OFFSET = HB_TAG('s','p','x','o'),
    HB_OT_METRICS_TAG_SUPERSCRIPT_EM_Y_OFFSET = HB_TAG('s','p','y','o'),
    HB_OT_METRICS_TAG_STRIKEOUT_SIZE        = HB_TAG('s','t','r','s'),
    HB_OT_METRICS_TAG_STRIKEOUT_OFFSET      = HB_TAG('s','t','r','o'),
    HB_OT_METRICS_TAG_UNDERLINE_SIZE        = HB_TAG('u','n','d','s'),
    HB_OT_METRICS_TAG_UNDERLINE_OFFSET      = HB_TAG('u','n','d','o'),
    _HB_OT_METRICS_TAG_MAX_VALUE = HB_TAG_MAX_SIGNED
} hb_ot_metrics_tag_t;

hb_bool_t     hb_ot_metrics_get_position(hb_font_t *font,
                                            hb_ot_metrics_tag_t metrics_tag,
                                            hb_position_t *position);

/* ========================================================================= */
/* OpenType Name                                                             */
/* ========================================================================= */

typedef unsigned int hb_ot_name_id_t;

#define HB_OT_NAME_ID_INVALID  0xFFFF

typedef struct hb_ot_name_entry_t {
    hb_ot_name_id_t  name_id;
    hb_var_int_t      var;
    hb_language_t     language;
} hb_ot_name_entry_t;

const hb_ot_name_entry_t *hb_ot_name_list_names(hb_face_t *face,
                                                    unsigned int *num_entries);
unsigned int  hb_ot_name_get_utf8(hb_face_t *face,
                                    hb_ot_name_id_t name_id,
                                    hb_language_t language,
                                    unsigned int *text_size,
                                    char *text);
unsigned int  hb_ot_name_get_utf16(hb_face_t *face,
                                     hb_ot_name_id_t name_id,
                                     hb_language_t language,
                                     unsigned int *text_size,
                                     uint16_t *text);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_OT_H */

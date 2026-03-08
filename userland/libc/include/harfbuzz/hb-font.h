/*
 * VeridianOS libc -- harfbuzz/hb-font.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz font API.
 */

#ifndef _HARFBUZZ_HB_FONT_H
#define _HARFBUZZ_HB_FONT_H

#include <harfbuzz/hb-common.h>
#include <harfbuzz/hb-face.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct hb_font_t hb_font_t;
typedef struct hb_font_funcs_t hb_font_funcs_t;

typedef struct hb_font_extents_t {
    hb_position_t  ascender;
    hb_position_t  descender;
    hb_position_t  line_gap;
    hb_position_t  reserved9;
    hb_position_t  reserved8;
    hb_position_t  reserved7;
    hb_position_t  reserved6;
    hb_position_t  reserved5;
    hb_position_t  reserved4;
    hb_position_t  reserved3;
    hb_position_t  reserved2;
    hb_position_t  reserved1;
} hb_font_extents_t;

typedef struct hb_glyph_extents_t {
    hb_position_t  x_bearing;
    hb_position_t  y_bearing;
    hb_position_t  width;
    hb_position_t  height;
} hb_glyph_extents_t;

/* Font funcs */
hb_font_funcs_t *hb_font_funcs_create(void);
hb_font_funcs_t *hb_font_funcs_get_empty(void);
hb_font_funcs_t *hb_font_funcs_reference(hb_font_funcs_t *ffuncs);
void             hb_font_funcs_destroy(hb_font_funcs_t *ffuncs);
void             hb_font_funcs_make_immutable(hb_font_funcs_t *ffuncs);
hb_bool_t        hb_font_funcs_is_immutable(hb_font_funcs_t *ffuncs);

/* Font lifecycle */
hb_font_t   *hb_font_create(hb_face_t *face);
hb_font_t   *hb_font_create_sub_font(hb_font_t *parent);
hb_font_t   *hb_font_get_empty(void);
hb_font_t   *hb_font_reference(hb_font_t *font);
void          hb_font_destroy(hb_font_t *font);
void          hb_font_make_immutable(hb_font_t *font);
hb_bool_t     hb_font_is_immutable(hb_font_t *font);

/* Font properties */
hb_face_t   *hb_font_get_face(hb_font_t *font);
void          hb_font_set_funcs(hb_font_t *font, hb_font_funcs_t *klass,
                                  void *font_data,
                                  hb_destroy_func_t destroy);
void          hb_font_set_funcs_data(hb_font_t *font, void *font_data,
                                       hb_destroy_func_t destroy);
void          hb_font_set_scale(hb_font_t *font, int x_scale, int y_scale);
void          hb_font_get_scale(hb_font_t *font, int *x_scale, int *y_scale);
void          hb_font_set_ppem(hb_font_t *font, unsigned int x_ppem,
                                 unsigned int y_ppem);
void          hb_font_get_ppem(hb_font_t *font, unsigned int *x_ppem,
                                 unsigned int *y_ppem);
void          hb_font_set_ptem(hb_font_t *font, float ptem);
float         hb_font_get_ptem(hb_font_t *font);
void          hb_font_set_synthetic_bold(hb_font_t *font,
                                           float x_embolden,
                                           float y_embolden,
                                           hb_bool_t in_place);
void          hb_font_set_synthetic_slant(hb_font_t *font, float slant);

/* Glyph queries */
hb_bool_t     hb_font_get_h_extents(hb_font_t *font,
                                       hb_font_extents_t *extents);
hb_bool_t     hb_font_get_v_extents(hb_font_t *font,
                                       hb_font_extents_t *extents);
hb_bool_t     hb_font_get_nominal_glyph(hb_font_t *font,
                                           hb_codepoint_t unicode,
                                           hb_codepoint_t *glyph);
hb_bool_t     hb_font_get_variation_glyph(hb_font_t *font,
                                             hb_codepoint_t unicode,
                                             hb_codepoint_t variation_selector,
                                             hb_codepoint_t *glyph);
hb_bool_t     hb_font_get_nominal_glyphs(hb_font_t *font,
                                            unsigned int count,
                                            const hb_codepoint_t *first_unicode,
                                            unsigned int unicode_stride,
                                            hb_codepoint_t *first_glyph,
                                            unsigned int glyph_stride);
hb_position_t hb_font_get_glyph_h_advance(hb_font_t *font,
                                             hb_codepoint_t glyph);
hb_position_t hb_font_get_glyph_v_advance(hb_font_t *font,
                                             hb_codepoint_t glyph);
hb_bool_t     hb_font_get_glyph_h_origin(hb_font_t *font,
                                            hb_codepoint_t glyph,
                                            hb_position_t *x,
                                            hb_position_t *y);
hb_bool_t     hb_font_get_glyph_v_origin(hb_font_t *font,
                                            hb_codepoint_t glyph,
                                            hb_position_t *x,
                                            hb_position_t *y);
hb_position_t hb_font_get_glyph_h_kerning(hb_font_t *font,
                                             hb_codepoint_t left_glyph,
                                             hb_codepoint_t right_glyph);
hb_bool_t     hb_font_get_glyph_extents(hb_font_t *font,
                                           hb_codepoint_t glyph,
                                           hb_glyph_extents_t *extents);
hb_bool_t     hb_font_get_glyph_contour_point(hb_font_t *font,
                                                  hb_codepoint_t glyph,
                                                  unsigned int point_index,
                                                  hb_position_t *x,
                                                  hb_position_t *y);
hb_bool_t     hb_font_get_glyph_name(hb_font_t *font,
                                        hb_codepoint_t glyph,
                                        char *name, unsigned int size);
hb_bool_t     hb_font_get_glyph_from_name(hb_font_t *font,
                                             const char *name, int len,
                                             hb_codepoint_t *glyph);

/* Variations */
void          hb_font_set_variations(hb_font_t *font,
                                       const hb_variation_t *variations,
                                       unsigned int variations_length);
void          hb_font_set_var_coords_design(hb_font_t *font,
                                              const float *coords,
                                              unsigned int coords_length);
void          hb_font_set_var_coords_normalized(hb_font_t *font,
                                                   const int *coords,
                                                   unsigned int coords_length);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_FONT_H */

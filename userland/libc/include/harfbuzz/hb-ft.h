/*
 * VeridianOS libc -- harfbuzz/hb-ft.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz FreeType integration API.
 */

#ifndef _HARFBUZZ_HB_FT_H
#define _HARFBUZZ_HB_FT_H

#include <harfbuzz/hb-common.h>
#include <harfbuzz/hb-font.h>
#include <harfbuzz/hb-face.h>
#include <ft2build.h>
#include <freetype/freetype.h>

#ifdef __cplusplus
extern "C" {
#endif

hb_face_t   *hb_ft_face_create(FT_Face ft_face,
                                  hb_destroy_func_t destroy);
hb_face_t   *hb_ft_face_create_cached(FT_Face ft_face);
hb_face_t   *hb_ft_face_create_referenced(FT_Face ft_face);

hb_font_t   *hb_ft_font_create(FT_Face ft_face,
                                  hb_destroy_func_t destroy);
hb_font_t   *hb_ft_font_create_referenced(FT_Face ft_face);

FT_Face       hb_ft_font_get_face(hb_font_t *font);
FT_Face       hb_ft_font_lock_face(hb_font_t *font);
void          hb_ft_font_unlock_face(hb_font_t *font);

void          hb_ft_font_set_load_flags(hb_font_t *font, int load_flags);
int           hb_ft_font_get_load_flags(hb_font_t *font);
void          hb_ft_font_changed(hb_font_t *font);
hb_bool_t     hb_ft_hb_font_changed(hb_font_t *font);

void          hb_ft_font_set_funcs(hb_font_t *font);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_FT_H */

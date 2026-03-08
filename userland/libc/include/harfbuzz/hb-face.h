/*
 * VeridianOS libc -- harfbuzz/hb-face.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz face (font file) API.
 */

#ifndef _HARFBUZZ_HB_FACE_H
#define _HARFBUZZ_HB_FACE_H

#include <harfbuzz/hb-common.h>
#include <harfbuzz/hb-blob.h>
#include <harfbuzz/hb-set.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct hb_face_t hb_face_t;

typedef hb_blob_t *(*hb_reference_table_func_t)(hb_face_t *face,
                                                   hb_tag_t tag,
                                                   void *user_data);

hb_face_t    *hb_face_create(hb_blob_t *blob, unsigned int index);
hb_face_t    *hb_face_create_for_tables(hb_reference_table_func_t reference_table_func,
                                          void *user_data,
                                          hb_destroy_func_t destroy);
hb_face_t    *hb_face_get_empty(void);
hb_face_t    *hb_face_reference(hb_face_t *face);
void          hb_face_destroy(hb_face_t *face);
void          hb_face_make_immutable(hb_face_t *face);
hb_bool_t     hb_face_is_immutable(hb_face_t *face);

hb_blob_t    *hb_face_reference_table(hb_face_t *face, hb_tag_t tag);
hb_blob_t    *hb_face_reference_blob(hb_face_t *face);

void          hb_face_set_index(hb_face_t *face, unsigned int index);
unsigned int  hb_face_get_index(hb_face_t *face);

void          hb_face_set_upem(hb_face_t *face, unsigned int upem);
unsigned int  hb_face_get_upem(hb_face_t *face);

void          hb_face_set_glyph_count(hb_face_t *face, unsigned int glyph_count);
unsigned int  hb_face_get_glyph_count(hb_face_t *face);

unsigned int  hb_face_get_table_tags(hb_face_t *face,
                                       unsigned int start_offset,
                                       unsigned int *table_count,
                                       hb_tag_t *table_tags);

void          hb_face_collect_unicodes(hb_face_t *face, hb_set_t *out);
void          hb_face_collect_nominal_glyph_mapping(hb_face_t *face,
                                                       hb_map_t *mapping,
                                                       hb_set_t *unicodes);
void          hb_face_collect_variation_selectors(hb_face_t *face,
                                                    hb_set_t *out);
void          hb_face_collect_variation_unicodes(hb_face_t *face,
                                                   hb_codepoint_t variation_selector,
                                                   hb_set_t *out);

/* Builder API */
hb_face_t    *hb_face_builder_create(void);
hb_bool_t     hb_face_builder_add_table(hb_face_t *face, hb_tag_t tag,
                                          hb_blob_t *blob);
void          hb_face_builder_sort_tables(hb_face_t *face,
                                            const hb_tag_t *tags);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_FACE_H */

/*
 * VeridianOS libc -- harfbuzz/hb-shape.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz shaping API.
 */

#ifndef _HARFBUZZ_HB_SHAPE_H
#define _HARFBUZZ_HB_SHAPE_H

#include <harfbuzz/hb-common.h>
#include <harfbuzz/hb-buffer.h>
#include <harfbuzz/hb-font.h>

#ifdef __cplusplus
extern "C" {
#endif

void          hb_shape(hb_font_t *font, hb_buffer_t *buffer,
                        const hb_feature_t *features,
                        unsigned int num_features);

hb_bool_t     hb_shape_full(hb_font_t *font, hb_buffer_t *buffer,
                              const hb_feature_t *features,
                              unsigned int num_features,
                              const char * const *shaper_list);

hb_bool_t     hb_shape_justify(hb_font_t *font, hb_buffer_t *buffer,
                                 const hb_feature_t *features,
                                 unsigned int num_features,
                                 const char * const *shaper_list,
                                 float min_target_advance,
                                 float max_target_advance,
                                 float *advance,
                                 hb_tag_t *var_tag,
                                 float *var_value);

const char  **hb_shape_list_shapers(void);

/* Plan (advanced) */
typedef struct hb_shape_plan_t hb_shape_plan_t;

hb_shape_plan_t *hb_shape_plan_create(hb_face_t *face,
                                         const void *props,
                                         const hb_feature_t *user_features,
                                         unsigned int num_user_features,
                                         const char * const *shaper_list);
hb_shape_plan_t *hb_shape_plan_create_cached(hb_face_t *face,
                                                const void *props,
                                                const hb_feature_t *user_features,
                                                unsigned int num_user_features,
                                                const char * const *shaper_list);
hb_shape_plan_t *hb_shape_plan_create2(hb_face_t *face,
                                          const void *props,
                                          const hb_feature_t *user_features,
                                          unsigned int num_user_features,
                                          const int *coords,
                                          unsigned int num_coords,
                                          const char * const *shaper_list);
hb_shape_plan_t *hb_shape_plan_create_cached2(hb_face_t *face,
                                                 const void *props,
                                                 const hb_feature_t *user_features,
                                                 unsigned int num_user_features,
                                                 const int *coords,
                                                 unsigned int num_coords,
                                                 const char * const *shaper_list);
hb_shape_plan_t *hb_shape_plan_get_empty(void);
hb_shape_plan_t *hb_shape_plan_reference(hb_shape_plan_t *shape_plan);
void             hb_shape_plan_destroy(hb_shape_plan_t *shape_plan);
hb_bool_t        hb_shape_plan_execute(hb_shape_plan_t *shape_plan,
                                          hb_font_t *font,
                                          hb_buffer_t *buffer,
                                          const hb_feature_t *features,
                                          unsigned int num_features);
const char      *hb_shape_plan_get_shaper(hb_shape_plan_t *shape_plan);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_SHAPE_H */

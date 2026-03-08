/*
 * VeridianOS libc -- harfbuzz/hb-set.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz set API.
 */

#ifndef _HARFBUZZ_HB_SET_H
#define _HARFBUZZ_HB_SET_H

#include <harfbuzz/hb-common.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct hb_set_t hb_set_t;

#define HB_SET_VALUE_INVALID ((hb_codepoint_t)-1)

hb_set_t     *hb_set_create(void);
hb_set_t     *hb_set_get_empty(void);
hb_set_t     *hb_set_reference(hb_set_t *set);
void          hb_set_destroy(hb_set_t *set);

hb_bool_t     hb_set_allocation_successful(hb_set_t *set);
hb_set_t     *hb_set_copy(const hb_set_t *set);
void          hb_set_clear(hb_set_t *set);
hb_bool_t     hb_set_is_empty(const hb_set_t *set);
hb_bool_t     hb_set_has(const hb_set_t *set, hb_codepoint_t codepoint);
void          hb_set_add(hb_set_t *set, hb_codepoint_t codepoint);
void          hb_set_add_range(hb_set_t *set, hb_codepoint_t first,
                                hb_codepoint_t last);
void          hb_set_add_sorted_array(hb_set_t *set,
                                        const hb_codepoint_t *sorted_codepoints,
                                        unsigned int num_codepoints);
void          hb_set_del(hb_set_t *set, hb_codepoint_t codepoint);
void          hb_set_del_range(hb_set_t *set, hb_codepoint_t first,
                                hb_codepoint_t last);
hb_bool_t     hb_set_is_equal(const hb_set_t *set, const hb_set_t *other);
hb_bool_t     hb_set_is_subset(const hb_set_t *set, const hb_set_t *larger_set);
unsigned int  hb_set_get_population(const hb_set_t *set);
hb_codepoint_t hb_set_get_min(const hb_set_t *set);
hb_codepoint_t hb_set_get_max(const hb_set_t *set);
hb_bool_t     hb_set_next(const hb_set_t *set, hb_codepoint_t *codepoint);
hb_bool_t     hb_set_previous(const hb_set_t *set, hb_codepoint_t *codepoint);
hb_bool_t     hb_set_next_range(const hb_set_t *set, hb_codepoint_t *first,
                                  hb_codepoint_t *last);
hb_bool_t     hb_set_previous_range(const hb_set_t *set,
                                       hb_codepoint_t *first,
                                       hb_codepoint_t *last);
unsigned int  hb_set_next_many(const hb_set_t *set, hb_codepoint_t codepoint,
                                 hb_codepoint_t *out,
                                 unsigned int size);
void          hb_set_union(hb_set_t *set, const hb_set_t *other);
void          hb_set_intersect(hb_set_t *set, const hb_set_t *other);
void          hb_set_subtract(hb_set_t *set, const hb_set_t *other);
void          hb_set_symmetric_difference(hb_set_t *set,
                                             const hb_set_t *other);
void          hb_set_invert(hb_set_t *set);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_SET_H */

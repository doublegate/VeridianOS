/*
 * VeridianOS libc -- harfbuzz/hb-map.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz map API.
 */

#ifndef _HARFBUZZ_HB_MAP_H
#define _HARFBUZZ_HB_MAP_H

#include <harfbuzz/hb-common.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct hb_map_t hb_map_t;

#define HB_MAP_VALUE_INVALID ((hb_codepoint_t)-1)

hb_map_t     *hb_map_create(void);
hb_map_t     *hb_map_get_empty(void);
hb_map_t     *hb_map_reference(hb_map_t *map);
void          hb_map_destroy(hb_map_t *map);
hb_bool_t     hb_map_allocation_successful(hb_map_t *map);
hb_map_t     *hb_map_copy(const hb_map_t *map);
void          hb_map_clear(hb_map_t *map);
hb_bool_t     hb_map_is_empty(const hb_map_t *map);
unsigned int  hb_map_get_population(const hb_map_t *map);
hb_bool_t     hb_map_is_equal(const hb_map_t *map, const hb_map_t *other);
unsigned int  hb_map_hash(const hb_map_t *map);
void          hb_map_set(hb_map_t *map, hb_codepoint_t key,
                           hb_codepoint_t value);
hb_codepoint_t hb_map_get(const hb_map_t *map, hb_codepoint_t key);
void          hb_map_del(hb_map_t *map, hb_codepoint_t key);
hb_bool_t     hb_map_has(const hb_map_t *map, hb_codepoint_t key);
void          hb_map_update(hb_map_t *map, const hb_map_t *other);
hb_bool_t     hb_map_next(const hb_map_t *map, int *idx,
                            hb_codepoint_t *key, hb_codepoint_t *value);
void          hb_map_keys(const hb_map_t *map, hb_set_t *keys);
void          hb_map_values(const hb_map_t *map, hb_set_t *values);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_MAP_H */

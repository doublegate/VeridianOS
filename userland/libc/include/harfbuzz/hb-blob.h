/*
 * VeridianOS libc -- harfbuzz/hb-blob.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz blob (binary data) API.
 */

#ifndef _HARFBUZZ_HB_BLOB_H
#define _HARFBUZZ_HB_BLOB_H

#include <harfbuzz/hb-common.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct hb_blob_t hb_blob_t;

hb_blob_t    *hb_blob_create(const char *data, unsigned int length,
                               hb_memory_mode_t mode,
                               void *user_data,
                               hb_destroy_func_t destroy);
hb_blob_t    *hb_blob_create_from_file(const char *file_name);
hb_blob_t    *hb_blob_create_from_file_or_fail(const char *file_name);
hb_blob_t    *hb_blob_create_sub_blob(hb_blob_t *parent,
                                        unsigned int offset,
                                        unsigned int length);
hb_blob_t    *hb_blob_copy_writable_or_fail(hb_blob_t *blob);
hb_blob_t    *hb_blob_get_empty(void);
hb_blob_t    *hb_blob_reference(hb_blob_t *blob);
void          hb_blob_destroy(hb_blob_t *blob);
void          hb_blob_make_immutable(hb_blob_t *blob);
hb_bool_t     hb_blob_is_immutable(hb_blob_t *blob);
unsigned int  hb_blob_get_length(hb_blob_t *blob);
const char   *hb_blob_get_data(hb_blob_t *blob, unsigned int *length);
char         *hb_blob_get_data_writable(hb_blob_t *blob,
                                          unsigned int *length);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_BLOB_H */

/*
 * VeridianOS libc -- harfbuzz/hb-buffer.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz buffer API for text shaping.
 */

#ifndef _HARFBUZZ_HB_BUFFER_H
#define _HARFBUZZ_HB_BUFFER_H

#include <harfbuzz/hb-common.h>
#include <harfbuzz/hb-unicode.h>
#include <harfbuzz/hb-font.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct hb_buffer_t hb_buffer_t;

/* ========================================================================= */
/* Glyph info and position                                                   */
/* ========================================================================= */

typedef struct hb_glyph_info_t {
    hb_codepoint_t  codepoint;
    hb_mask_t       mask;
    uint32_t        cluster;
    hb_var_int_t    var1;
    hb_var_int_t    var2;
} hb_glyph_info_t;

typedef struct hb_glyph_position_t {
    hb_position_t  x_advance;
    hb_position_t  y_advance;
    hb_position_t  x_offset;
    hb_position_t  y_offset;
    hb_var_int_t   var;
} hb_glyph_position_t;

/* ========================================================================= */
/* Content type                                                              */
/* ========================================================================= */

typedef enum {
    HB_BUFFER_CONTENT_TYPE_INVALID = 0,
    HB_BUFFER_CONTENT_TYPE_UNICODE,
    HB_BUFFER_CONTENT_TYPE_GLYPHS
} hb_buffer_content_type_t;

typedef enum {
    HB_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES  = 0,
    HB_BUFFER_CLUSTER_LEVEL_MONOTONE_CHARACTERS = 1,
    HB_BUFFER_CLUSTER_LEVEL_CHARACTERS          = 2,
    HB_BUFFER_CLUSTER_LEVEL_DEFAULT = HB_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES
} hb_buffer_cluster_level_t;

typedef enum {
    HB_BUFFER_FLAG_DEFAULT                      = 0x00000000u,
    HB_BUFFER_FLAG_BOT                          = 0x00000001u,
    HB_BUFFER_FLAG_EOT                          = 0x00000002u,
    HB_BUFFER_FLAG_PRESERVE_DEFAULT_IGNORABLES  = 0x00000004u,
    HB_BUFFER_FLAG_REMOVE_DEFAULT_IGNORABLES    = 0x00000008u,
    HB_BUFFER_FLAG_DO_NOT_INSERT_DOTTED_CIRCLE  = 0x00000010u,
    HB_BUFFER_FLAG_VERIFY                       = 0x00000020u,
    HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT     = 0x00000040u,
    HB_BUFFER_FLAG_PRODUCE_SAFE_TO_INSERT_TATWEEL = 0x00000080u,
    HB_BUFFER_FLAG_DEFINED                      = 0x000000FFu
} hb_buffer_flags_t;

typedef enum {
    HB_BUFFER_SERIALIZE_FORMAT_TEXT    = HB_TAG('T','E','X','T'),
    HB_BUFFER_SERIALIZE_FORMAT_JSON    = HB_TAG('J','S','O','N'),
    HB_BUFFER_SERIALIZE_FORMAT_INVALID = HB_TAG_NONE
} hb_buffer_serialize_format_t;

typedef enum {
    HB_BUFFER_SERIALIZE_FLAG_DEFAULT          = 0x00000000u,
    HB_BUFFER_SERIALIZE_FLAG_NO_CLUSTERS      = 0x00000001u,
    HB_BUFFER_SERIALIZE_FLAG_NO_POSITIONS     = 0x00000002u,
    HB_BUFFER_SERIALIZE_FLAG_NO_GLYPH_NAMES   = 0x00000004u,
    HB_BUFFER_SERIALIZE_FLAG_GLYPH_EXTENTS    = 0x00000008u,
    HB_BUFFER_SERIALIZE_FLAG_GLYPH_FLAGS      = 0x00000010u,
    HB_BUFFER_SERIALIZE_FLAG_NO_ADVANCES      = 0x00000020u,
    HB_BUFFER_SERIALIZE_FLAG_DEFINED          = 0x0000003Fu
} hb_buffer_serialize_flags_t;

typedef enum {
    HB_BUFFER_DIFF_FLAG_EQUAL                  = 0x0000,
    HB_BUFFER_DIFF_FLAG_CONTENT_TYPE_MISMATCH  = 0x0001,
    HB_BUFFER_DIFF_FLAG_LENGTH_MISMATCH        = 0x0002,
    HB_BUFFER_DIFF_FLAG_NOTDEF_PRESENT         = 0x0004,
    HB_BUFFER_DIFF_FLAG_DOTTED_CIRCLE_PRESENT  = 0x0008,
    HB_BUFFER_DIFF_FLAG_CODEPOINT_MISMATCH     = 0x0010,
    HB_BUFFER_DIFF_FLAG_CLUSTER_MISMATCH       = 0x0020,
    HB_BUFFER_DIFF_FLAG_GLYPH_FLAGS_MISMATCH   = 0x0040,
    HB_BUFFER_DIFF_FLAG_POSITION_MISMATCH      = 0x0080
} hb_buffer_diff_flags_t;

/* ========================================================================= */
/* Buffer lifecycle                                                          */
/* ========================================================================= */

hb_buffer_t          *hb_buffer_create(void);
hb_buffer_t          *hb_buffer_create_similar(const hb_buffer_t *src);
hb_buffer_t          *hb_buffer_get_empty(void);
hb_buffer_t          *hb_buffer_reference(hb_buffer_t *buffer);
void                  hb_buffer_destroy(hb_buffer_t *buffer);
void                  hb_buffer_reset(hb_buffer_t *buffer);
void                  hb_buffer_clear_contents(hb_buffer_t *buffer);

hb_bool_t             hb_buffer_pre_allocate(hb_buffer_t *buffer,
                                                unsigned int size);
hb_bool_t             hb_buffer_allocation_successful(hb_buffer_t *buffer);

/* ========================================================================= */
/* Buffer content                                                            */
/* ========================================================================= */

void                  hb_buffer_add(hb_buffer_t *buffer,
                                      hb_codepoint_t codepoint,
                                      unsigned int cluster);
void                  hb_buffer_add_utf8(hb_buffer_t *buffer,
                                           const char *text, int text_length,
                                           unsigned int item_offset,
                                           int item_length);
void                  hb_buffer_add_utf16(hb_buffer_t *buffer,
                                            const uint16_t *text,
                                            int text_length,
                                            unsigned int item_offset,
                                            int item_length);
void                  hb_buffer_add_utf32(hb_buffer_t *buffer,
                                            const uint32_t *text,
                                            int text_length,
                                            unsigned int item_offset,
                                            int item_length);
void                  hb_buffer_add_latin1(hb_buffer_t *buffer,
                                             const uint8_t *text,
                                             int text_length,
                                             unsigned int item_offset,
                                             int item_length);
void                  hb_buffer_add_codepoints(hb_buffer_t *buffer,
                                                  const hb_codepoint_t *text,
                                                  int text_length,
                                                  unsigned int item_offset,
                                                  int item_length);
void                  hb_buffer_append(hb_buffer_t *buffer,
                                         const hb_buffer_t *source,
                                         unsigned int start,
                                         unsigned int end);

/* ========================================================================= */
/* Buffer properties                                                         */
/* ========================================================================= */

void                  hb_buffer_set_content_type(hb_buffer_t *buffer,
                                                    hb_buffer_content_type_t content_type);
hb_buffer_content_type_t hb_buffer_get_content_type(hb_buffer_t *buffer);

void                  hb_buffer_set_unicode_funcs(hb_buffer_t *buffer,
                                                     hb_unicode_funcs_t *unicode_funcs);
hb_unicode_funcs_t   *hb_buffer_get_unicode_funcs(hb_buffer_t *buffer);

void                  hb_buffer_set_direction(hb_buffer_t *buffer,
                                                 hb_direction_t direction);
hb_direction_t        hb_buffer_get_direction(hb_buffer_t *buffer);

void                  hb_buffer_set_script(hb_buffer_t *buffer,
                                              hb_script_t script);
hb_script_t           hb_buffer_get_script(hb_buffer_t *buffer);

void                  hb_buffer_set_language(hb_buffer_t *buffer,
                                               hb_language_t language);
hb_language_t         hb_buffer_get_language(hb_buffer_t *buffer);

void                  hb_buffer_set_segment_properties(hb_buffer_t *buffer,
                                                          const void *props);
void                  hb_buffer_get_segment_properties(hb_buffer_t *buffer,
                                                          void *props);

void                  hb_buffer_guess_segment_properties(hb_buffer_t *buffer);

void                  hb_buffer_set_flags(hb_buffer_t *buffer,
                                             hb_buffer_flags_t flags);
hb_buffer_flags_t     hb_buffer_get_flags(hb_buffer_t *buffer);

void                  hb_buffer_set_cluster_level(hb_buffer_t *buffer,
                                                     hb_buffer_cluster_level_t cluster_level);
hb_buffer_cluster_level_t hb_buffer_get_cluster_level(hb_buffer_t *buffer);

void                  hb_buffer_set_length(hb_buffer_t *buffer,
                                              unsigned int length);
unsigned int          hb_buffer_get_length(hb_buffer_t *buffer);

/* ========================================================================= */
/* Glyph access                                                              */
/* ========================================================================= */

hb_glyph_info_t     *hb_buffer_get_glyph_infos(hb_buffer_t *buffer,
                                                   unsigned int *length);
hb_glyph_position_t *hb_buffer_get_glyph_positions(hb_buffer_t *buffer,
                                                       unsigned int *length);

hb_bool_t             hb_buffer_has_positions(hb_buffer_t *buffer);

void                  hb_buffer_normalize_glyphs(hb_buffer_t *buffer);
void                  hb_buffer_reverse(hb_buffer_t *buffer);
void                  hb_buffer_reverse_range(hb_buffer_t *buffer,
                                                 unsigned int start,
                                                 unsigned int end);
void                  hb_buffer_reverse_clusters(hb_buffer_t *buffer);

/* ========================================================================= */
/* Serialization                                                             */
/* ========================================================================= */

unsigned int          hb_buffer_serialize_glyphs(hb_buffer_t *buffer,
                                                    unsigned int start,
                                                    unsigned int end,
                                                    char *buf,
                                                    unsigned int buf_size,
                                                    unsigned int *buf_consumed,
                                                    hb_font_t *font,
                                                    hb_buffer_serialize_format_t format,
                                                    hb_buffer_serialize_flags_t flags);
hb_bool_t             hb_buffer_deserialize_glyphs(hb_buffer_t *buffer,
                                                      const char *buf,
                                                      int buf_len,
                                                      const char **end_ptr,
                                                      hb_font_t *font,
                                                      hb_buffer_serialize_format_t format);

/* Diff */
hb_buffer_diff_flags_t hb_buffer_diff(hb_buffer_t *buffer,
                                         hb_buffer_t *reference,
                                         hb_codepoint_t dottedcircle_glyph,
                                         unsigned int position_fuzz);

/* Message */
typedef hb_bool_t (*hb_buffer_message_func_t)(hb_buffer_t *buffer,
                                                 hb_font_t *font,
                                                 const char *message,
                                                 void *user_data);
void                  hb_buffer_set_message_func(hb_buffer_t *buffer,
                                                    hb_buffer_message_func_t func,
                                                    void *user_data,
                                                    hb_destroy_func_t destroy);

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_BUFFER_H */

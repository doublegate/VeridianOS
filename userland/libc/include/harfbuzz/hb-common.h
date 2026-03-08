/*
 * VeridianOS libc -- harfbuzz/hb-common.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HarfBuzz common types and enumerations.
 */

#ifndef _HARFBUZZ_HB_COMMON_H
#define _HARFBUZZ_HB_COMMON_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Basic types                                                               */
/* ========================================================================= */

typedef int          hb_bool_t;
typedef uint32_t     hb_codepoint_t;
typedef int32_t      hb_position_t;
typedef uint32_t     hb_mask_t;
typedef uint32_t     hb_tag_t;

typedef union _hb_var_int_t {
    uint32_t u32;
    int32_t  i32;
    uint16_t u16[2];
    int16_t  i16[2];
    uint8_t  u8[4];
    int8_t   i8[4];
} hb_var_int_t;

typedef union _hb_var_num_t {
    float    f;
    uint32_t u32;
    int32_t  i32;
} hb_var_num_t;

/* ========================================================================= */
/* Tag creation                                                              */
/* ========================================================================= */

#define HB_TAG(c1,c2,c3,c4) ((hb_tag_t)(((uint32_t)(c1)<<24)|((uint32_t)(c2)<<16)|((uint32_t)(c3)<<8)|(uint32_t)(c4)))
#define HB_UNTAG(tag)        ((uint8_t)((tag)>>24)), ((uint8_t)((tag)>>16)), ((uint8_t)((tag)>>8)), ((uint8_t)(tag))
#define HB_TAG_NONE          HB_TAG(0,0,0,0)
#define HB_TAG_MAX           HB_TAG(0xFF,0xFF,0xFF,0xFF)
#define HB_TAG_MAX_SIGNED    HB_TAG(0x7F,0xFF,0xFF,0xFF)

/* ========================================================================= */
/* Direction                                                                 */
/* ========================================================================= */

typedef enum {
    HB_DIRECTION_INVALID = 0,
    HB_DIRECTION_LTR     = 4,
    HB_DIRECTION_RTL     = 5,
    HB_DIRECTION_TTB     = 6,
    HB_DIRECTION_BTT     = 7
} hb_direction_t;

#define HB_DIRECTION_IS_VALID(dir)      ((dir) >= HB_DIRECTION_LTR && (dir) <= HB_DIRECTION_BTT)
#define HB_DIRECTION_IS_HORIZONTAL(dir) ((dir) == HB_DIRECTION_LTR || (dir) == HB_DIRECTION_RTL)
#define HB_DIRECTION_IS_VERTICAL(dir)   ((dir) == HB_DIRECTION_TTB || (dir) == HB_DIRECTION_BTT)
#define HB_DIRECTION_IS_FORWARD(dir)    ((dir) == HB_DIRECTION_LTR || (dir) == HB_DIRECTION_TTB)
#define HB_DIRECTION_IS_BACKWARD(dir)   ((dir) == HB_DIRECTION_RTL || (dir) == HB_DIRECTION_BTT)
#define HB_DIRECTION_REVERSE(dir)       ((hb_direction_t)(((int)(dir)) ^ 1))

hb_direction_t hb_direction_from_string(const char *str, int len);
const char    *hb_direction_to_string(hb_direction_t direction);

/* ========================================================================= */
/* Script                                                                    */
/* ========================================================================= */

typedef enum {
    HB_SCRIPT_COMMON              = HB_TAG('Z','y','y','y'),
    HB_SCRIPT_INHERITED           = HB_TAG('Z','i','n','h'),
    HB_SCRIPT_UNKNOWN             = HB_TAG('Z','z','z','z'),
    HB_SCRIPT_ARABIC              = HB_TAG('A','r','a','b'),
    HB_SCRIPT_ARMENIAN            = HB_TAG('A','r','m','n'),
    HB_SCRIPT_BENGALI             = HB_TAG('B','e','n','g'),
    HB_SCRIPT_CYRILLIC            = HB_TAG('C','y','r','l'),
    HB_SCRIPT_DEVANAGARI          = HB_TAG('D','e','v','a'),
    HB_SCRIPT_GEORGIAN            = HB_TAG('G','e','o','r'),
    HB_SCRIPT_GREEK               = HB_TAG('G','r','e','k'),
    HB_SCRIPT_GUJARATI            = HB_TAG('G','u','j','r'),
    HB_SCRIPT_GURMUKHI            = HB_TAG('G','u','r','u'),
    HB_SCRIPT_HANGUL              = HB_TAG('H','a','n','g'),
    HB_SCRIPT_HAN                 = HB_TAG('H','a','n','i'),
    HB_SCRIPT_HEBREW              = HB_TAG('H','e','b','r'),
    HB_SCRIPT_HIRAGANA            = HB_TAG('H','i','r','a'),
    HB_SCRIPT_KANNADA             = HB_TAG('K','n','d','a'),
    HB_SCRIPT_KATAKANA            = HB_TAG('K','a','n','a'),
    HB_SCRIPT_LAO                 = HB_TAG('L','a','o','o'),
    HB_SCRIPT_LATIN               = HB_TAG('L','a','t','n'),
    HB_SCRIPT_MALAYALAM           = HB_TAG('M','l','y','m'),
    HB_SCRIPT_ORIYA               = HB_TAG('O','r','y','a'),
    HB_SCRIPT_TAMIL               = HB_TAG('T','a','m','l'),
    HB_SCRIPT_TELUGU              = HB_TAG('T','e','l','u'),
    HB_SCRIPT_THAI                = HB_TAG('T','h','a','i'),
    HB_SCRIPT_TIBETAN             = HB_TAG('T','i','b','t'),
    HB_SCRIPT_BOPOMOFO            = HB_TAG('B','o','p','o'),
    HB_SCRIPT_BRAILLE             = HB_TAG('B','r','a','i'),
    HB_SCRIPT_CANADIAN_SYLLABICS  = HB_TAG('C','a','n','s'),
    HB_SCRIPT_CHEROKEE            = HB_TAG('C','h','e','r'),
    HB_SCRIPT_ETHIOPIC            = HB_TAG('E','t','h','i'),
    HB_SCRIPT_KHMER               = HB_TAG('K','h','m','r'),
    HB_SCRIPT_MONGOLIAN           = HB_TAG('M','o','n','g'),
    HB_SCRIPT_MYANMAR             = HB_TAG('M','y','m','r'),
    HB_SCRIPT_OGHAM               = HB_TAG('O','g','a','m'),
    HB_SCRIPT_RUNIC               = HB_TAG('R','u','n','r'),
    HB_SCRIPT_SINHALA             = HB_TAG('S','i','n','h'),
    HB_SCRIPT_SYRIAC              = HB_TAG('S','y','r','c'),
    HB_SCRIPT_THAANA              = HB_TAG('T','h','a','a'),
    HB_SCRIPT_YI                  = HB_TAG('Y','i','i','i'),
    HB_SCRIPT_INVALID             = HB_TAG_NONE,
    _HB_SCRIPT_MAX_VALUE          = HB_TAG_MAX_SIGNED
} hb_script_t;

hb_script_t    hb_script_from_iso15924_tag(hb_tag_t tag);
hb_script_t    hb_script_from_string(const char *str, int len);
hb_tag_t       hb_script_to_iso15924_tag(hb_script_t script);
hb_direction_t hb_script_get_horizontal_direction(hb_script_t script);

/* ========================================================================= */
/* Language                                                                  */
/* ========================================================================= */

typedef const struct hb_language_impl_t *hb_language_t;

#define HB_LANGUAGE_INVALID ((hb_language_t)0)

hb_language_t  hb_language_from_string(const char *str, int len);
const char    *hb_language_to_string(hb_language_t language);
hb_language_t  hb_language_get_default(void);
hb_bool_t      hb_language_matches(hb_language_t language,
                                     hb_language_t specific);

/* ========================================================================= */
/* Feature                                                                   */
/* ========================================================================= */

typedef struct hb_feature_t {
    hb_tag_t      tag;
    uint32_t      value;
    unsigned int  start;
    unsigned int  end;
} hb_feature_t;

#define HB_FEATURE_GLOBAL_START  0
#define HB_FEATURE_GLOBAL_END   ((unsigned int)-1)

hb_bool_t  hb_feature_from_string(const char *str, int len,
                                    hb_feature_t *feature);
void       hb_feature_to_string(hb_feature_t *feature, char *buf,
                                  unsigned int size);

/* ========================================================================= */
/* Variation                                                                 */
/* ========================================================================= */

typedef struct hb_variation_t {
    hb_tag_t  tag;
    float     value;
} hb_variation_t;

hb_bool_t  hb_variation_from_string(const char *str, int len,
                                      hb_variation_t *variation);
void       hb_variation_to_string(hb_variation_t *variation, char *buf,
                                    unsigned int size);

/* ========================================================================= */
/* Destroy / reference callbacks                                             */
/* ========================================================================= */

typedef void (*hb_destroy_func_t)(void *user_data);
typedef hb_bool_t (*hb_user_data_key_t);

/* ========================================================================= */
/* Memory mode                                                               */
/* ========================================================================= */

typedef enum {
    HB_MEMORY_MODE_DUPLICATE,
    HB_MEMORY_MODE_READONLY,
    HB_MEMORY_MODE_WRITABLE,
    HB_MEMORY_MODE_READONLY_MAY_MAKE_WRITABLE
} hb_memory_mode_t;

#ifdef __cplusplus
}
#endif

#endif /* _HARFBUZZ_HB_COMMON_H */

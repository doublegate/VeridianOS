/*
 * VeridianOS libc -- pcre2.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * PCRE2 10.43 compatible API (8-bit code unit width).
 * Perl Compatible Regular Expressions library.
 */

#ifndef _PCRE2_H
#define _PCRE2_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Version                                                                   */
/* ========================================================================= */

#define PCRE2_MAJOR     10
#define PCRE2_MINOR     43
#define PCRE2_DATE      "2024-06-07"

/* ========================================================================= */
/* Code unit width                                                           */
/* ========================================================================= */

#ifndef PCRE2_CODE_UNIT_WIDTH
#define PCRE2_CODE_UNIT_WIDTH 8
#endif

typedef unsigned char PCRE2_UCHAR8;
typedef const unsigned char *PCRE2_SPTR8;
typedef size_t PCRE2_SIZE;

/* Use 8-bit types by default */
typedef PCRE2_UCHAR8 PCRE2_UCHAR;
typedef PCRE2_SPTR8  PCRE2_SPTR;

#define PCRE2_ZERO_TERMINATED (~(PCRE2_SIZE)0)
#define PCRE2_UNSET           (~(PCRE2_SIZE)0)

/* ========================================================================= */
/* Error codes                                                               */
/* ========================================================================= */

#define PCRE2_ERROR_NOMATCH         (-1)
#define PCRE2_ERROR_PARTIAL         (-2)
#define PCRE2_ERROR_BADDATA         (-29)
#define PCRE2_ERROR_NOMEMORY        (-48)
#define PCRE2_ERROR_BADOPTION       (-45)
#define PCRE2_ERROR_BADOFFSET       (-33)
#define PCRE2_ERROR_NULL            (-51)
#define PCRE2_ERROR_INTERNAL        (-44)

/* Compile error codes */
#define PCRE2_ERROR_END_BACKSLASH          101
#define PCRE2_ERROR_MISSING_SQUARE_BRACKET 106
#define PCRE2_ERROR_MISSING_CLOSING_PARENTHESIS 114
#define PCRE2_ERROR_QUANTIFIER_OUT_OF_ORDER 104
#define PCRE2_ERROR_BAD_ESCAPE_SEQUENCE    103

/* ========================================================================= */
/* Compile options                                                           */
/* ========================================================================= */

#define PCRE2_ANCHORED          0x80000000u
#define PCRE2_CASELESS          0x00000008u
#define PCRE2_DOTALL            0x00000020u
#define PCRE2_EXTENDED          0x00000080u
#define PCRE2_MULTILINE         0x00000400u
#define PCRE2_NO_AUTO_CAPTURE   0x00000800u
#define PCRE2_UNGREEDY          0x00040000u
#define PCRE2_UTF               0x00080000u
#define PCRE2_UCP               0x00020000u
#define PCRE2_NO_UTF_CHECK      0x40000000u

/* ========================================================================= */
/* Match options                                                             */
/* ========================================================================= */

#define PCRE2_NOTBOL            0x00000001u
#define PCRE2_NOTEOL            0x00000002u
#define PCRE2_NOTEMPTY          0x00000004u
#define PCRE2_PARTIAL_SOFT      0x00000010u
#define PCRE2_PARTIAL_HARD      0x00000020u

/* ========================================================================= */
/* Info codes                                                                */
/* ========================================================================= */

#define PCRE2_INFO_CAPTURECOUNT  4
#define PCRE2_INFO_NAMECOUNT     16
#define PCRE2_INFO_NAMEENTRYSIZE 17
#define PCRE2_INFO_NAMETABLE     18
#define PCRE2_INFO_SIZE          21

/* ========================================================================= */
/* Opaque types                                                              */
/* ========================================================================= */

typedef struct pcre2_real_code_8         pcre2_code_8;
typedef struct pcre2_real_match_data_8   pcre2_match_data_8;
typedef struct pcre2_real_compile_context_8 pcre2_compile_context_8;
typedef struct pcre2_real_match_context_8   pcre2_match_context_8;
typedef struct pcre2_real_general_context_8 pcre2_general_context_8;

/* Default width aliases */
typedef pcre2_code_8             pcre2_code;
typedef pcre2_match_data_8       pcre2_match_data;
typedef pcre2_compile_context_8  pcre2_compile_context;
typedef pcre2_match_context_8    pcre2_match_context;
typedef pcre2_general_context_8  pcre2_general_context;

/* ========================================================================= */
/* API functions (8-bit)                                                     */
/* ========================================================================= */

/* Compile a pattern */
pcre2_code *pcre2_compile_8(PCRE2_SPTR pattern, PCRE2_SIZE length,
                            uint32_t options, int *errorcode,
                            PCRE2_SIZE *erroroffset,
                            pcre2_compile_context *ccontext);

/* Free compiled code */
void pcre2_code_free_8(pcre2_code *code);

/* Create match data from pattern */
pcre2_match_data *pcre2_match_data_create_from_pattern_8(
    const pcre2_code *code, pcre2_general_context *gcontext);

/* Create match data with explicit size */
pcre2_match_data *pcre2_match_data_create_8(
    uint32_t ovecsize, pcre2_general_context *gcontext);

/* Free match data */
void pcre2_match_data_free_8(pcre2_match_data *match_data);

/* Execute a match */
int pcre2_match_8(const pcre2_code *code, PCRE2_SPTR subject,
                  PCRE2_SIZE length, PCRE2_SIZE startoffset,
                  uint32_t options, pcre2_match_data *match_data,
                  pcre2_match_context *mcontext);

/* Get the ovector pointer from match data */
PCRE2_SIZE *pcre2_get_ovector_pointer_8(pcre2_match_data *match_data);

/* Get ovector count */
uint32_t pcre2_get_ovector_count_8(pcre2_match_data *match_data);

/* Get error message */
int pcre2_get_error_message_8(int errorcode, PCRE2_UCHAR *buffer,
                              PCRE2_SIZE bufflen);

/* Pattern info */
int pcre2_pattern_info_8(const pcre2_code *code, uint32_t what,
                         void *where);

/* Substitute (replace) */
int pcre2_substitute_8(const pcre2_code *code, PCRE2_SPTR subject,
                       PCRE2_SIZE length, PCRE2_SIZE startoffset,
                       uint32_t options, pcre2_match_data *match_data,
                       pcre2_match_context *mcontext,
                       PCRE2_SPTR replacement, PCRE2_SIZE rlength,
                       PCRE2_UCHAR *outputbuffer,
                       PCRE2_SIZE *outlengthptr);

/* Default-width aliases (macros) */
#define pcre2_compile                   pcre2_compile_8
#define pcre2_code_free                 pcre2_code_free_8
#define pcre2_match_data_create_from_pattern pcre2_match_data_create_from_pattern_8
#define pcre2_match_data_create         pcre2_match_data_create_8
#define pcre2_match_data_free           pcre2_match_data_free_8
#define pcre2_match                     pcre2_match_8
#define pcre2_get_ovector_pointer       pcre2_get_ovector_pointer_8
#define pcre2_get_ovector_count         pcre2_get_ovector_count_8
#define pcre2_get_error_message         pcre2_get_error_message_8
#define pcre2_pattern_info              pcre2_pattern_info_8
#define pcre2_substitute                pcre2_substitute_8

#ifdef __cplusplus
}
#endif

#endif /* _PCRE2_H */

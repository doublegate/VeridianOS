/*
 * VeridianOS libc -- pcre2_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * PCRE2 10.43 shim using POSIX regex as backend.
 * Translates PCRE2 API calls to POSIX BRE/ERE regex calls.
 * Sufficient for Qt 6 QRegularExpression basic usage.
 */

#include <pcre2.h>
#include <regex.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Internal structures                                                       */
/* ========================================================================= */

#define MAX_CAPTURES 64

struct pcre2_real_code_8 {
    regex_t     posix_re;
    uint32_t    options;
    int         capture_count;
    int         compiled;
};

struct pcre2_real_match_data_8 {
    PCRE2_SIZE  ovector[MAX_CAPTURES * 2];
    uint32_t    ovec_count;
    uint32_t    match_count;
};

/* ========================================================================= */
/* Compile                                                                   */
/* ========================================================================= */

pcre2_code *pcre2_compile_8(PCRE2_SPTR pattern, PCRE2_SIZE length,
                            uint32_t options, int *errorcode,
                            PCRE2_SIZE *erroroffset,
                            pcre2_compile_context *ccontext)
{
    pcre2_code *code;
    char *pat_str;
    int cflags = REG_EXTENDED;
    int ret;

    (void)ccontext;

    if (pattern == NULL) {
        if (errorcode) *errorcode = PCRE2_ERROR_NULL;
        return NULL;
    }

    code = (pcre2_code *)calloc(1, sizeof(*code));
    if (code == NULL) {
        if (errorcode) *errorcode = PCRE2_ERROR_NOMEMORY;
        return NULL;
    }

    if (length == PCRE2_ZERO_TERMINATED)
        length = strlen((const char *)pattern);

    pat_str = (char *)malloc(length + 1);
    if (pat_str == NULL) {
        free(code);
        if (errorcode) *errorcode = PCRE2_ERROR_NOMEMORY;
        return NULL;
    }
    memcpy(pat_str, pattern, length);
    pat_str[length] = '\0';

    if (options & PCRE2_CASELESS)
        cflags |= REG_ICASE;
    if (options & PCRE2_MULTILINE)
        cflags |= REG_NEWLINE;

    code->options = options;

    ret = regcomp(&code->posix_re, pat_str, cflags);
    free(pat_str);

    if (ret != 0) {
        if (errorcode) *errorcode = PCRE2_ERROR_BAD_ESCAPE_SEQUENCE;
        if (erroroffset) *erroroffset = 0;
        free(code);
        return NULL;
    }

    code->compiled = 1;
    code->capture_count = (int)code->posix_re.re_nsub;

    if (errorcode) *errorcode = 0;
    if (erroroffset) *erroroffset = 0;

    return code;
}

void pcre2_code_free_8(pcre2_code *code)
{
    if (code == NULL)
        return;
    if (code->compiled)
        regfree(&code->posix_re);
    free(code);
}

/* ========================================================================= */
/* Match data                                                                */
/* ========================================================================= */

pcre2_match_data *pcre2_match_data_create_from_pattern_8(
    const pcre2_code *code, pcre2_general_context *gcontext)
{
    pcre2_match_data *md;
    (void)gcontext;

    md = (pcre2_match_data *)calloc(1, sizeof(*md));
    if (md == NULL)
        return NULL;

    md->ovec_count = (code != NULL) ? (uint32_t)(code->capture_count + 1)
                                    : MAX_CAPTURES;
    if (md->ovec_count > MAX_CAPTURES)
        md->ovec_count = MAX_CAPTURES;

    return md;
}

pcre2_match_data *pcre2_match_data_create_8(
    uint32_t ovecsize, pcre2_general_context *gcontext)
{
    pcre2_match_data *md;
    (void)gcontext;

    md = (pcre2_match_data *)calloc(1, sizeof(*md));
    if (md == NULL)
        return NULL;

    md->ovec_count = ovecsize;
    if (md->ovec_count > MAX_CAPTURES)
        md->ovec_count = MAX_CAPTURES;

    return md;
}

void pcre2_match_data_free_8(pcre2_match_data *match_data)
{
    free(match_data);
}

/* ========================================================================= */
/* Match execution                                                           */
/* ========================================================================= */

int pcre2_match_8(const pcre2_code *code, PCRE2_SPTR subject,
                  PCRE2_SIZE length, PCRE2_SIZE startoffset,
                  uint32_t options, pcre2_match_data *match_data,
                  pcre2_match_context *mcontext)
{
    regmatch_t pmatch[MAX_CAPTURES];
    int eflags = 0;
    int ret;
    uint32_t i;
    char *subj_str;
    PCRE2_SIZE slen;

    (void)mcontext;

    if (code == NULL || subject == NULL || match_data == NULL)
        return PCRE2_ERROR_NULL;

    if (!code->compiled)
        return PCRE2_ERROR_INTERNAL;

    if (options & PCRE2_NOTBOL)
        eflags |= REG_NOTBOL;
    if (options & PCRE2_NOTEOL)
        eflags |= REG_NOTEOL;

    if (length == PCRE2_ZERO_TERMINATED)
        slen = strlen((const char *)subject);
    else
        slen = length;

    if (startoffset > slen)
        return PCRE2_ERROR_BADOFFSET;

    subj_str = (char *)malloc(slen - startoffset + 1);
    if (subj_str == NULL)
        return PCRE2_ERROR_NOMEMORY;

    memcpy(subj_str, subject + startoffset, slen - startoffset);
    subj_str[slen - startoffset] = '\0';

    memset(pmatch, 0, sizeof(pmatch));

    ret = regexec(&((pcre2_code *)code)->posix_re, subj_str,
                  match_data->ovec_count, pmatch, eflags);

    if (ret == REG_NOMATCH) {
        free(subj_str);
        return PCRE2_ERROR_NOMATCH;
    }

    if (ret != 0) {
        free(subj_str);
        return PCRE2_ERROR_INTERNAL;
    }

    match_data->match_count = 0;
    for (i = 0; i < match_data->ovec_count; i++) {
        if (pmatch[i].rm_so == -1) {
            match_data->ovector[i * 2] = PCRE2_UNSET;
            match_data->ovector[i * 2 + 1] = PCRE2_UNSET;
        } else {
            match_data->ovector[i * 2] =
                (PCRE2_SIZE)pmatch[i].rm_so + startoffset;
            match_data->ovector[i * 2 + 1] =
                (PCRE2_SIZE)pmatch[i].rm_eo + startoffset;
            match_data->match_count = i + 1;
        }
    }

    free(subj_str);
    return (int)match_data->match_count;
}

/* ========================================================================= */
/* Result access                                                             */
/* ========================================================================= */

PCRE2_SIZE *pcre2_get_ovector_pointer_8(pcre2_match_data *match_data)
{
    if (match_data == NULL)
        return NULL;
    return match_data->ovector;
}

uint32_t pcre2_get_ovector_count_8(pcre2_match_data *match_data)
{
    if (match_data == NULL)
        return 0;
    return match_data->ovec_count;
}

/* ========================================================================= */
/* Error messages                                                            */
/* ========================================================================= */

int pcre2_get_error_message_8(int errorcode, PCRE2_UCHAR *buffer,
                              PCRE2_SIZE bufflen)
{
    const char *msg;

    switch (errorcode) {
    case 0:                    msg = "no error"; break;
    case PCRE2_ERROR_NOMATCH:  msg = "no match"; break;
    case PCRE2_ERROR_PARTIAL:  msg = "partial match"; break;
    case PCRE2_ERROR_NOMEMORY: msg = "out of memory"; break;
    case PCRE2_ERROR_NULL:     msg = "null argument"; break;
    case PCRE2_ERROR_BADOFFSET: msg = "bad offset"; break;
    case PCRE2_ERROR_BADOPTION: msg = "bad option"; break;
    case PCRE2_ERROR_INTERNAL: msg = "internal error"; break;
    default:                   msg = "unknown error"; break;
    }

    if (buffer == NULL || bufflen == 0)
        return PCRE2_ERROR_NOMEMORY;

    {
        size_t len = strlen(msg);
        if (len >= bufflen)
            len = bufflen - 1;
        memcpy(buffer, msg, len);
        buffer[len] = '\0';
        return (int)len;
    }
}

/* ========================================================================= */
/* Pattern info                                                              */
/* ========================================================================= */

int pcre2_pattern_info_8(const pcre2_code *code, uint32_t what,
                         void *where)
{
    if (code == NULL || where == NULL)
        return PCRE2_ERROR_NULL;

    switch (what) {
    case PCRE2_INFO_CAPTURECOUNT:
        *(uint32_t *)where = (uint32_t)code->capture_count;
        return 0;
    case PCRE2_INFO_SIZE:
        *(PCRE2_SIZE *)where = sizeof(*code);
        return 0;
    case PCRE2_INFO_NAMECOUNT:
        *(uint32_t *)where = 0;
        return 0;
    default:
        return PCRE2_ERROR_BADOPTION;
    }
}

/* ========================================================================= */
/* Substitute (stub)                                                         */
/* ========================================================================= */

int pcre2_substitute_8(const pcre2_code *code, PCRE2_SPTR subject,
                       PCRE2_SIZE length, PCRE2_SIZE startoffset,
                       uint32_t options, pcre2_match_data *match_data,
                       pcre2_match_context *mcontext,
                       PCRE2_SPTR replacement, PCRE2_SIZE rlength,
                       PCRE2_UCHAR *outputbuffer,
                       PCRE2_SIZE *outlengthptr)
{
    (void)code; (void)subject; (void)length; (void)startoffset;
    (void)options; (void)match_data; (void)mcontext;
    (void)replacement; (void)rlength; (void)outputbuffer;
    (void)outlengthptr;
    return PCRE2_ERROR_INTERNAL;
}

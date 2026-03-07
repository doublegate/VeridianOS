/*
 * VeridianOS libc -- iconv.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Minimal iconv implementation for Qt 6 / KDE Plasma porting.
 * Supports conversions between:
 *   - UTF-8
 *   - UTF-16 (LE/BE, with BOM detection for plain "UTF-16")
 *   - UTF-32 (LE/BE, with BOM detection for plain "UTF-32")
 *   - ISO-8859-1 / LATIN1
 *   - ASCII / US-ASCII
 *
 * This is NOT a full glibc iconv -- just enough for Qt's text codec
 * bootstrap which primarily needs UTF-8 <-> UTF-16 and UTF-8 <-> Latin-1.
 */

#include <iconv.h>
#include <errno.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Internal encoding IDs                                                     */
/* ========================================================================= */

#define ENC_UNKNOWN     0
#define ENC_UTF8        1
#define ENC_UTF16LE     2
#define ENC_UTF16BE     3
#define ENC_UTF16       4   /* Native byte order or BOM-detected */
#define ENC_UTF32LE     5
#define ENC_UTF32BE     6
#define ENC_UTF32       7   /* Native byte order or BOM-detected */
#define ENC_LATIN1      8   /* ISO-8859-1 */
#define ENC_ASCII       9

/* Conversion descriptor (heap-allocated, returned as iconv_t) */
struct __iconv_desc {
    int from;
    int to;
};

/* ========================================================================= */
/* Helpers                                                                   */
/* ========================================================================= */

/*
 * Case-insensitive comparison for ASCII encoding names.
 */
static int __enc_casecmp(const char *a, const char *b)
{
    while (*a && *b) {
        char ca = *a, cb = *b;
        if (ca >= 'a' && ca <= 'z') ca -= 32;
        if (cb >= 'a' && cb <= 'z') cb -= 32;
        /* Skip hyphens and underscores for fuzzy matching */
        if (ca == '_') ca = '-';
        if (cb == '_') cb = '-';
        if (ca != cb) return 1;
        a++; b++;
    }
    return *a != *b;
}

/*
 * Map an encoding name string to an internal ID.
 */
static int __enc_lookup(const char *name)
{
    if (!name) return ENC_UNKNOWN;

    /* Strip "//IGNORE" and "//TRANSLIT" suffixes if present */
    /* We just match the prefix before "//" */
    char buf[64];
    size_t len = 0;
    while (name[len] && name[len] != '/' && len < sizeof(buf) - 1) {
        buf[len] = name[len];
        len++;
    }
    buf[len] = '\0';

    if (__enc_casecmp(buf, "UTF-8") == 0 ||
        __enc_casecmp(buf, "UTF8") == 0)
        return ENC_UTF8;

    if (__enc_casecmp(buf, "UTF-16LE") == 0 ||
        __enc_casecmp(buf, "UTF16LE") == 0)
        return ENC_UTF16LE;

    if (__enc_casecmp(buf, "UTF-16BE") == 0 ||
        __enc_casecmp(buf, "UTF16BE") == 0)
        return ENC_UTF16BE;

    if (__enc_casecmp(buf, "UTF-16") == 0 ||
        __enc_casecmp(buf, "UTF16") == 0 ||
        __enc_casecmp(buf, "UCS-2") == 0 ||
        __enc_casecmp(buf, "UCS2") == 0)
        return ENC_UTF16;

    if (__enc_casecmp(buf, "UTF-32LE") == 0 ||
        __enc_casecmp(buf, "UTF32LE") == 0)
        return ENC_UTF32LE;

    if (__enc_casecmp(buf, "UTF-32BE") == 0 ||
        __enc_casecmp(buf, "UTF32BE") == 0)
        return ENC_UTF32BE;

    if (__enc_casecmp(buf, "UTF-32") == 0 ||
        __enc_casecmp(buf, "UTF32") == 0 ||
        __enc_casecmp(buf, "UCS-4") == 0 ||
        __enc_casecmp(buf, "UCS4") == 0)
        return ENC_UTF32;

    if (__enc_casecmp(buf, "ISO-8859-1") == 0 ||
        __enc_casecmp(buf, "ISO8859-1") == 0 ||
        __enc_casecmp(buf, "LATIN1") == 0 ||
        __enc_casecmp(buf, "LATIN-1") == 0 ||
        __enc_casecmp(buf, "ISO88591") == 0)
        return ENC_LATIN1;

    if (__enc_casecmp(buf, "ASCII") == 0 ||
        __enc_casecmp(buf, "US-ASCII") == 0 ||
        __enc_casecmp(buf, "ANSI_X3.4-1968") == 0)
        return ENC_ASCII;

    return ENC_UNKNOWN;
}

/*
 * Decode one Unicode code point from the input buffer.
 * Returns the code point (0..0x10FFFF), or (uint32_t)-1 on error.
 * Advances *in and decrements *inleft.
 */
static uint32_t __decode_codepoint(int enc, const unsigned char **in,
                                   size_t *inleft)
{
    const unsigned char *p = *in;
    uint32_t cp;

    switch (enc) {
    case ENC_UTF8: {
        if (*inleft < 1) { errno = EINVAL; return (uint32_t)-1; }
        unsigned char c = p[0];
        if (c < 0x80) {
            cp = c;
            *in += 1; *inleft -= 1;
        } else if ((c & 0xE0) == 0xC0) {
            if (*inleft < 2) { errno = EINVAL; return (uint32_t)-1; }
            if ((p[1] & 0xC0) != 0x80) { errno = EILSEQ; return (uint32_t)-1; }
            cp = ((uint32_t)(c & 0x1F) << 6) | (p[1] & 0x3F);
            if (cp < 0x80) { errno = EILSEQ; return (uint32_t)-1; }
            *in += 2; *inleft -= 2;
        } else if ((c & 0xF0) == 0xE0) {
            if (*inleft < 3) { errno = EINVAL; return (uint32_t)-1; }
            if ((p[1] & 0xC0) != 0x80 || (p[2] & 0xC0) != 0x80)
                { errno = EILSEQ; return (uint32_t)-1; }
            cp = ((uint32_t)(c & 0x0F) << 12) |
                 ((uint32_t)(p[1] & 0x3F) << 6) |
                 (p[2] & 0x3F);
            if (cp < 0x800) { errno = EILSEQ; return (uint32_t)-1; }
            *in += 3; *inleft -= 3;
        } else if ((c & 0xF8) == 0xF0) {
            if (*inleft < 4) { errno = EINVAL; return (uint32_t)-1; }
            if ((p[1] & 0xC0) != 0x80 || (p[2] & 0xC0) != 0x80 ||
                (p[3] & 0xC0) != 0x80)
                { errno = EILSEQ; return (uint32_t)-1; }
            cp = ((uint32_t)(c & 0x07) << 18) |
                 ((uint32_t)(p[1] & 0x3F) << 12) |
                 ((uint32_t)(p[2] & 0x3F) << 6) |
                 (p[3] & 0x3F);
            if (cp < 0x10000 || cp > 0x10FFFF)
                { errno = EILSEQ; return (uint32_t)-1; }
            *in += 4; *inleft -= 4;
        } else {
            errno = EILSEQ; return (uint32_t)-1;
        }
        return cp;
    }

    case ENC_UTF16LE:
    case ENC_UTF16:
    case ENC_UTF16BE: {
        if (*inleft < 2) { errno = EINVAL; return (uint32_t)-1; }
        uint16_t w;
        if (enc == ENC_UTF16BE) {
            w = ((uint16_t)p[0] << 8) | p[1];
        } else {
            w = p[0] | ((uint16_t)p[1] << 8);
        }
        if (w >= 0xD800 && w <= 0xDBFF) {
            /* High surrogate -- need low surrogate */
            if (*inleft < 4) { errno = EINVAL; return (uint32_t)-1; }
            uint16_t w2;
            if (enc == ENC_UTF16BE) {
                w2 = ((uint16_t)p[2] << 8) | p[3];
            } else {
                w2 = p[2] | ((uint16_t)p[3] << 8);
            }
            if (w2 < 0xDC00 || w2 > 0xDFFF)
                { errno = EILSEQ; return (uint32_t)-1; }
            cp = 0x10000 + ((uint32_t)(w - 0xD800) << 10) + (w2 - 0xDC00);
            *in += 4; *inleft -= 4;
        } else if (w >= 0xDC00 && w <= 0xDFFF) {
            errno = EILSEQ; return (uint32_t)-1;
        } else {
            cp = w;
            *in += 2; *inleft -= 2;
        }
        return cp;
    }

    case ENC_UTF32LE:
    case ENC_UTF32:
    case ENC_UTF32BE: {
        if (*inleft < 4) { errno = EINVAL; return (uint32_t)-1; }
        if (enc == ENC_UTF32BE) {
            cp = ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
                 ((uint32_t)p[2] << 8) | p[3];
        } else {
            cp = p[0] | ((uint32_t)p[1] << 8) |
                 ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
        }
        if (cp > 0x10FFFF || (cp >= 0xD800 && cp <= 0xDFFF))
            { errno = EILSEQ; return (uint32_t)-1; }
        *in += 4; *inleft -= 4;
        return cp;
    }

    case ENC_LATIN1:
        if (*inleft < 1) { errno = EINVAL; return (uint32_t)-1; }
        cp = p[0];  /* 0x00..0xFF maps directly to Unicode */
        *in += 1; *inleft -= 1;
        return cp;

    case ENC_ASCII:
        if (*inleft < 1) { errno = EINVAL; return (uint32_t)-1; }
        if (p[0] > 0x7F) { errno = EILSEQ; return (uint32_t)-1; }
        cp = p[0];
        *in += 1; *inleft -= 1;
        return cp;

    default:
        errno = EINVAL;
        return (uint32_t)-1;
    }
}

/*
 * Encode one Unicode code point into the output buffer.
 * Returns 0 on success, -1 on error (E2BIG if output buffer full,
 * EILSEQ if code point cannot be represented).
 */
static int __encode_codepoint(int enc, uint32_t cp,
                              unsigned char **out, size_t *outleft)
{
    unsigned char *p = *out;

    switch (enc) {
    case ENC_UTF8:
        if (cp < 0x80) {
            if (*outleft < 1) { errno = E2BIG; return -1; }
            p[0] = (unsigned char)cp;
            *out += 1; *outleft -= 1;
        } else if (cp < 0x800) {
            if (*outleft < 2) { errno = E2BIG; return -1; }
            p[0] = 0xC0 | (unsigned char)(cp >> 6);
            p[1] = 0x80 | (unsigned char)(cp & 0x3F);
            *out += 2; *outleft -= 2;
        } else if (cp < 0x10000) {
            if (*outleft < 3) { errno = E2BIG; return -1; }
            p[0] = 0xE0 | (unsigned char)(cp >> 12);
            p[1] = 0x80 | (unsigned char)((cp >> 6) & 0x3F);
            p[2] = 0x80 | (unsigned char)(cp & 0x3F);
            *out += 3; *outleft -= 3;
        } else if (cp <= 0x10FFFF) {
            if (*outleft < 4) { errno = E2BIG; return -1; }
            p[0] = 0xF0 | (unsigned char)(cp >> 18);
            p[1] = 0x80 | (unsigned char)((cp >> 12) & 0x3F);
            p[2] = 0x80 | (unsigned char)((cp >> 6) & 0x3F);
            p[3] = 0x80 | (unsigned char)(cp & 0x3F);
            *out += 4; *outleft -= 4;
        } else {
            errno = EILSEQ; return -1;
        }
        return 0;

    case ENC_UTF16LE:
    case ENC_UTF16:
        if (cp < 0x10000) {
            if (*outleft < 2) { errno = E2BIG; return -1; }
            p[0] = (unsigned char)(cp & 0xFF);
            p[1] = (unsigned char)(cp >> 8);
            *out += 2; *outleft -= 2;
        } else if (cp <= 0x10FFFF) {
            if (*outleft < 4) { errno = E2BIG; return -1; }
            uint32_t v = cp - 0x10000;
            uint16_t hi = 0xD800 + (uint16_t)(v >> 10);
            uint16_t lo = 0xDC00 + (uint16_t)(v & 0x3FF);
            p[0] = (unsigned char)(hi & 0xFF);
            p[1] = (unsigned char)(hi >> 8);
            p[2] = (unsigned char)(lo & 0xFF);
            p[3] = (unsigned char)(lo >> 8);
            *out += 4; *outleft -= 4;
        } else {
            errno = EILSEQ; return -1;
        }
        return 0;

    case ENC_UTF16BE:
        if (cp < 0x10000) {
            if (*outleft < 2) { errno = E2BIG; return -1; }
            p[0] = (unsigned char)(cp >> 8);
            p[1] = (unsigned char)(cp & 0xFF);
            *out += 2; *outleft -= 2;
        } else if (cp <= 0x10FFFF) {
            if (*outleft < 4) { errno = E2BIG; return -1; }
            uint32_t v = cp - 0x10000;
            uint16_t hi = 0xD800 + (uint16_t)(v >> 10);
            uint16_t lo = 0xDC00 + (uint16_t)(v & 0x3FF);
            p[0] = (unsigned char)(hi >> 8);
            p[1] = (unsigned char)(hi & 0xFF);
            p[2] = (unsigned char)(lo >> 8);
            p[3] = (unsigned char)(lo & 0xFF);
            *out += 4; *outleft -= 4;
        } else {
            errno = EILSEQ; return -1;
        }
        return 0;

    case ENC_UTF32LE:
    case ENC_UTF32:
        if (*outleft < 4) { errno = E2BIG; return -1; }
        p[0] = (unsigned char)(cp & 0xFF);
        p[1] = (unsigned char)((cp >> 8) & 0xFF);
        p[2] = (unsigned char)((cp >> 16) & 0xFF);
        p[3] = (unsigned char)((cp >> 24) & 0xFF);
        *out += 4; *outleft -= 4;
        return 0;

    case ENC_UTF32BE:
        if (*outleft < 4) { errno = E2BIG; return -1; }
        p[0] = (unsigned char)((cp >> 24) & 0xFF);
        p[1] = (unsigned char)((cp >> 16) & 0xFF);
        p[2] = (unsigned char)((cp >> 8) & 0xFF);
        p[3] = (unsigned char)(cp & 0xFF);
        *out += 4; *outleft -= 4;
        return 0;

    case ENC_LATIN1:
        if (cp > 0xFF) { errno = EILSEQ; return -1; }
        if (*outleft < 1) { errno = E2BIG; return -1; }
        p[0] = (unsigned char)cp;
        *out += 1; *outleft -= 1;
        return 0;

    case ENC_ASCII:
        if (cp > 0x7F) { errno = EILSEQ; return -1; }
        if (*outleft < 1) { errno = E2BIG; return -1; }
        p[0] = (unsigned char)cp;
        *out += 1; *outleft -= 1;
        return 0;

    default:
        errno = EILSEQ;
        return -1;
    }
}

/* ========================================================================= */
/* Public API                                                                */
/* ========================================================================= */

iconv_t iconv_open(const char *tocode, const char *fromcode)
{
    int from = __enc_lookup(fromcode);
    int to   = __enc_lookup(tocode);

    if (from == ENC_UNKNOWN || to == ENC_UNKNOWN) {
        errno = EINVAL;
        return (iconv_t)-1;
    }

    struct __iconv_desc *cd = (struct __iconv_desc *)malloc(
        sizeof(struct __iconv_desc));
    if (!cd) {
        errno = ENOMEM;
        return (iconv_t)-1;
    }

    cd->from = from;
    cd->to   = to;
    return (iconv_t)cd;
}

size_t iconv(iconv_t cd_opaque, char **inbuf, size_t *inbytesleft,
             char **outbuf, size_t *outbytesleft)
{
    struct __iconv_desc *cd = (struct __iconv_desc *)cd_opaque;

    if (!cd || cd == (struct __iconv_desc *)(iconv_t)-1) {
        errno = EBADF;
        return (size_t)-1;
    }

    /* NULL inbuf: reset shift state (no shift states in our encodings) */
    if (!inbuf || !*inbuf) {
        return 0;
    }

    size_t nonreversible = 0;
    const unsigned char *in  = (const unsigned char *)*inbuf;
    unsigned char *out       = (unsigned char *)*outbuf;
    size_t inleft  = *inbytesleft;
    size_t outleft = *outbytesleft;

    while (inleft > 0) {
        /* Save position in case we need to back up on E2BIG */
        const unsigned char *save_in = in;
        size_t save_inleft = inleft;

        uint32_t cp = __decode_codepoint(cd->from, &in, &inleft);
        if (cp == (uint32_t)-1) {
            /* Decode error -- errno already set */
            *inbuf        = (char *)in;
            *outbuf       = (char *)out;
            *inbytesleft  = inleft;
            *outbytesleft = outleft;
            return (size_t)-1;
        }

        int rc = __encode_codepoint(cd->to, cp, &out, &outleft);
        if (rc < 0) {
            if (errno == E2BIG) {
                /* Restore input position -- we consumed but couldn't write */
                in = save_in;
                inleft = save_inleft;
            }
            *inbuf        = (char *)in;
            *outbuf       = (char *)out;
            *inbytesleft  = inleft;
            *outbytesleft = outleft;
            return (size_t)-1;
        }
    }

    *inbuf        = (char *)in;
    *outbuf       = (char *)out;
    *inbytesleft  = inleft;
    *outbytesleft = outleft;
    return nonreversible;
}

int iconv_close(iconv_t cd_opaque)
{
    struct __iconv_desc *cd = (struct __iconv_desc *)cd_opaque;
    if (!cd || cd == (struct __iconv_desc *)(iconv_t)-1) {
        errno = EBADF;
        return -1;
    }
    free(cd);
    return 0;
}

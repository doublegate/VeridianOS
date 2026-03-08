/*
 * VeridianOS libc -- icu_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * ICU 75.x shim -- Unicode character properties, string operations,
 * normalization (NFC passthrough), collation, break iteration, and
 * charset conversion.  Covers ASCII + Latin-1 range with a small
 * built-in property table.  Sufficient for Qt 6 basic Unicode support.
 */

#include <unicode/utypes.h>
#include <unicode/uchar.h>
#include <unicode/ustring.h>
#include <unicode/ucnv.h>
#include <unicode/unorm2.h>
#include <unicode/ucol.h>
#include <unicode/ubrk.h>
#include <stdlib.h>
#include <string.h>

/* ========================================================================= */
/* Character properties (ASCII + Latin-1)                                    */
/* ========================================================================= */

int8_t u_charType(UChar32 c)
{
    if (c < 0)
        return (int8_t)U_UNASSIGNED;
    if (c <= 0x1F || c == 0x7F)
        return (int8_t)U_CONTROL_CHAR;
    if (c >= 0x80 && c <= 0x9F)
        return (int8_t)U_CONTROL_CHAR;
    if (c == ' ')
        return (int8_t)U_SPACE_SEPARATOR;
    if (c >= '0' && c <= '9')
        return (int8_t)U_DECIMAL_DIGIT_NUMBER;
    if (c >= 'A' && c <= 'Z')
        return (int8_t)U_UPPERCASE_LETTER;
    if (c >= 'a' && c <= 'z')
        return (int8_t)U_LOWERCASE_LETTER;
    /* Latin-1 uppercase letters (0xC0-0xD6, 0xD8-0xDE) */
    if ((c >= 0xC0 && c <= 0xD6) || (c >= 0xD8 && c <= 0xDE))
        return (int8_t)U_UPPERCASE_LETTER;
    /* Latin-1 lowercase letters (0xE0-0xF6, 0xF8-0xFE) */
    if ((c >= 0xE0 && c <= 0xF6) || (c >= 0xF8 && c <= 0xFE))
        return (int8_t)U_LOWERCASE_LETTER;
    if (c == 0xFF)
        return (int8_t)U_LOWERCASE_LETTER;
    /* Punctuation */
    if ((c >= '!' && c <= '/') || (c >= ':' && c <= '@') ||
        (c >= '[' && c <= '`') || (c >= '{' && c <= '~'))
        return (int8_t)U_OTHER_PUNCTUATION;
    /* Latin-1 symbols */
    if (c == 0xA0)
        return (int8_t)U_SPACE_SEPARATOR;
    if (c >= 0xA1 && c <= 0xBF)
        return (int8_t)U_OTHER_PUNCTUATION;
    if (c == 0xD7)
        return (int8_t)U_MATH_SYMBOL;
    if (c == 0xF7)
        return (int8_t)U_MATH_SYMBOL;
    /* Default: treat as letter for BMP range */
    if (c <= 0xFFFF)
        return (int8_t)U_OTHER_LETTER;
    if (c <= 0x10FFFF)
        return (int8_t)U_OTHER_LETTER;
    return (int8_t)U_UNASSIGNED;
}

UBool u_isalpha(UChar32 c)
{
    int8_t t = u_charType(c);
    return (t == U_UPPERCASE_LETTER || t == U_LOWERCASE_LETTER ||
            t == U_TITLECASE_LETTER || t == U_MODIFIER_LETTER ||
            t == U_OTHER_LETTER) ? TRUE : FALSE;
}

UBool u_isdigit(UChar32 c)
{
    return (c >= '0' && c <= '9') ? TRUE : FALSE;
}

UBool u_isalnum(UChar32 c)
{
    return (u_isalpha(c) || u_isdigit(c)) ? TRUE : FALSE;
}

UBool u_isspace(UChar32 c)
{
    return (c == ' ' || c == '\t' || c == '\n' || c == '\r' ||
            c == '\f' || c == '\v' || c == 0xA0) ? TRUE : FALSE;
}

UBool u_isWhitespace(UChar32 c)
{
    return u_isspace(c);
}

UBool u_isupper(UChar32 c)
{
    if (c >= 'A' && c <= 'Z')
        return TRUE;
    if ((c >= 0xC0 && c <= 0xD6) || (c >= 0xD8 && c <= 0xDE))
        return TRUE;
    return FALSE;
}

UBool u_islower(UChar32 c)
{
    if (c >= 'a' && c <= 'z')
        return TRUE;
    if ((c >= 0xE0 && c <= 0xF6) || (c >= 0xF8 && c <= 0xFF))
        return TRUE;
    return FALSE;
}

UBool u_istitle(UChar32 c)
{
    (void)c;
    return FALSE;  /* No titlecase in ASCII/Latin-1 */
}

UBool u_iscntrl(UChar32 c)
{
    return ((c >= 0 && c <= 0x1F) || c == 0x7F ||
            (c >= 0x80 && c <= 0x9F)) ? TRUE : FALSE;
}

UBool u_isprint(UChar32 c)
{
    return (c >= 0x20 && c != 0x7F && !(c >= 0x80 && c <= 0x9F))
           ? TRUE : FALSE;
}

UChar32 u_tolower(UChar32 c)
{
    if (c >= 'A' && c <= 'Z')
        return c + 32;
    if ((c >= 0xC0 && c <= 0xD6) || (c >= 0xD8 && c <= 0xDE))
        return c + 32;
    return c;
}

UChar32 u_toupper(UChar32 c)
{
    if (c >= 'a' && c <= 'z')
        return c - 32;
    if ((c >= 0xE0 && c <= 0xF6) || (c >= 0xF8 && c <= 0xFE))
        return c - 32;
    return c;
}

UChar32 u_totitle(UChar32 c)
{
    return u_toupper(c);
}

int32_t u_charDigitValue(UChar32 c)
{
    if (c >= '0' && c <= '9')
        return c - '0';
    return -1;
}

UBool u_isdefined(UChar32 c)
{
    return (c >= 0 && c <= 0x10FFFF) ? TRUE : FALSE;
}

int32_t ublock_getCode(UChar32 c)
{
    if (c < 0x80)    return 1;   /* Basic Latin */
    if (c < 0x100)   return 2;   /* Latin-1 Supplement */
    if (c < 0x180)   return 3;   /* Latin Extended-A */
    if (c < 0x250)   return 4;   /* Latin Extended-B */
    return 0;  /* Unknown */
}

/* ========================================================================= */
/* UTF-16 string operations                                                  */
/* ========================================================================= */

int32_t u_strlen(const UChar *s)
{
    int32_t len = 0;
    if (s == NULL) return 0;
    while (s[len] != 0) len++;
    return len;
}

UChar *u_strcpy(UChar *dst, const UChar *src)
{
    UChar *d = dst;
    while ((*d++ = *src++) != 0)
        ;
    return dst;
}

UChar *u_strncpy(UChar *dst, const UChar *src, int32_t n)
{
    int32_t i;
    for (i = 0; i < n && src[i] != 0; i++)
        dst[i] = src[i];
    for (; i < n; i++)
        dst[i] = 0;
    return dst;
}

UChar *u_strcat(UChar *dst, const UChar *src)
{
    UChar *d = dst;
    while (*d != 0) d++;
    while ((*d++ = *src++) != 0)
        ;
    return dst;
}

int32_t u_strcmp(const UChar *s1, const UChar *s2)
{
    while (*s1 && *s1 == *s2) {
        s1++;
        s2++;
    }
    return (int32_t)*s1 - (int32_t)*s2;
}

int32_t u_strncmp(const UChar *s1, const UChar *s2, int32_t n)
{
    int32_t i;
    for (i = 0; i < n; i++) {
        if (s1[i] != s2[i])
            return (int32_t)s1[i] - (int32_t)s2[i];
        if (s1[i] == 0)
            break;
    }
    return 0;
}

int32_t u_strcasecmp(const UChar *s1, const UChar *s2, uint32_t options)
{
    (void)options;
    while (*s1 && *s2) {
        UChar32 c1 = u_tolower((UChar32)*s1);
        UChar32 c2 = u_tolower((UChar32)*s2);
        if (c1 != c2)
            return (int32_t)c1 - (int32_t)c2;
        s1++;
        s2++;
    }
    return (int32_t)*s1 - (int32_t)*s2;
}

UChar *u_strchr(const UChar *s, UChar c)
{
    while (*s) {
        if (*s == c)
            return (UChar *)s;
        s++;
    }
    return (c == 0) ? (UChar *)s : NULL;
}

UChar *u_strstr(const UChar *s, const UChar *substring)
{
    int32_t sublen = u_strlen(substring);
    if (sublen == 0)
        return (UChar *)s;
    while (*s) {
        if (u_strncmp(s, substring, sublen) == 0)
            return (UChar *)s;
        s++;
    }
    return NULL;
}

/* ========================================================================= */
/* UTF-8 <-> UTF-16 conversion                                              */
/* ========================================================================= */

UChar *u_strFromUTF8(UChar *dest, int32_t destCapacity,
                     int32_t *pDestLength,
                     const char *src, int32_t srcLength,
                     UErrorCode *pErrorCode)
{
    int32_t si = 0, di = 0;
    int32_t slen;

    if (pErrorCode == NULL || U_FAILURE(*pErrorCode))
        return dest;

    slen = (srcLength < 0) ? (int32_t)strlen(src) : srcLength;

    while (si < slen && di < destCapacity) {
        unsigned char b = (unsigned char)src[si];
        if (b < 0x80) {
            dest[di++] = (UChar)b;
            si++;
        } else if ((b & 0xE0) == 0xC0 && si + 1 < slen) {
            UChar32 cp = ((b & 0x1F) << 6) | (src[si + 1] & 0x3F);
            dest[di++] = (UChar)cp;
            si += 2;
        } else if ((b & 0xF0) == 0xE0 && si + 2 < slen) {
            UChar32 cp = ((b & 0x0F) << 12) |
                         ((src[si + 1] & 0x3F) << 6) |
                         (src[si + 2] & 0x3F);
            dest[di++] = (UChar)cp;
            si += 3;
        } else if ((b & 0xF8) == 0xF0 && si + 3 < slen) {
            /* Supplementary: encode as surrogate pair */
            UChar32 cp = ((b & 0x07) << 18) |
                         ((src[si + 1] & 0x3F) << 12) |
                         ((src[si + 2] & 0x3F) << 6) |
                         (src[si + 3] & 0x3F);
            cp -= 0x10000;
            if (di + 1 < destCapacity) {
                dest[di++] = (UChar)(0xD800 + (cp >> 10));
                dest[di++] = (UChar)(0xDC00 + (cp & 0x3FF));
            }
            si += 4;
        } else {
            /* Invalid: replace with U+FFFD */
            dest[di++] = 0xFFFD;
            si++;
        }
    }

    if (di < destCapacity)
        dest[di] = 0;
    if (pDestLength)
        *pDestLength = di;
    if (si < slen)
        *pErrorCode = U_BUFFER_OVERFLOW_ERROR;

    return dest;
}

char *u_strToUTF8(char *dest, int32_t destCapacity,
                  int32_t *pDestLength,
                  const UChar *src, int32_t srcLength,
                  UErrorCode *pErrorCode)
{
    int32_t si = 0, di = 0;
    int32_t slen;

    if (pErrorCode == NULL || U_FAILURE(*pErrorCode))
        return dest;

    slen = (srcLength < 0) ? u_strlen(src) : srcLength;

    while (si < slen && di < destCapacity) {
        UChar32 cp = src[si++];

        /* Handle surrogate pairs */
        if (cp >= 0xD800 && cp <= 0xDBFF && si < slen) {
            UChar low = src[si];
            if (low >= 0xDC00 && low <= 0xDFFF) {
                cp = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                si++;
            }
        }

        if (cp < 0x80) {
            dest[di++] = (char)cp;
        } else if (cp < 0x800) {
            if (di + 1 < destCapacity) {
                dest[di++] = (char)(0xC0 | (cp >> 6));
                dest[di++] = (char)(0x80 | (cp & 0x3F));
            } else break;
        } else if (cp < 0x10000) {
            if (di + 2 < destCapacity) {
                dest[di++] = (char)(0xE0 | (cp >> 12));
                dest[di++] = (char)(0x80 | ((cp >> 6) & 0x3F));
                dest[di++] = (char)(0x80 | (cp & 0x3F));
            } else break;
        } else {
            if (di + 3 < destCapacity) {
                dest[di++] = (char)(0xF0 | (cp >> 18));
                dest[di++] = (char)(0x80 | ((cp >> 12) & 0x3F));
                dest[di++] = (char)(0x80 | ((cp >> 6) & 0x3F));
                dest[di++] = (char)(0x80 | (cp & 0x3F));
            } else break;
        }
    }

    if (di < destCapacity)
        dest[di] = '\0';
    if (pDestLength)
        *pDestLength = di;
    if (si < slen)
        *pErrorCode = U_BUFFER_OVERFLOW_ERROR;

    return dest;
}

/* ========================================================================= */
/* Charset converter (UTF-8 only)                                            */
/* ========================================================================= */

struct UConverter {
    char name[32];
};

UConverter *ucnv_open(const char *converterName, UErrorCode *err)
{
    UConverter *cnv;

    if (err == NULL)
        return NULL;

    cnv = (UConverter *)calloc(1, sizeof(*cnv));
    if (cnv == NULL) {
        *err = U_MEMORY_ALLOCATION_ERROR;
        return NULL;
    }

    if (converterName == NULL)
        strncpy(cnv->name, "UTF-8", sizeof(cnv->name) - 1);
    else
        strncpy(cnv->name, converterName, sizeof(cnv->name) - 1);

    *err = U_ZERO_ERROR;
    return cnv;
}

void ucnv_close(UConverter *converter)
{
    free(converter);
}

const char *ucnv_getName(const UConverter *converter, UErrorCode *err)
{
    if (err) *err = U_ZERO_ERROR;
    if (converter == NULL)
        return "UTF-8";
    return converter->name;
}

void ucnv_toUChars(UConverter *cnv, UChar *dest, int32_t destCapacity,
                   const char *src, int32_t srcLength,
                   UErrorCode *pErrorCode)
{
    (void)cnv;
    u_strFromUTF8(dest, destCapacity, NULL, src, srcLength, pErrorCode);
}

void ucnv_fromUChars(UConverter *cnv, char *dest, int32_t destCapacity,
                     const UChar *src, int32_t srcLength,
                     UErrorCode *pErrorCode)
{
    (void)cnv;
    u_strToUTF8(dest, destCapacity, NULL, src, srcLength, pErrorCode);
}

int8_t ucnv_getMaxCharSize(const UConverter *converter)
{
    (void)converter;
    return 4;  /* UTF-8 max */
}

int8_t ucnv_getMinCharSize(const UConverter *converter)
{
    (void)converter;
    return 1;
}

int32_t ucnv_countAvailable(void)
{
    return 1;  /* Only UTF-8 */
}

const char *ucnv_getAvailableName(int32_t n)
{
    (void)n;
    return "UTF-8";
}

/* ========================================================================= */
/* Normalization (NFC passthrough)                                           */
/* ========================================================================= */

static UNormalizer2 nfc_instance;
static UNormalizer2 nfd_instance;
static UNormalizer2 nfkc_instance;
static UNormalizer2 nfkd_instance;

const UNormalizer2 *unorm2_getNFCInstance(UErrorCode *pErrorCode)
{
    if (pErrorCode) *pErrorCode = U_ZERO_ERROR;
    return &nfc_instance;
}

const UNormalizer2 *unorm2_getNFDInstance(UErrorCode *pErrorCode)
{
    if (pErrorCode) *pErrorCode = U_ZERO_ERROR;
    return &nfd_instance;
}

const UNormalizer2 *unorm2_getNFKCInstance(UErrorCode *pErrorCode)
{
    if (pErrorCode) *pErrorCode = U_ZERO_ERROR;
    return &nfkc_instance;
}

const UNormalizer2 *unorm2_getNFKDInstance(UErrorCode *pErrorCode)
{
    if (pErrorCode) *pErrorCode = U_ZERO_ERROR;
    return &nfkd_instance;
}

int32_t unorm2_normalize(const UNormalizer2 *norm2,
                         const UChar *src, int32_t length,
                         UChar *dest, int32_t capacity,
                         UErrorCode *pErrorCode)
{
    int32_t slen;
    (void)norm2;

    if (pErrorCode == NULL || U_FAILURE(*pErrorCode))
        return 0;

    slen = (length < 0) ? u_strlen(src) : length;

    /* NFC passthrough: most text is already in NFC form */
    if (slen >= capacity) {
        *pErrorCode = U_BUFFER_OVERFLOW_ERROR;
        return slen;
    }

    memcpy(dest, src, (size_t)slen * sizeof(UChar));
    dest[slen] = 0;
    return slen;
}

UNormalizationCheckResult unorm2_quickCheck(const UNormalizer2 *norm2,
                                            const UChar *s, int32_t length,
                                            UErrorCode *pErrorCode)
{
    (void)norm2;
    (void)s;
    (void)length;
    if (pErrorCode) *pErrorCode = U_ZERO_ERROR;
    return UNORM_YES;  /* Assume normalized */
}

UBool unorm2_isNormalized(const UNormalizer2 *norm2,
                          const UChar *s, int32_t length,
                          UErrorCode *pErrorCode)
{
    (void)norm2;
    (void)s;
    (void)length;
    if (pErrorCode) *pErrorCode = U_ZERO_ERROR;
    return TRUE;
}

/* ========================================================================= */
/* Collation (simple code-point comparison)                                  */
/* ========================================================================= */

struct UCollator {
    UCollationStrength strength;
    char locale[32];
};

UCollator *ucol_open(const char *loc, UErrorCode *status)
{
    UCollator *coll;

    if (status == NULL)
        return NULL;

    coll = (UCollator *)calloc(1, sizeof(*coll));
    if (coll == NULL) {
        *status = U_MEMORY_ALLOCATION_ERROR;
        return NULL;
    }

    coll->strength = UCOL_TERTIARY;
    if (loc)
        strncpy(coll->locale, loc, sizeof(coll->locale) - 1);

    *status = U_ZERO_ERROR;
    return coll;
}

void ucol_close(UCollator *coll)
{
    free(coll);
}

UCollationResult ucol_strcoll(const UCollator *coll,
                              const UChar *source, int32_t sourceLength,
                              const UChar *target, int32_t targetLength)
{
    int32_t slen, tlen, minlen, i;
    (void)coll;

    slen = (sourceLength < 0) ? u_strlen(source) : sourceLength;
    tlen = (targetLength < 0) ? u_strlen(target) : targetLength;
    minlen = (slen < tlen) ? slen : tlen;

    for (i = 0; i < minlen; i++) {
        if (source[i] < target[i])
            return UCOL_LESS;
        if (source[i] > target[i])
            return UCOL_GREATER;
    }

    if (slen < tlen) return UCOL_LESS;
    if (slen > tlen) return UCOL_GREATER;
    return UCOL_EQUAL;
}

UCollationStrength ucol_getStrength(const UCollator *coll)
{
    return coll ? coll->strength : UCOL_TERTIARY;
}

void ucol_setStrength(UCollator *coll, UCollationStrength newStrength)
{
    if (coll) coll->strength = newStrength;
}

int32_t ucol_getSortKey(const UCollator *coll,
                        const UChar *source, int32_t sourceLength,
                        uint8_t *result, int32_t resultLength)
{
    int32_t slen, i, j = 0;
    (void)coll;

    slen = (sourceLength < 0) ? u_strlen(source) : sourceLength;

    /* Simple sort key: just the raw UChar values as bytes */
    for (i = 0; i < slen && j + 2 < resultLength; i++) {
        result[j++] = (uint8_t)(source[i] >> 8);
        result[j++] = (uint8_t)(source[i] & 0xFF);
    }
    if (j < resultLength)
        result[j] = 0;

    return j + 1;
}

/* ========================================================================= */
/* Break iterator (character-based)                                          */
/* ========================================================================= */

struct UBreakIterator {
    UBreakIteratorType type;
    const UChar *text;
    int32_t      textLength;
    int32_t      pos;
};

UBreakIterator *ubrk_open(UBreakIteratorType type, const char *locale,
                           const UChar *text, int32_t textLength,
                           UErrorCode *status)
{
    UBreakIterator *bi;
    (void)locale;

    if (status == NULL)
        return NULL;

    bi = (UBreakIterator *)calloc(1, sizeof(*bi));
    if (bi == NULL) {
        *status = U_MEMORY_ALLOCATION_ERROR;
        return NULL;
    }

    bi->type = type;
    bi->text = text;
    bi->textLength = (textLength < 0 && text) ? u_strlen(text) : textLength;
    bi->pos = 0;

    *status = U_ZERO_ERROR;
    return bi;
}

void ubrk_close(UBreakIterator *bi)
{
    free(bi);
}

void ubrk_setText(UBreakIterator *bi, const UChar *text,
                  int32_t textLength, UErrorCode *status)
{
    if (bi == NULL) return;
    bi->text = text;
    bi->textLength = (textLength < 0 && text) ? u_strlen(text) : textLength;
    bi->pos = 0;
    if (status) *status = U_ZERO_ERROR;
}

int32_t ubrk_first(UBreakIterator *bi)
{
    if (bi == NULL) return UBRK_DONE;
    bi->pos = 0;
    return 0;
}

int32_t ubrk_last(UBreakIterator *bi)
{
    if (bi == NULL) return UBRK_DONE;
    bi->pos = bi->textLength;
    return bi->pos;
}

int32_t ubrk_next(UBreakIterator *bi)
{
    if (bi == NULL || bi->pos >= bi->textLength)
        return UBRK_DONE;

    /* Simple: advance by one code unit for character boundaries,
     * by word for word boundaries (space-delimited) */
    if (bi->type == UBRK_WORD) {
        /* Skip non-spaces, then spaces */
        while (bi->pos < bi->textLength && bi->text[bi->pos] != ' ')
            bi->pos++;
        while (bi->pos < bi->textLength && bi->text[bi->pos] == ' ')
            bi->pos++;
    } else {
        bi->pos++;
        /* Skip low surrogate if we hit a high surrogate */
        if (bi->pos < bi->textLength &&
            bi->text[bi->pos - 1] >= 0xD800 &&
            bi->text[bi->pos - 1] <= 0xDBFF &&
            bi->text[bi->pos] >= 0xDC00 &&
            bi->text[bi->pos] <= 0xDFFF)
            bi->pos++;
    }

    return (bi->pos <= bi->textLength) ? bi->pos : UBRK_DONE;
}

int32_t ubrk_previous(UBreakIterator *bi)
{
    if (bi == NULL || bi->pos <= 0)
        return UBRK_DONE;
    bi->pos--;
    return bi->pos;
}

int32_t ubrk_following(UBreakIterator *bi, int32_t offset)
{
    if (bi == NULL) return UBRK_DONE;
    bi->pos = offset;
    return ubrk_next(bi);
}

int32_t ubrk_preceding(UBreakIterator *bi, int32_t offset)
{
    if (bi == NULL) return UBRK_DONE;
    bi->pos = offset;
    return ubrk_previous(bi);
}

int32_t ubrk_current(const UBreakIterator *bi)
{
    if (bi == NULL) return UBRK_DONE;
    return bi->pos;
}

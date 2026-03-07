/*
 * VeridianOS C Library -- <iconv.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Character set conversion interface (POSIX / SUSv3).
 * Minimal implementation supporting UTF-8, UTF-16, UTF-32,
 * ISO-8859-1 (Latin-1), and ASCII conversions for Qt 6.
 */

#ifndef _ICONV_H
#define _ICONV_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Opaque conversion descriptor. */
typedef void *iconv_t;

/**
 * Allocate a conversion descriptor for converting between character
 * encodings @p fromcode and @p tocode.
 *
 * Supported encodings: "UTF-8", "UTF-16", "UTF-16LE", "UTF-16BE",
 * "UTF-32", "UTF-32LE", "UTF-32BE", "ISO-8859-1", "LATIN1",
 * "ASCII", "US-ASCII".
 *
 * @return Conversion descriptor on success, (iconv_t)-1 on error
 *         (errno set to EINVAL for unsupported encoding).
 */
iconv_t iconv_open(const char *tocode, const char *fromcode);

/**
 * Convert characters from one encoding to another.
 *
 * @param cd       Conversion descriptor from iconv_open().
 * @param inbuf    Pointer to input buffer pointer (advanced on return).
 * @param inbytesleft  Pointer to remaining input bytes (decremented).
 * @param outbuf   Pointer to output buffer pointer (advanced on return).
 * @param outbytesleft Pointer to remaining output space (decremented).
 * @return Number of non-reversible conversions, or (size_t)-1 on error.
 */
size_t iconv(iconv_t cd, char **inbuf, size_t *inbytesleft,
             char **outbuf, size_t *outbytesleft);

/**
 * Free a conversion descriptor.
 *
 * @return 0 on success, -1 on error.
 */
int iconv_close(iconv_t cd);

#ifdef __cplusplus
}
#endif

#endif /* _ICONV_H */

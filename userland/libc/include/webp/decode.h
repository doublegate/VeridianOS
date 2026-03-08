/*
 * VeridianOS libc -- webp/decode.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libwebp 1.4.x decoding API.
 */

#ifndef _WEBP_DECODE_H
#define _WEBP_DECODE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Version                                                                   */
/* ========================================================================= */

#define WEBP_DECODER_ABI_VERSION 0x0209

/* ========================================================================= */
/* Decoding API                                                              */
/* ========================================================================= */

/** Get basic info about a WebP image without decoding. */
int WebPGetInfo(const uint8_t *data, size_t data_size,
                int *width, int *height);

/** Decode a WebP image to RGBA. Returns allocated buffer. */
uint8_t *WebPDecodeRGBA(const uint8_t *data, size_t data_size,
                         int *width, int *height);

/** Decode a WebP image to ARGB. Returns allocated buffer. */
uint8_t *WebPDecodeARGB(const uint8_t *data, size_t data_size,
                         int *width, int *height);

/** Decode a WebP image to BGR. Returns allocated buffer. */
uint8_t *WebPDecodeBGR(const uint8_t *data, size_t data_size,
                        int *width, int *height);

/** Decode a WebP image to BGRA. Returns allocated buffer. */
uint8_t *WebPDecodeBGRA(const uint8_t *data, size_t data_size,
                         int *width, int *height);

/** Decode a WebP image to RGB. Returns allocated buffer. */
uint8_t *WebPDecodeRGB(const uint8_t *data, size_t data_size,
                        int *width, int *height);

/** Decode into a preallocated buffer. */
uint8_t *WebPDecodeRGBAInto(const uint8_t *data, size_t data_size,
                             uint8_t *output_buffer,
                             size_t output_buffer_size,
                             int output_stride);

/** Free a buffer allocated by WebPDecode*. */
void WebPFree(void *ptr);

/** Get the decoder version. */
int WebPGetDecoderVersion(void);

#ifdef __cplusplus
}
#endif

#endif /* _WEBP_DECODE_H */

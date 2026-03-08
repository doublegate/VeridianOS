/*
 * VeridianOS libc -- libwebp_shim.c
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * libwebp 1.4.x stubs.
 * WebP decoding requires a VP8/VP8L decoder which is not yet implemented.
 * These stubs provide the API surface for linking.
 */

#include <webp/decode.h>
#include <stdlib.h>

int WebPGetInfo(const uint8_t *data, size_t data_size,
                int *width, int *height)
{
    /* WebP files start with "RIFF" + size + "WEBP" */
    if (data == NULL || data_size < 30)
        return 0;

    if (data[0] != 'R' || data[1] != 'I' ||
        data[2] != 'F' || data[3] != 'F')
        return 0;

    if (data[8] != 'W' || data[9] != 'E' ||
        data[10] != 'B' || data[11] != 'P')
        return 0;

    /* Parse VP8 header for dimensions (simplified) */
    if (data[12] == 'V' && data[13] == 'P' && data[14] == '8' &&
        data[15] == ' ') {
        /* Lossy VP8 */
        if (data_size >= 30) {
            /* VP8 bitstream starts at offset 20 */
            /* Frame tag at bytes 20-22, then width/height */
            if (width)
                *width = (data[26] | (data[27] << 8)) & 0x3FFF;
            if (height)
                *height = (data[28] | (data[29] << 8)) & 0x3FFF;
            return 1;
        }
    }

    /* Default: unknown */
    if (width) *width = 0;
    if (height) *height = 0;
    return 0;
}

uint8_t *WebPDecodeRGBA(const uint8_t *data, size_t data_size,
                         int *width, int *height)
{
    (void)data; (void)data_size; (void)width; (void)height;
    return NULL;  /* VP8 decoder not implemented */
}

uint8_t *WebPDecodeARGB(const uint8_t *data, size_t data_size,
                         int *width, int *height)
{
    (void)data; (void)data_size; (void)width; (void)height;
    return NULL;
}

uint8_t *WebPDecodeBGR(const uint8_t *data, size_t data_size,
                        int *width, int *height)
{
    (void)data; (void)data_size; (void)width; (void)height;
    return NULL;
}

uint8_t *WebPDecodeBGRA(const uint8_t *data, size_t data_size,
                         int *width, int *height)
{
    (void)data; (void)data_size; (void)width; (void)height;
    return NULL;
}

uint8_t *WebPDecodeRGB(const uint8_t *data, size_t data_size,
                        int *width, int *height)
{
    (void)data; (void)data_size; (void)width; (void)height;
    return NULL;
}

uint8_t *WebPDecodeRGBAInto(const uint8_t *data, size_t data_size,
                             uint8_t *output_buffer,
                             size_t output_buffer_size,
                             int output_stride)
{
    (void)data; (void)data_size; (void)output_buffer;
    (void)output_buffer_size; (void)output_stride;
    return NULL;
}

void WebPFree(void *ptr)
{
    free(ptr);
}

int WebPGetDecoderVersion(void)
{
    return WEBP_DECODER_ABI_VERSION;
}

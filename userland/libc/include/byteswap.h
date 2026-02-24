/*
 * VeridianOS libc -- <byteswap.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Byte swapping macros for endian conversion.
 */

#ifndef _BYTESWAP_H
#define _BYTESWAP_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

static inline uint16_t bswap_16(uint16_t x)
{
    return (uint16_t)((x >> 8) | (x << 8));
}

static inline uint32_t bswap_32(uint32_t x)
{
    return ((x & 0xFF000000U) >> 24) |
           ((x & 0x00FF0000U) >> 8)  |
           ((x & 0x0000FF00U) << 8)  |
           ((x & 0x000000FFU) << 24);
}

static inline uint64_t bswap_64(uint64_t x)
{
    return ((x & 0xFF00000000000000ULL) >> 56) |
           ((x & 0x00FF000000000000ULL) >> 40) |
           ((x & 0x0000FF0000000000ULL) >> 24) |
           ((x & 0x000000FF00000000ULL) >> 8)  |
           ((x & 0x00000000FF000000ULL) << 8)  |
           ((x & 0x0000000000FF0000ULL) << 24) |
           ((x & 0x000000000000FF00ULL) << 40) |
           ((x & 0x00000000000000FFULL) << 56);
}

#ifdef __cplusplus
}
#endif

#endif /* _BYTESWAP_H */

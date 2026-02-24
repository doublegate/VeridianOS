/*
 * VeridianOS libc -- <endian.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Byte order definitions.
 * VeridianOS x86_64 is always little-endian.
 */

#ifndef _ENDIAN_H
#define _ENDIAN_H

#define __LITTLE_ENDIAN 1234
#define __BIG_ENDIAN    4321
#define __PDP_ENDIAN    3412

#define LITTLE_ENDIAN   __LITTLE_ENDIAN
#define BIG_ENDIAN      __BIG_ENDIAN
#define PDP_ENDIAN      __PDP_ENDIAN

#if defined(__x86_64__) || defined(__i386__) || defined(__aarch64__) || \
    (defined(__riscv) && __riscv_xlen == 64)
# define __BYTE_ORDER   __LITTLE_ENDIAN
# define BYTE_ORDER     __BYTE_ORDER
#else
# error "Unknown architecture -- cannot determine byte order"
#endif

/* Conversion macros */
#include <byteswap.h>

#if __BYTE_ORDER == __LITTLE_ENDIAN
# define htobe16(x) bswap_16(x)
# define htobe32(x) bswap_32(x)
# define htobe64(x) bswap_64(x)
# define htole16(x) (x)
# define htole32(x) (x)
# define htole64(x) (x)
# define be16toh(x) bswap_16(x)
# define be32toh(x) bswap_32(x)
# define be64toh(x) bswap_64(x)
# define le16toh(x) (x)
# define le32toh(x) (x)
# define le64toh(x) (x)
#else
# define htobe16(x) (x)
# define htobe32(x) (x)
# define htobe64(x) (x)
# define htole16(x) bswap_16(x)
# define htole32(x) bswap_32(x)
# define htole64(x) bswap_64(x)
# define be16toh(x) (x)
# define be32toh(x) (x)
# define be64toh(x) (x)
# define le16toh(x) bswap_16(x)
# define le32toh(x) bswap_32(x)
# define le64toh(x) bswap_64(x)
#endif

#endif /* _ENDIAN_H */

/*
 * VeridianOS libc -- <netinet/in.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Internet protocol (IPv4/IPv6) address definitions.
 */

#ifndef _NETINET_IN_H
#define _NETINET_IN_H

#include <sys/socket.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* IP protocols */
#define IPPROTO_IP      0
#define IPPROTO_TCP     6
#define IPPROTO_UDP     17

/* Internet address (IPv4) */
typedef uint32_t in_addr_t;
typedef uint16_t in_port_t;

struct in_addr {
    in_addr_t s_addr;
};

/* Socket address for IPv4 */
struct sockaddr_in {
    sa_family_t    sin_family;
    in_port_t      sin_port;
    struct in_addr sin_addr;
    unsigned char  sin_zero[8];
};

/* IPv6 address */
struct in6_addr {
    union {
        uint8_t  s6_addr[16];
        uint16_t s6_addr16[8];
        uint32_t s6_addr32[4];
    };
};

/* Socket address for IPv6 */
struct sockaddr_in6 {
    sa_family_t     sin6_family;
    in_port_t       sin6_port;
    uint32_t        sin6_flowinfo;
    struct in6_addr sin6_addr;
    uint32_t        sin6_scope_id;
};

/* Special addresses */
#define INADDR_ANY          ((in_addr_t)0x00000000)
#define INADDR_BROADCAST    ((in_addr_t)0xffffffff)
#define INADDR_LOOPBACK     ((in_addr_t)0x7f000001)
#define INADDR_NONE         ((in_addr_t)0xffffffff)

/* Byte order conversion (assuming little-endian host) */
static inline uint16_t __bswap16(uint16_t x) {
    return (uint16_t)((x >> 8) | (x << 8));
}
static inline uint32_t __bswap32(uint32_t x) {
    return ((x >> 24) & 0xff) | ((x >> 8) & 0xff00) |
           ((x << 8) & 0xff0000) | ((x << 24) & 0xff000000u);
}

#ifndef htons
#define htons(x)    __bswap16(x)
#define ntohs(x)    __bswap16(x)
#define htonl(x)    __bswap32(x)
#define ntohl(x)    __bswap32(x)
#endif

#ifdef __cplusplus
}
#endif

#endif /* _NETINET_IN_H */

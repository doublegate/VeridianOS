/*
 * VeridianOS libc -- <arpa/inet.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Internet address manipulation functions.
 */

#ifndef _ARPA_INET_H
#define _ARPA_INET_H

#include <netinet/in.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Convert IPv4 dotted-decimal string to network byte order. */
in_addr_t inet_addr(const char *cp);

/** Convert IPv4 address to dotted-decimal string. */
char *inet_ntoa(struct in_addr in);

/** Convert address from text to binary form. */
int inet_pton(int af, const char *src, void *dst);

/** Convert address from binary to text form. */
const char *inet_ntop(int af, const void *src, char *dst, socklen_t size);

#ifdef __cplusplus
}
#endif

#endif /* _ARPA_INET_H */

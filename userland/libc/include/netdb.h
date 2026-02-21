/*
 * VeridianOS C Library -- <netdb.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

#ifndef _NETDB_H
#define _NETDB_H

#include <sys/types.h>
#include <sys/socket.h>

#ifdef __cplusplus
extern "C" {
#endif

struct hostent {
    char  *h_name;
    char **h_aliases;
    int    h_addrtype;
    int    h_length;
    char **h_addr_list;
};

#define h_addr h_addr_list[0]

struct hostent *gethostbyname(const char *name);

/* getaddrinfo flags */
#define AI_PASSIVE      0x01
#define AI_CANONNAME    0x02
#define AI_NUMERICHOST  0x04
#define AI_NUMERICSERV  0x400
#define AI_ADDRCONFIG   0x20

/* getaddrinfo/getnameinfo error codes */
#define EAI_NONAME      -2
#define EAI_SERVICE     -8
#define EAI_FAIL        -4
#define EAI_MEMORY      -10
#define EAI_FAMILY      -6

/* Name info flags */
#define NI_NUMERICHOST  1
#define NI_NUMERICSERV  2
#define NI_MAXHOST      1025
#define NI_MAXSERV      32

struct addrinfo {
    int              ai_flags;
    int              ai_family;
    int              ai_socktype;
    int              ai_protocol;
    socklen_t        ai_addrlen;
    struct sockaddr *ai_addr;
    char            *ai_canonname;
    struct addrinfo *ai_next;
};

/** Resolve host/service to socket addresses. */
int getaddrinfo(const char *node, const char *service,
                const struct addrinfo *hints, struct addrinfo **res);

/** Free result from getaddrinfo. */
void freeaddrinfo(struct addrinfo *res);

/** Translate getaddrinfo error code to string. */
const char *gai_strerror(int errcode);

/** Reverse lookup: address to host/service name. */
int getnameinfo(const struct sockaddr *sa, socklen_t salen,
                char *host, socklen_t hostlen,
                char *serv, socklen_t servlen, int flags);

#ifdef __cplusplus
}
#endif

#endif /* _NETDB_H */

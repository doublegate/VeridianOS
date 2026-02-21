/*
 * VeridianOS libc -- <sys/socket.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Socket interface definitions.
 */

#ifndef _SYS_SOCKET_H
#define _SYS_SOCKET_H

#include <sys/types.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Socket types */
#define SOCK_STREAM     1   /* Sequenced, reliable, connection-based */
#define SOCK_DGRAM      2   /* Connectionless, unreliable datagrams */
#define SOCK_RAW        3   /* Raw protocol interface */
#define SOCK_SEQPACKET  5   /* Sequenced, reliable, connection-based, fixed-length */

/* Address families */
#define AF_UNSPEC       0
#define AF_LOCAL        1
#define AF_UNIX         AF_LOCAL
#define AF_INET         2
#define AF_INET6        10

/* Protocol families (same as address families) */
#define PF_UNSPEC       AF_UNSPEC
#define PF_LOCAL        AF_LOCAL
#define PF_UNIX         AF_UNIX
#define PF_INET         AF_INET
#define PF_INET6        AF_INET6

/* Socket options */
#define SOL_SOCKET      1
#define SO_REUSEADDR    2
#define SO_ERROR        4
#define SO_KEEPALIVE    9

/* Shutdown modes */
#define SHUT_RD         0
#define SHUT_WR         1
#define SHUT_RDWR       2

/* Message flags */
#define MSG_PEEK        0x02
#define MSG_WAITALL     0x100
#define MSG_DONTWAIT    0x40
#define MSG_NOSIGNAL    0x4000

/* Socket address structure */
typedef unsigned short sa_family_t;
typedef unsigned int socklen_t;

struct sockaddr {
    sa_family_t sa_family;
    char        sa_data[14];
};

struct sockaddr_storage {
    sa_family_t ss_family;
    char        __ss_padding[126];
};

/* I/O vector for scatter/gather */
struct iovec {
    void   *iov_base;
    size_t  iov_len;
};

struct msghdr {
    void         *msg_name;
    socklen_t     msg_namelen;
    struct iovec *msg_iov;
    int           msg_iovlen;
    void         *msg_control;
    socklen_t     msg_controllen;
    int           msg_flags;
};

/* Socket functions */
int socket(int domain, int type, int protocol);
int bind(int sockfd, const struct sockaddr *addr, socklen_t addrlen);
int listen(int sockfd, int backlog);
int accept(int sockfd, struct sockaddr *addr, socklen_t *addrlen);
int connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen);
ssize_t send(int sockfd, const void *buf, size_t len, int flags);
ssize_t recv(int sockfd, void *buf, size_t len, int flags);
ssize_t sendto(int sockfd, const void *buf, size_t len, int flags,
               const struct sockaddr *dest_addr, socklen_t addrlen);
ssize_t recvfrom(int sockfd, void *buf, size_t len, int flags,
                 struct sockaddr *src_addr, socklen_t *addrlen);
ssize_t sendmsg(int sockfd, const struct msghdr *msg, int flags);
ssize_t recvmsg(int sockfd, struct msghdr *msg, int flags);
int shutdown(int sockfd, int how);
int getsockopt(int sockfd, int level, int optname, void *optval, socklen_t *optlen);
int setsockopt(int sockfd, int level, int optname, const void *optval, socklen_t optlen);
int getpeername(int sockfd, struct sockaddr *addr, socklen_t *addrlen);
int getsockname(int sockfd, struct sockaddr *addr, socklen_t *addrlen);
int socketpair(int domain, int type, int protocol, int sv[2]);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_SOCKET_H */

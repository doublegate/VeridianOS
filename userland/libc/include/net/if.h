/*
 * VeridianOS libc -- <net/if.h>
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Network interface definitions.
 */

#ifndef _NET_IF_H
#define _NET_IF_H

#include <sys/types.h>
#include <sys/socket.h>

#ifdef __cplusplus
extern "C" {
#endif

#define IF_NAMESIZE     16
#define IFNAMSIZ        IF_NAMESIZE

/* Interface flags (ioctl SIOCGIFFLAGS/SIOCSIFFLAGS) */
#define IFF_UP          0x1
#define IFF_BROADCAST   0x2
#define IFF_DEBUG       0x4
#define IFF_LOOPBACK    0x8
#define IFF_POINTOPOINT 0x10
#define IFF_RUNNING     0x40
#define IFF_NOARP       0x80
#define IFF_PROMISC     0x100
#define IFF_MULTICAST   0x1000

struct ifreq {
    char ifr_name[IFNAMSIZ];
    union {
        struct sockaddr ifr_addr;
        struct sockaddr ifr_dstaddr;
        struct sockaddr ifr_broadaddr;
        struct sockaddr ifr_netmask;
        struct sockaddr ifr_hwaddr;
        short           ifr_flags;
        int             ifr_ifindex;
        int             ifr_metric;
        int             ifr_mtu;
        char            ifr_slave[IFNAMSIZ];
        char            ifr_newname[IFNAMSIZ];
        void           *ifr_data;
    };
};

struct ifconf {
    int                 ifc_len;
    union {
        char           *ifc_buf;
        struct ifreq   *ifc_req;
    };
};

unsigned int if_nametoindex(const char *ifname);
char *if_indextoname(unsigned int ifindex, char *ifname);

#ifdef __cplusplus
}
#endif

#endif /* _NET_IF_H */

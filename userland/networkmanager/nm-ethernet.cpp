/*
 * VeridianOS -- nm-ethernet.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Ethernet backend implementation for the NetworkManager shim.
 *
 * Provides link state detection via SIOCGIFFLAGS, DHCP via the kernel
 * DHCP client (kernel/src/net/dhcp.rs), static IP via SIOCSIFADDR,
 * and MAC address queries from /sys/class/net/<iface>/address.
 */

#include "nm-ethernet.h"

#include <QDebug>
#include <QFile>
#include <QElapsedTimer>

#include <cstring>
#include <cstdio>
#include <cstdlib>

#include <unistd.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

/* ========================================================================= */
/* VeridianOS ioctl definitions                                              */
/* ========================================================================= */

/* Standard Linux-compatible ioctl numbers */
#ifndef SIOCGIFFLAGS
#define SIOCGIFFLAGS    0x8913
#endif
#ifndef SIOCSIFFLAGS
#define SIOCSIFFLAGS    0x8914
#endif
#ifndef SIOCSIFADDR
#define SIOCSIFADDR     0x8916
#endif
#ifndef SIOCSIFNETMASK
#define SIOCSIFNETMASK  0x891C
#endif
#ifndef SIOCADDRT
#define SIOCADDRT       0x890B
#endif

/* Interface flags */
#ifndef IFF_UP
#define IFF_UP          0x0001
#endif
#ifndef IFF_RUNNING
#define IFF_RUNNING     0x0040
#endif

/* VeridianOS kernel DHCP ioctl */
static const unsigned long VERIDIAN_DHCP_START   = 0x8D01;
static const unsigned long VERIDIAN_DHCP_STATUS  = 0x8D02;

/* DHCP timeout */
static const int DHCP_POLL_INTERVAL_MS = 500;

/* ========================================================================= */
/* Interface request structure (matches Linux ifreq)                         */
/* ========================================================================= */

struct veridian_ifreq {
    char ifr_name[16];
    union {
        struct sockaddr ifr_addr;
        struct sockaddr ifr_netmask;
        struct sockaddr ifr_gateway;
        short ifr_flags;
        int   ifr_ifindex;
        char  ifr_data[16];
    };
};

/* ========================================================================= */
/* Route entry structure (matches Linux rtentry, simplified)                 */
/* ========================================================================= */

struct veridian_rtentry {
    struct sockaddr rt_dst;
    struct sockaddr rt_gateway;
    struct sockaddr rt_genmask;
    unsigned short  rt_flags;
    char            rt_dev[16];
};

/* Route flags */
#define RTF_UP      0x0001
#define RTF_GATEWAY 0x0002

/* ========================================================================= */
/* DHCP result from kernel                                                   */
/* ========================================================================= */

struct DhcpResult {
    uint8_t  state;          /* 0=idle, 1=discovering, 2=bound, 3=failed */
    uint8_t  _padding[3];
    uint32_t ip_addr;        /* network byte order */
    uint32_t netmask;
    uint32_t gateway;
    uint32_t dns1;
    uint32_t dns2;
    uint32_t lease_time;     /* seconds */
    uint32_t server_id;      /* DHCP server IP */
};

/* ========================================================================= */
/* Helper: open a socket for ioctl operations                                */
/* ========================================================================= */

static int open_ioctl_socket(void)
{
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (fd < 0) {
        qWarning("NM-Ethernet: cannot open ioctl socket: %m");
    }
    return fd;
}

/* ========================================================================= */
/* Helper: set sockaddr_in from dotted-quad string                           */
/* ========================================================================= */

static void set_sockaddr_in(struct sockaddr *sa, const char *addr_str)
{
    struct sockaddr_in *sin = (struct sockaddr_in *)sa;
    memset(sin, 0, sizeof(*sin));
    sin->sin_family = AF_INET;
    if (addr_str) {
        inet_pton(AF_INET, addr_str, &sin->sin_addr);
    }
}

/* ========================================================================= */
/* Helper: set sockaddr_in from uint32_t (network byte order)                */
/* ========================================================================= */

static void set_sockaddr_in_raw(struct sockaddr *sa, uint32_t addr)
{
    struct sockaddr_in *sin = (struct sockaddr_in *)sa;
    memset(sin, 0, sizeof(*sin));
    sin->sin_family = AF_INET;
    sin->sin_addr.s_addr = addr;
}

/* ========================================================================= */
/* Helper: bring interface up                                                */
/* ========================================================================= */

static bool bring_interface_up(int fd, const char *iface)
{
    struct veridian_ifreq ifr;
    memset(&ifr, 0, sizeof(ifr));
    strncpy(ifr.ifr_name, iface, 15);

    if (ioctl(fd, SIOCGIFFLAGS, &ifr) < 0) {
        qWarning("NM-Ethernet: SIOCGIFFLAGS failed on %s: %m", iface);
        return false;
    }

    ifr.ifr_flags |= IFF_UP;

    if (ioctl(fd, SIOCSIFFLAGS, &ifr) < 0) {
        qWarning("NM-Ethernet: SIOCSIFFLAGS (UP) failed on %s: %m", iface);
        return false;
    }

    return true;
}

/* ========================================================================= */
/* Link state detection                                                      */
/* ========================================================================= */

bool nm_ethernet_detect_link(const char *iface)
{
    if (!iface)
        return false;

    /* Method 1: sysfs carrier file */
    QString carrierPath = QStringLiteral("/sys/class/net/%1/carrier")
                              .arg(QString::fromLatin1(iface));
    QFile file(carrierPath);
    if (file.open(QIODevice::ReadOnly | QIODevice::Text)) {
        QByteArray data = file.readAll().trimmed();
        if (data == "1") {
            qDebug("NM-Ethernet: link detected on %s (sysfs)", iface);
            return true;
        }
    }

    /* Method 2: ioctl SIOCGIFFLAGS */
    int fd = open_ioctl_socket();
    if (fd < 0)
        return false;

    struct veridian_ifreq ifr;
    memset(&ifr, 0, sizeof(ifr));
    strncpy(ifr.ifr_name, iface, 15);

    bool link_up = false;
    if (ioctl(fd, SIOCGIFFLAGS, &ifr) == 0) {
        link_up = (ifr.ifr_flags & IFF_RUNNING) != 0;
    }

    close(fd);

    if (link_up)
        qDebug("NM-Ethernet: link detected on %s (ioctl)", iface);
    else
        qDebug("NM-Ethernet: no link on %s", iface);

    return link_up;
}

/* ========================================================================= */
/* Static IP configuration                                                   */
/* ========================================================================= */

bool nm_ethernet_set_ip(const char *iface, const char *addr,
                        const char *netmask, const char *gateway)
{
    if (!iface || !addr || !netmask)
        return false;

    int fd = open_ioctl_socket();
    if (fd < 0)
        return false;

    bool success = true;

    /* Bring interface up */
    if (!bring_interface_up(fd, iface)) {
        close(fd);
        return false;
    }

    /* Set IP address */
    {
        struct veridian_ifreq ifr;
        memset(&ifr, 0, sizeof(ifr));
        strncpy(ifr.ifr_name, iface, 15);
        set_sockaddr_in(&ifr.ifr_addr, addr);

        if (ioctl(fd, SIOCSIFADDR, &ifr) < 0) {
            qWarning("NM-Ethernet: SIOCSIFADDR failed on %s: %m", iface);
            success = false;
        }
    }

    /* Set netmask */
    if (success) {
        struct veridian_ifreq ifr;
        memset(&ifr, 0, sizeof(ifr));
        strncpy(ifr.ifr_name, iface, 15);
        set_sockaddr_in(&ifr.ifr_netmask, netmask);

        if (ioctl(fd, SIOCSIFNETMASK, &ifr) < 0) {
            qWarning("NM-Ethernet: SIOCSIFNETMASK failed on %s: %m", iface);
            success = false;
        }
    }

    /* Set default gateway */
    if (success && gateway) {
        struct veridian_rtentry route;
        memset(&route, 0, sizeof(route));

        set_sockaddr_in(&route.rt_dst, "0.0.0.0");
        set_sockaddr_in(&route.rt_gateway, gateway);
        set_sockaddr_in(&route.rt_genmask, "0.0.0.0");
        route.rt_flags = RTF_UP | RTF_GATEWAY;
        strncpy(route.rt_dev, iface, 15);

        if (ioctl(fd, SIOCADDRT, &route) < 0) {
            qWarning("NM-Ethernet: SIOCADDRT failed on %s: %m", iface);
            /* Non-fatal: might already have a route */
        }
    }

    close(fd);

    if (success) {
        qDebug("NM-Ethernet: configured %s: %s/%s gw %s",
               iface, addr, netmask, gateway ? gateway : "(none)");
    }

    return success;
}

/* ========================================================================= */
/* DHCP                                                                      */
/* ========================================================================= */

bool nm_ethernet_trigger_dhcp(const char *iface, uint32_t timeout_sec)
{
    if (!iface)
        return false;

    int fd = open_ioctl_socket();
    if (fd < 0)
        return false;

    /* Bring interface up first */
    if (!bring_interface_up(fd, iface)) {
        close(fd);
        return false;
    }

    qDebug("NM-Ethernet: starting DHCP on %s (timeout %u sec)",
           iface, timeout_sec);

    /* Tell kernel to start DHCP discovery */
    struct {
        char ifname[16];
    } dhcp_req;
    memset(&dhcp_req, 0, sizeof(dhcp_req));
    strncpy(dhcp_req.ifname, iface, 15);

    if (ioctl(fd, VERIDIAN_DHCP_START, &dhcp_req) < 0) {
        qWarning("NM-Ethernet: DHCP start ioctl failed on %s: %m", iface);
        close(fd);
        return false;
    }

    /* Poll for DHCP completion */
    QElapsedTimer timer;
    timer.start();
    int64_t timeout_ms = (int64_t)timeout_sec * 1000;

    while (timer.elapsed() < timeout_ms) {
        usleep(DHCP_POLL_INTERVAL_MS * 1000);

        DhcpResult result;
        memset(&result, 0, sizeof(result));

        struct {
            char ifname[16];
            DhcpResult *result;
        } status_req;
        memset(&status_req, 0, sizeof(status_req));
        strncpy(status_req.ifname, iface, 15);
        status_req.result = &result;

        if (ioctl(fd, VERIDIAN_DHCP_STATUS, &status_req) < 0)
            continue;

        if (result.state == 2) {  /* bound */
            /* Apply the configuration */
            struct veridian_ifreq ifr;

            /* Set IP address */
            memset(&ifr, 0, sizeof(ifr));
            strncpy(ifr.ifr_name, iface, 15);
            set_sockaddr_in_raw(&ifr.ifr_addr, result.ip_addr);
            ioctl(fd, SIOCSIFADDR, &ifr);

            /* Set netmask */
            memset(&ifr, 0, sizeof(ifr));
            strncpy(ifr.ifr_name, iface, 15);
            set_sockaddr_in_raw(&ifr.ifr_netmask, result.netmask);
            ioctl(fd, SIOCSIFNETMASK, &ifr);

            /* Set default route */
            if (result.gateway != 0) {
                struct veridian_rtentry route;
                memset(&route, 0, sizeof(route));
                set_sockaddr_in(&route.rt_dst, "0.0.0.0");
                set_sockaddr_in_raw(&route.rt_gateway, result.gateway);
                set_sockaddr_in(&route.rt_genmask, "0.0.0.0");
                route.rt_flags = RTF_UP | RTF_GATEWAY;
                strncpy(route.rt_dev, iface, 15);
                ioctl(fd, SIOCADDRT, &route);
            }

            char ip_str[INET_ADDRSTRLEN];
            char gw_str[INET_ADDRSTRLEN];
            struct in_addr ip_in, gw_in;
            ip_in.s_addr = result.ip_addr;
            gw_in.s_addr = result.gateway;
            inet_ntop(AF_INET, &ip_in, ip_str, sizeof(ip_str));
            inet_ntop(AF_INET, &gw_in, gw_str, sizeof(gw_str));

            qDebug("NM-Ethernet: DHCP bound on %s: %s gw %s (lease %u sec)",
                   iface, ip_str, gw_str, result.lease_time);

            close(fd);
            return true;
        }

        if (result.state == 3) {  /* failed */
            qWarning("NM-Ethernet: DHCP failed on %s", iface);
            close(fd);
            return false;
        }
    }

    qWarning("NM-Ethernet: DHCP timeout on %s after %u sec",
             iface, timeout_sec);
    close(fd);
    return false;
}

/* ========================================================================= */
/* MAC address query                                                         */
/* ========================================================================= */

bool nm_ethernet_get_mac_address(const char *iface, char *buf, uint32_t len)
{
    if (!iface || !buf || len < 18)
        return false;

    QString path = QStringLiteral("/sys/class/net/%1/address")
                       .arg(QString::fromLatin1(iface));
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text)) {
        qWarning("NM-Ethernet: cannot read MAC for %s", iface);
        return false;
    }

    QByteArray mac = file.readAll().trimmed();
    if (mac.isEmpty())
        return false;

    size_t copyLen = (size_t)mac.size();
    if (copyLen >= len)
        copyLen = len - 1;
    memcpy(buf, mac.constData(), copyLen);
    buf[copyLen] = '\0';

    return true;
}

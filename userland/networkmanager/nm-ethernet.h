/*
 * VeridianOS -- nm-ethernet.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Ethernet backend for the NetworkManager shim.  Provides link state
 * detection, DHCP triggering, static IP configuration, and MAC address
 * queries via ioctl to the VeridianOS kernel network subsystem.
 */

#ifndef NM_ETHERNET_H
#define NM_ETHERNET_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Link state                                                                */
/* ========================================================================= */

/**
 * Detect link carrier state for an Ethernet interface.
 * Queries the kernel via SIOCGIFFLAGS ioctl.
 * Returns true if link is up (carrier detected).
 */
bool nm_ethernet_detect_link(const char *iface);

/* ========================================================================= */
/* IP configuration                                                          */
/* ========================================================================= */

/**
 * Set a static IPv4 address and netmask on the interface.
 * Uses SIOCSIFADDR/SIOCSIFNETMASK ioctls.
 * @param iface   Interface name (e.g. "eth0").
 * @param addr    IPv4 address string (e.g. "192.168.1.100").
 * @param netmask Subnet mask string (e.g. "255.255.255.0").
 * @param gateway Default gateway (NULL to skip route setup).
 */
bool nm_ethernet_set_ip(const char *iface, const char *addr,
                        const char *netmask, const char *gateway);

/**
 * Trigger DHCP on the interface.
 * Sends DHCPDISCOVER, processes DHCPOFFER, sends DHCPREQUEST,
 * and applies the configuration from DHCPACK.
 * Blocks until an address is obtained or the timeout expires.
 * @param iface       Interface name.
 * @param timeout_sec Maximum seconds to wait for a lease.
 */
bool nm_ethernet_trigger_dhcp(const char *iface, uint32_t timeout_sec);

/* ========================================================================= */
/* Hardware info                                                             */
/* ========================================================================= */

/**
 * Get the MAC address of an Ethernet interface.
 * Writes the MAC as "xx:xx:xx:xx:xx:xx" into @p buf.
 * @param iface Interface name.
 * @param buf   Output buffer (at least 18 bytes).
 * @param len   Buffer size.
 */
bool nm_ethernet_get_mac_address(const char *iface, char *buf, uint32_t len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* NM_ETHERNET_H */

/*
 * VeridianOS -- nm-dns.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * DNS configuration backend for the NetworkManager shim.  Manages
 * /etc/resolv.conf and per-connection DNS server lists.
 */

#ifndef NM_DNS_H
#define NM_DNS_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#define NM_DNS_MAX_SERVERS  8
#define NM_DNS_MAX_DOMAINS  8
#define NM_DNS_MAX_ADDR_LEN 46

/**
 * Set the system DNS configuration.
 * Writes /etc/resolv.conf with the given nameservers and search domains.
 * @param servers      Array of DNS server address strings.
 * @param server_count Number of servers (max NM_DNS_MAX_SERVERS).
 * @param domains      Array of search domain strings (may be NULL).
 * @param domain_count Number of search domains.
 */
bool nm_dns_set_servers(const char **servers, uint32_t server_count,
                        const char **domains, uint32_t domain_count);

/**
 * Flush the DNS resolver cache.
 * Called automatically when connections change.
 */
void nm_dns_flush_cache(void);

/**
 * Restore /etc/resolv.conf from backup (e.g. on NM shutdown).
 */
bool nm_dns_restore_resolv_conf(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* NM_DNS_H */

/*
 * VeridianOS -- nm-wifi.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Wi-Fi backend for the NetworkManager shim.  Interfaces with the
 * VeridianOS kernel's mac80211 layer (kernel/src/drivers/wifi/mac80211.rs)
 * and WPA2 supplicant (kernel/src/drivers/wifi/wpa.rs) to provide
 * scanning, authentication, and association.
 *
 * The backend maintains a cached list of access points and supports
 * periodic background scanning when disconnected.
 */

#ifndef NM_WIFI_H
#define NM_WIFI_H

#include <stdint.h>
#include <stdbool.h>

#include "nm-veridian.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Opaque backend handle                                                     */
/* ========================================================================= */

typedef struct NMWifiBackend NMWifiBackend;

/* ========================================================================= */
/* Wi-Fi backend lifecycle                                                   */
/* ========================================================================= */

/**
 * Create a new Wi-Fi backend for the given interface.
 * Returns NULL if the interface is not a Wi-Fi device.
 */
NMWifiBackend *nm_wifi_backend_new(const char *iface);

/**
 * Destroy the Wi-Fi backend and release resources.
 */
void nm_wifi_backend_destroy(NMWifiBackend *backend);

/* ========================================================================= */
/* Scanning                                                                  */
/* ========================================================================= */

/**
 * Trigger a Wi-Fi scan via the kernel mac80211 interface.
 * Non-blocking; results available via nm_wifi_get_scan_results().
 */
bool nm_wifi_scan(NMWifiBackend *backend);

/**
 * Retrieve the most recent scan results.
 */
bool nm_wifi_get_scan_results(NMWifiBackend *backend,
                              NMAccessPointList *out);

/**
 * Get signal strength of the currently associated AP.
 * Returns RSSI in dBm, or 0 if not connected.
 */
int32_t nm_wifi_get_signal_strength(NMWifiBackend *backend);

/**
 * Get the SSID of the currently associated network.
 * Returns NULL if not connected.  Returned pointer is valid until
 * the next connect/disconnect call.
 */
const char *nm_wifi_get_current_ssid(NMWifiBackend *backend);

/* ========================================================================= */
/* Connection management                                                     */
/* ========================================================================= */

/**
 * Connect to a Wi-Fi network.
 * @param ssid      Network SSID (NUL-terminated).
 * @param password  WPA2/WPA3 passphrase (NULL for open networks).
 *
 * Performs:
 *   1. Authenticate with the AP (open or WPA2 4-way handshake)
 *   2. Associate
 *   3. Request IP via DHCP (or apply static config)
 *
 * Blocks until the handshake completes or times out.
 */
bool nm_wifi_connect(NMWifiBackend *backend, const char *ssid,
                     const char *password);

/**
 * Disconnect from the current Wi-Fi network.
 * Sends a deauthentication frame and brings the interface down.
 */
bool nm_wifi_disconnect(NMWifiBackend *backend);

/* ========================================================================= */
/* Background scanning                                                       */
/* ========================================================================= */

/**
 * Start periodic background scanning (every interval_sec seconds).
 * Scanning is only active while disconnected.
 */
void nm_wifi_start_background_scan(NMWifiBackend *backend,
                                   uint32_t interval_sec);

/**
 * Stop periodic background scanning.
 */
void nm_wifi_stop_background_scan(NMWifiBackend *backend);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* NM_WIFI_H */

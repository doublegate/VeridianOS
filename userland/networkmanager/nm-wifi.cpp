/*
 * VeridianOS -- nm-wifi.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Wi-Fi backend implementation for the NetworkManager shim.
 *
 * Interfaces with the VeridianOS kernel's mac80211 layer for scanning
 * and the WPA2 supplicant for authentication.  Maintains a cached
 * list of visible access points and supports periodic background
 * scanning when disconnected.
 */

#include "nm-wifi.h"
#include "nm-ethernet.h"

#include <QDebug>
#include <QTimer>
#include <QElapsedTimer>

#include <cstring>
#include <cstdio>
#include <cstdlib>

#include <unistd.h>
#include <sys/ioctl.h>

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

/* ioctl commands for mac80211 (VeridianOS kernel) */
static const unsigned long VERIDIAN_WLAN_SCAN       = 0x8B01;
static const unsigned long VERIDIAN_WLAN_GET_SCAN   = 0x8B02;
static const unsigned long VERIDIAN_WLAN_ASSOCIATE  = 0x8B03;
static const unsigned long VERIDIAN_WLAN_DEAUTH     = 0x8B04;
static const unsigned long VERIDIAN_WLAN_GET_RSSI   = 0x8B05;
static const unsigned long VERIDIAN_WLAN_GET_SSID   = 0x8B06;
static const unsigned long VERIDIAN_WLAN_WPA_START  = 0x8B10;
static const unsigned long VERIDIAN_WLAN_WPA_STATUS = 0x8B11;

/* Timeouts */
static const int SCAN_TIMEOUT_MS       = 5000;
static const int HANDSHAKE_TIMEOUT_MS  = 10000;
static const int ASSOCIATE_TIMEOUT_MS  = 5000;

/* Maximum scan buffer size (kernel fills this) */
static const size_t SCAN_BUF_SIZE = 8192;

/* ========================================================================= */
/* Kernel scan result format                                                 */
/* ========================================================================= */

/* Matches struct wlan_scan_entry in kernel mac80211.rs */
struct KernelScanEntry {
    uint8_t  ssid[32];
    uint8_t  ssid_len;
    uint8_t  bssid[6];
    uint16_t frequency;     /* MHz */
    int16_t  rssi;          /* dBm */
    uint16_t capability;    /* IEEE 802.11 capability info */
    uint8_t  channel;
    uint32_t max_rate;      /* kbps */
    uint8_t  _padding[3];
};

/* Matches struct wlan_scan_result in kernel mac80211.rs */
struct KernelScanResult {
    uint32_t count;
    KernelScanEntry entries[128];
};

/* WPA handshake request to kernel */
struct KernelWpaRequest {
    uint8_t  ssid[32];
    uint8_t  ssid_len;
    uint8_t  bssid[6];
    uint8_t  passphrase[64];
    uint8_t  passphrase_len;
    uint8_t  _padding[1];
};

/* WPA handshake status */
struct KernelWpaStatus {
    uint8_t state;      /* 0=idle, 1=in_progress, 2=complete, 3=failed */
    uint8_t _padding[3];
};

/* ========================================================================= */
/* NMWifiBackend internal state                                              */
/* ========================================================================= */

struct NMWifiBackend {
    char interface_name[16];
    int  socket_fd;

    /* Cached scan results */
    NMAccessPoint ap_cache[NM_MAX_ACCESS_POINTS];
    uint32_t ap_count;

    /* Current connection */
    char current_ssid[NM_MAX_SSID_LEN];
    char current_bssid[NM_MAX_BSSID_LEN];
    int32_t current_rssi;
    bool connected;

    /* Background scan */
    QTimer *scan_timer;
    bool scan_active;
};

/* ========================================================================= */
/* Helper: convert RSSI to percentage (0-100)                                */
/* ========================================================================= */

static uint8_t rssi_to_percent(int16_t rssi)
{
    /* Typical range: -30 dBm (excellent) to -90 dBm (very weak) */
    if (rssi >= -30) return 100;
    if (rssi <= -90) return 0;
    /* Linear interpolation: (-30 - (-90)) = 60 dBm range */
    return (uint8_t)(((rssi + 90) * 100) / 60);
}

/* ========================================================================= */
/* Helper: decode security flags from capability bitmask                     */
/* ========================================================================= */

static uint32_t decode_security_flags(uint16_t capability)
{
    uint32_t flags = NM_WIFI_SEC_NONE;

    /* IEEE 802.11 capability info bits */
    if (capability & 0x0010)  /* Privacy bit -- WEP or better */
        flags |= NM_WIFI_SEC_WEP;

    /* RSN/WPA2 detection requires parsing IEs, which the kernel does
     * for us via the capability field extensions:
     *   bit 8: WPA-PSK
     *   bit 9: WPA2-PSK
     *   bit 10: WPA3-SAE
     *   bit 11: WPA-EAP
     *   bit 12: WPA2-EAP
     *   (VeridianOS mac80211 convention) */
    if (capability & 0x0100) flags |= NM_WIFI_SEC_WPA_PSK;
    if (capability & 0x0200) flags |= NM_WIFI_SEC_WPA2_PSK;
    if (capability & 0x0400) flags |= NM_WIFI_SEC_WPA3_SAE;
    if (capability & 0x0800) flags |= NM_WIFI_SEC_WPA_EAP;
    if (capability & 0x1000) flags |= NM_WIFI_SEC_WPA2_EAP;

    return flags;
}

/* ========================================================================= */
/* Helper: format BSSID                                                      */
/* ========================================================================= */

static void format_bssid(const uint8_t bssid[6], char *buf, size_t len)
{
    snprintf(buf, len, "%02x:%02x:%02x:%02x:%02x:%02x",
             bssid[0], bssid[1], bssid[2],
             bssid[3], bssid[4], bssid[5]);
}

/* ========================================================================= */
/* Helper: frequency to channel                                              */
/* ========================================================================= */

static uint8_t freq_to_channel(uint16_t freq)
{
    /* 2.4 GHz band */
    if (freq >= 2412 && freq <= 2484) {
        if (freq == 2484) return 14;
        return (uint8_t)((freq - 2412) / 5 + 1);
    }
    /* 5 GHz band */
    if (freq >= 5180 && freq <= 5825) {
        return (uint8_t)((freq - 5180) / 5 + 36);
    }
    /* 6 GHz band */
    if (freq >= 5955 && freq <= 7115) {
        return (uint8_t)((freq - 5955) / 5 + 1);
    }
    return 0;
}

/* ========================================================================= */
/* Helper: open wireless ioctl socket                                        */
/* ========================================================================= */

static int open_wlan_socket(const char *iface)
{
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (fd < 0) {
        qWarning("NM-WiFi: cannot open socket for %s: %m", iface);
        return -1;
    }
    return fd;
}

/* ========================================================================= */
/* Wi-Fi backend lifecycle                                                   */
/* ========================================================================= */

NMWifiBackend *nm_wifi_backend_new(const char *iface)
{
    if (!iface)
        return nullptr;

    NMWifiBackend *backend = new NMWifiBackend;
    memset(backend, 0, sizeof(*backend));

    strncpy(backend->interface_name, iface, 15);
    backend->interface_name[15] = '\0';
    backend->socket_fd = open_wlan_socket(iface);
    backend->connected = false;
    backend->scan_active = false;
    backend->ap_count = 0;
    backend->current_rssi = 0;
    backend->scan_timer = nullptr;

    qDebug("NM-WiFi: backend created for %s (fd=%d)",
           backend->interface_name, backend->socket_fd);

    return backend;
}

void nm_wifi_backend_destroy(NMWifiBackend *backend)
{
    if (!backend)
        return;

    nm_wifi_stop_background_scan(backend);

    if (backend->connected)
        nm_wifi_disconnect(backend);

    if (backend->socket_fd >= 0)
        close(backend->socket_fd);

    delete backend->scan_timer;
    delete backend;
}

/* ========================================================================= */
/* Scanning                                                                  */
/* ========================================================================= */

bool nm_wifi_scan(NMWifiBackend *backend)
{
    if (!backend || backend->socket_fd < 0)
        return false;

    qDebug("NM-WiFi: triggering scan on %s", backend->interface_name);

    /* Issue scan command to kernel mac80211 */
    struct {
        char ifname[16];
    } scan_req;
    memset(&scan_req, 0, sizeof(scan_req));
    strncpy(scan_req.ifname, backend->interface_name, 15);

    if (ioctl(backend->socket_fd, VERIDIAN_WLAN_SCAN, &scan_req) < 0) {
        qWarning("NM-WiFi: scan ioctl failed on %s: %m",
                 backend->interface_name);
        return false;
    }

    /* Wait for scan to complete (kernel signals via the same ioctl) */
    QElapsedTimer timeout;
    timeout.start();

    while (timeout.elapsed() < SCAN_TIMEOUT_MS) {
        usleep(100000);  /* 100ms polling interval */

        /* Fetch results */
        KernelScanResult result;
        memset(&result, 0, sizeof(result));

        struct {
            char ifname[16];
            KernelScanResult *buf;
            size_t buf_size;
        } get_req;
        memset(&get_req, 0, sizeof(get_req));
        strncpy(get_req.ifname, backend->interface_name, 15);
        get_req.buf = &result;
        get_req.buf_size = sizeof(result);

        if (ioctl(backend->socket_fd, VERIDIAN_WLAN_GET_SCAN, &get_req) < 0)
            continue;

        if (result.count == 0)
            continue;

        /* Convert kernel scan entries to NMAccessPoint */
        backend->ap_count = 0;
        for (uint32_t i = 0; i < result.count && i < NM_MAX_ACCESS_POINTS; ++i) {
            const KernelScanEntry *ke = &result.entries[i];
            NMAccessPoint *ap = &backend->ap_cache[backend->ap_count];

            memset(ap, 0, sizeof(*ap));

            /* SSID */
            size_t ssid_len = ke->ssid_len;
            if (ssid_len > NM_MAX_SSID_LEN - 1)
                ssid_len = NM_MAX_SSID_LEN - 1;
            memcpy(ap->ssid, ke->ssid, ssid_len);
            ap->ssid[ssid_len] = '\0';

            /* BSSID */
            format_bssid(ke->bssid, ap->bssid, NM_MAX_BSSID_LEN);

            ap->frequency = ke->frequency;
            ap->signal_strength = ke->rssi;
            ap->signal_percent = rssi_to_percent(ke->rssi);
            ap->security_flags = decode_security_flags(ke->capability);
            ap->max_bitrate = ke->max_rate;
            ap->channel = ke->channel ? ke->channel : freq_to_channel(ke->frequency);

            backend->ap_count++;
        }

        qDebug("NM-WiFi: scan complete on %s -- %u APs found",
               backend->interface_name, backend->ap_count);
        return true;
    }

    qWarning("NM-WiFi: scan timeout on %s", backend->interface_name);
    return false;
}

bool nm_wifi_get_scan_results(NMWifiBackend *backend, NMAccessPointList *out)
{
    if (!backend || !out)
        return false;

    memcpy(out->access_points, backend->ap_cache,
           sizeof(NMAccessPoint) * backend->ap_count);
    out->count = backend->ap_count;
    return true;
}

int32_t nm_wifi_get_signal_strength(NMWifiBackend *backend)
{
    if (!backend || !backend->connected)
        return 0;

    /* Query current RSSI from kernel */
    struct {
        char ifname[16];
        int32_t rssi;
    } rssi_req;
    memset(&rssi_req, 0, sizeof(rssi_req));
    strncpy(rssi_req.ifname, backend->interface_name, 15);

    if (ioctl(backend->socket_fd, VERIDIAN_WLAN_GET_RSSI, &rssi_req) == 0) {
        backend->current_rssi = rssi_req.rssi;
    }

    return backend->current_rssi;
}

const char *nm_wifi_get_current_ssid(NMWifiBackend *backend)
{
    if (!backend || !backend->connected)
        return nullptr;

    return backend->current_ssid;
}

/* ========================================================================= */
/* Connection management                                                     */
/* ========================================================================= */

bool nm_wifi_connect(NMWifiBackend *backend, const char *ssid,
                     const char *password)
{
    if (!backend || !ssid)
        return false;

    qDebug("NM-WiFi: connecting to '%s' on %s", ssid, backend->interface_name);

    /* Find the AP in scan cache to get BSSID */
    const NMAccessPoint *target_ap = nullptr;
    for (uint32_t i = 0; i < backend->ap_count; ++i) {
        if (strcmp(backend->ap_cache[i].ssid, ssid) == 0) {
            target_ap = &backend->ap_cache[i];
            break;
        }
    }

    /* If not in cache, do a quick scan first */
    if (!target_ap) {
        qDebug("NM-WiFi: AP '%s' not in cache, scanning...", ssid);
        nm_wifi_scan(backend);

        for (uint32_t i = 0; i < backend->ap_count; ++i) {
            if (strcmp(backend->ap_cache[i].ssid, ssid) == 0) {
                target_ap = &backend->ap_cache[i];
                break;
            }
        }
    }

    if (!target_ap) {
        qWarning("NM-WiFi: AP '%s' not found", ssid);
        return false;
    }

    /* Determine if authentication is needed */
    bool needs_auth = (target_ap->security_flags != NM_WIFI_SEC_NONE);

    if (needs_auth && !password) {
        qWarning("NM-WiFi: AP '%s' requires authentication but no password given",
                 ssid);
        return false;
    }

    /* Step 1: WPA2 4-way handshake (if secured) */
    if (needs_auth) {
        qDebug("NM-WiFi: starting WPA2 handshake with '%s'", ssid);

        KernelWpaRequest wpa_req;
        memset(&wpa_req, 0, sizeof(wpa_req));

        size_t ssid_len = strlen(ssid);
        if (ssid_len > 31) ssid_len = 31;
        memcpy(wpa_req.ssid, ssid, ssid_len);
        wpa_req.ssid_len = (uint8_t)ssid_len;

        /* Parse BSSID from string */
        unsigned int b[6];
        if (sscanf(target_ap->bssid, "%x:%x:%x:%x:%x:%x",
                   &b[0], &b[1], &b[2], &b[3], &b[4], &b[5]) == 6) {
            for (int i = 0; i < 6; ++i)
                wpa_req.bssid[i] = (uint8_t)b[i];
        }

        if (password) {
            size_t pw_len = strlen(password);
            if (pw_len > 63) pw_len = 63;
            memcpy(wpa_req.passphrase, password, pw_len);
            wpa_req.passphrase_len = (uint8_t)pw_len;
        }

        if (ioctl(backend->socket_fd, VERIDIAN_WLAN_WPA_START, &wpa_req) < 0) {
            qWarning("NM-WiFi: WPA handshake start failed: %m");
            return false;
        }

        /* Poll for handshake completion */
        QElapsedTimer timeout;
        timeout.start();
        bool handshake_ok = false;

        while (timeout.elapsed() < HANDSHAKE_TIMEOUT_MS) {
            usleep(200000);  /* 200ms poll */

            KernelWpaStatus status;
            memset(&status, 0, sizeof(status));

            struct {
                char ifname[16];
                KernelWpaStatus *status;
            } status_req;
            memset(&status_req, 0, sizeof(status_req));
            strncpy(status_req.ifname, backend->interface_name, 15);
            status_req.status = &status;

            if (ioctl(backend->socket_fd, VERIDIAN_WLAN_WPA_STATUS,
                       &status_req) < 0)
                continue;

            if (status.state == 2) {  /* complete */
                handshake_ok = true;
                break;
            }
            if (status.state == 3) {  /* failed */
                qWarning("NM-WiFi: WPA handshake failed for '%s'", ssid);
                return false;
            }
        }

        if (!handshake_ok) {
            qWarning("NM-WiFi: WPA handshake timeout for '%s'", ssid);
            return false;
        }

        qDebug("NM-WiFi: WPA handshake complete for '%s'", ssid);
    }

    /* Step 2: Associate with AP */
    {
        struct {
            char ifname[16];
            uint8_t bssid[6];
            uint8_t ssid[32];
            uint8_t ssid_len;
            uint8_t _padding[1];
        } assoc_req;
        memset(&assoc_req, 0, sizeof(assoc_req));
        strncpy(assoc_req.ifname, backend->interface_name, 15);

        size_t ssid_len = strlen(ssid);
        if (ssid_len > 31) ssid_len = 31;
        memcpy(assoc_req.ssid, ssid, ssid_len);
        assoc_req.ssid_len = (uint8_t)ssid_len;

        unsigned int b[6];
        if (sscanf(target_ap->bssid, "%x:%x:%x:%x:%x:%x",
                   &b[0], &b[1], &b[2], &b[3], &b[4], &b[5]) == 6) {
            for (int i = 0; i < 6; ++i)
                assoc_req.bssid[i] = (uint8_t)b[i];
        }

        if (ioctl(backend->socket_fd, VERIDIAN_WLAN_ASSOCIATE, &assoc_req) < 0) {
            qWarning("NM-WiFi: association failed for '%s': %m", ssid);
            return false;
        }
    }

    /* Update state */
    strncpy(backend->current_ssid, ssid, NM_MAX_SSID_LEN - 1);
    backend->current_ssid[NM_MAX_SSID_LEN - 1] = '\0';
    strncpy(backend->current_bssid, target_ap->bssid, NM_MAX_BSSID_LEN - 1);
    backend->current_bssid[NM_MAX_BSSID_LEN - 1] = '\0';
    backend->current_rssi = target_ap->signal_strength;
    backend->connected = true;

    /* Stop background scanning while connected */
    nm_wifi_stop_background_scan(backend);

    qDebug("NM-WiFi: connected to '%s' (%s) rssi=%d",
           ssid, target_ap->bssid, target_ap->signal_strength);

    return true;
}

bool nm_wifi_disconnect(NMWifiBackend *backend)
{
    if (!backend)
        return false;

    if (!backend->connected) {
        qDebug("NM-WiFi: already disconnected on %s", backend->interface_name);
        return true;
    }

    qDebug("NM-WiFi: disconnecting from '%s' on %s",
           backend->current_ssid, backend->interface_name);

    /* Send deauthentication frame */
    struct {
        char ifname[16];
    } deauth_req;
    memset(&deauth_req, 0, sizeof(deauth_req));
    strncpy(deauth_req.ifname, backend->interface_name, 15);

    if (ioctl(backend->socket_fd, VERIDIAN_WLAN_DEAUTH, &deauth_req) < 0) {
        qWarning("NM-WiFi: deauth ioctl failed on %s: %m",
                 backend->interface_name);
        /* Continue cleanup regardless */
    }

    /* Clear state */
    backend->connected = false;
    backend->current_ssid[0] = '\0';
    backend->current_bssid[0] = '\0';
    backend->current_rssi = 0;

    qDebug("NM-WiFi: disconnected from %s", backend->interface_name);
    return true;
}

/* ========================================================================= */
/* Background scanning                                                       */
/* ========================================================================= */

void nm_wifi_start_background_scan(NMWifiBackend *backend,
                                   uint32_t interval_sec)
{
    if (!backend)
        return;

    if (backend->scan_timer) {
        backend->scan_timer->stop();
        delete backend->scan_timer;
    }

    backend->scan_timer = new QTimer();
    QObject::connect(backend->scan_timer, &QTimer::timeout,
                     [backend]() {
                         if (!backend->connected) {
                             nm_wifi_scan(backend);
                         }
                     });

    uint32_t interval_ms = interval_sec * 1000;
    backend->scan_timer->start((int)interval_ms);
    backend->scan_active = true;

    qDebug("NM-WiFi: background scanning started on %s (every %u sec)",
           backend->interface_name, interval_sec);
}

void nm_wifi_stop_background_scan(NMWifiBackend *backend)
{
    if (!backend || !backend->scan_timer)
        return;

    backend->scan_timer->stop();
    backend->scan_active = false;

    qDebug("NM-WiFi: background scanning stopped on %s",
           backend->interface_name);
}

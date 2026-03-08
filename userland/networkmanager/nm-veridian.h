/*
 * VeridianOS -- nm-veridian.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * NetworkManager D-Bus API shim for VeridianOS.  Provides the core
 * NetworkManager-compatible interface that Plasma's network management
 * applet (plasma-nm) expects.
 *
 * Responsibilities:
 *   - Network device enumeration and state tracking
 *   - Connection profile management (add/remove/activate/deactivate)
 *   - Wi-Fi access point scanning and association
 *   - Ethernet link detection and DHCP
 *   - IP address, route, and DNS configuration
 *   - D-Bus service at org.freedesktop.NetworkManager
 *   - Auto-connect on startup for saved connections
 *
 * On VeridianOS, network configuration uses the kernel's netlink-style
 * IPC (kernel/src/net/netlink.rs) and delegates Wi-Fi operations to
 * the mac80211/wpa kernel subsystems (kernel/src/drivers/wifi/).
 *
 * Object tree:
 *   /org/freedesktop/NetworkManager
 *   /org/freedesktop/NetworkManager/Devices/<n>
 *   /org/freedesktop/NetworkManager/ActiveConnection/<n>
 *   /org/freedesktop/NetworkManager/Settings
 *   /org/freedesktop/NetworkManager/Settings/<n>
 */

#ifndef NM_VERIDIAN_H
#define NM_VERIDIAN_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Forward declarations                                                      */
/* ========================================================================= */

typedef struct NMClient NMClient;

/* ========================================================================= */
/* NMState -- global network connectivity state                              */
/* ========================================================================= */

typedef enum {
    NM_STATE_UNKNOWN          = 0,
    NM_STATE_ASLEEP           = 10,
    NM_STATE_DISCONNECTED     = 20,
    NM_STATE_DISCONNECTING    = 30,
    NM_STATE_CONNECTING       = 40,
    NM_STATE_CONNECTED_LOCAL  = 50,
    NM_STATE_CONNECTED_SITE   = 60,
    NM_STATE_CONNECTED_GLOBAL = 70
} NMState;

/* ========================================================================= */
/* NMDeviceState -- per-device state machine                                 */
/* ========================================================================= */

typedef enum {
    NM_DEVICE_STATE_UNKNOWN       = 0,
    NM_DEVICE_STATE_UNMANAGED     = 10,
    NM_DEVICE_STATE_UNAVAILABLE   = 20,
    NM_DEVICE_STATE_DISCONNECTED  = 30,
    NM_DEVICE_STATE_PREPARE       = 40,
    NM_DEVICE_STATE_CONFIG        = 50,
    NM_DEVICE_STATE_NEED_AUTH     = 60,
    NM_DEVICE_STATE_IP_CONFIG     = 70,
    NM_DEVICE_STATE_IP_CHECK      = 80,
    NM_DEVICE_STATE_SECONDARIES   = 90,
    NM_DEVICE_STATE_ACTIVATED     = 100,
    NM_DEVICE_STATE_DEACTIVATING  = 110,
    NM_DEVICE_STATE_FAILED        = 120
} NMDeviceState;

/* ========================================================================= */
/* NMDeviceType -- network interface classification                          */
/* ========================================================================= */

typedef enum {
    NM_DEVICE_TYPE_UNKNOWN   = 0,
    NM_DEVICE_TYPE_ETHERNET  = 1,
    NM_DEVICE_TYPE_WIFI      = 2,
    NM_DEVICE_TYPE_BLUETOOTH = 5,
    NM_DEVICE_TYPE_BRIDGE    = 13,
    NM_DEVICE_TYPE_BOND      = 14,
    NM_DEVICE_TYPE_VLAN      = 15,
    NM_DEVICE_TYPE_LOOPBACK  = 16
} NMDeviceType;

/* ========================================================================= */
/* NMSecurityFlags -- Wi-Fi security capabilities                            */
/* ========================================================================= */

typedef enum {
    NM_WIFI_SEC_NONE         = 0x00,
    NM_WIFI_SEC_WEP          = 0x01,
    NM_WIFI_SEC_WPA_PSK      = 0x02,
    NM_WIFI_SEC_WPA2_PSK     = 0x04,
    NM_WIFI_SEC_WPA3_SAE     = 0x08,
    NM_WIFI_SEC_WPA_EAP      = 0x10,
    NM_WIFI_SEC_WPA2_EAP     = 0x20,
    NM_WIFI_SEC_OWE          = 0x40
} NMWifiSecurityFlags;

/* ========================================================================= */
/* NMDevice -- network interface descriptor                                  */
/* ========================================================================= */

#define NM_MAX_IFACE_NAME    16
#define NM_MAX_HWADDR_LEN    18   /* "xx:xx:xx:xx:xx:xx\0" */
#define NM_MAX_DRIVER_NAME   32
#define NM_MAX_DEVICES       16
#define NM_MAX_CONNECTIONS   64
#define NM_MAX_ACCESS_POINTS 128
#define NM_MAX_DNS_SERVERS   8
#define NM_MAX_ADDRESSES     8

typedef struct {
    char interface_name[NM_MAX_IFACE_NAME];
    char hw_address[NM_MAX_HWADDR_LEN];
    char driver[NM_MAX_DRIVER_NAME];
    NMDeviceType type;
    NMDeviceState state;
    uint32_t device_index;      /* kernel ifindex */
    uint32_t speed_mbps;        /* link speed, 0 if unknown */
    bool managed;
    bool autoconnect;
    int32_t active_connection;  /* index or -1 */
} NMDevice;

/* ========================================================================= */
/* NMActiveConnection -- currently active network connections                */
/* ========================================================================= */

#define NM_MAX_UUID_LEN      37  /* "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx\0" */
#define NM_MAX_CONN_TYPE     32
#define NM_MAX_CONN_ID       64

typedef enum {
    NM_ACTIVE_STATE_UNKNOWN      = 0,
    NM_ACTIVE_STATE_ACTIVATING   = 1,
    NM_ACTIVE_STATE_ACTIVATED    = 2,
    NM_ACTIVE_STATE_DEACTIVATING = 3,
    NM_ACTIVE_STATE_DEACTIVATED  = 4
} NMActiveConnectionState;

typedef struct {
    char uuid[NM_MAX_UUID_LEN];
    char type[NM_MAX_CONN_TYPE];
    char id[NM_MAX_CONN_ID];
    uint32_t device_index;
    NMActiveConnectionState state;
    bool is_default;            /* default route */
    bool is_vpn;
} NMActiveConnection;

/* ========================================================================= */
/* NMSettingsConnection -- saved connection profiles                         */
/* ========================================================================= */

#define NM_MAX_SETTINGS_KEY  64
#define NM_MAX_SETTINGS_VAL  256
#define NM_MAX_SETTINGS_KV   32

typedef struct {
    char key[NM_MAX_SETTINGS_KEY];
    char value[NM_MAX_SETTINGS_VAL];
} NMSettingEntry;

typedef struct {
    char id[NM_MAX_CONN_ID];
    char uuid[NM_MAX_UUID_LEN];
    char type[NM_MAX_CONN_TYPE];
    bool autoconnect;
    uint32_t timestamp;         /* last-used epoch */
    NMSettingEntry entries[NM_MAX_SETTINGS_KV];
    uint32_t entry_count;
} NMSettingsConnection;

/* ========================================================================= */
/* NMAccessPoint -- Wi-Fi scan result                                        */
/* ========================================================================= */

#define NM_MAX_SSID_LEN      33  /* 32 bytes + NUL */
#define NM_MAX_BSSID_LEN     18

typedef struct {
    char ssid[NM_MAX_SSID_LEN];
    char bssid[NM_MAX_BSSID_LEN];
    uint32_t frequency;          /* MHz */
    int32_t signal_strength;     /* dBm, typically -30 to -90 */
    uint8_t signal_percent;      /* 0-100 normalized */
    uint32_t security_flags;     /* bitmask of NMWifiSecurityFlags */
    uint32_t max_bitrate;        /* kbit/s */
    uint8_t channel;
} NMAccessPoint;

/* ========================================================================= */
/* NMIPConfig -- IP configuration for an active connection                   */
/* ========================================================================= */

#define NM_MAX_ADDR_STR      46  /* INET6_ADDRSTRLEN */
#define NM_MAX_SEARCH_DOMAIN 64

typedef struct {
    char address[NM_MAX_ADDR_STR];
    uint8_t prefix_length;
} NMIPAddress;

typedef struct {
    NMIPAddress addresses[NM_MAX_ADDRESSES];
    uint32_t address_count;
    char gateway[NM_MAX_ADDR_STR];
    char dns_servers[NM_MAX_DNS_SERVERS][NM_MAX_ADDR_STR];
    uint32_t dns_count;
    char search_domains[NM_MAX_DNS_SERVERS][NM_MAX_SEARCH_DOMAIN];
    uint32_t search_count;
} NMIPConfig;

/* ========================================================================= */
/* NMConnectionSettings -- key-value map for creating/editing profiles       */
/* ========================================================================= */

typedef struct {
    NMSettingEntry entries[NM_MAX_SETTINGS_KV];
    uint32_t count;
} NMConnectionSettings;

/* ========================================================================= */
/* Device list / AP list result types                                        */
/* ========================================================================= */

typedef struct {
    NMDevice devices[NM_MAX_DEVICES];
    uint32_t count;
} NMDeviceList;

typedef struct {
    NMActiveConnection connections[NM_MAX_CONNECTIONS];
    uint32_t count;
} NMActiveConnectionList;

typedef struct {
    NMAccessPoint access_points[NM_MAX_ACCESS_POINTS];
    uint32_t count;
} NMAccessPointList;

typedef struct {
    NMSettingsConnection connections[NM_MAX_CONNECTIONS];
    uint32_t count;
} NMSettingsConnectionList;

/* ========================================================================= */
/* Client lifecycle                                                          */
/* ========================================================================= */

/**
 * Create a new NetworkManager client instance.
 *
 * Connects to the NM D-Bus service and initialises local state.
 * Returns NULL on failure (e.g. D-Bus not available).
 */
NMClient *nm_client_new(void);

/**
 * Destroy a client and free all associated resources.
 */
void nm_client_destroy(NMClient *client);

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

/**
 * Query global network connectivity state.
 */
NMState nm_client_get_state(NMClient *client);

/**
 * Enable or disable networking globally.
 */
bool nm_client_set_networking_enabled(NMClient *client, bool enabled);

/**
 * Check whether networking is currently enabled.
 */
bool nm_client_get_networking_enabled(NMClient *client);

/**
 * Enable or disable Wi-Fi globally.
 */
bool nm_client_set_wireless_enabled(NMClient *client, bool enabled);

/**
 * Check whether Wi-Fi is currently enabled.
 */
bool nm_client_get_wireless_enabled(NMClient *client);

/* ========================================================================= */
/* Device enumeration                                                        */
/* ========================================================================= */

/**
 * Enumerate all known network devices.
 */
bool nm_client_get_devices(NMClient *client, NMDeviceList *out);

/**
 * Get the type of a device by interface name.
 */
NMDeviceType nm_device_get_type(NMClient *client, const char *iface);

/**
 * Get the state of a device by interface name.
 */
NMDeviceState nm_device_get_state(NMClient *client, const char *iface);

/**
 * Get IP configuration for a device.
 */
bool nm_device_get_ip_config(NMClient *client, const char *iface,
                             NMIPConfig *out);

/* ========================================================================= */
/* Active connections                                                        */
/* ========================================================================= */

/**
 * List all currently active connections.
 */
bool nm_client_get_active_connections(NMClient *client,
                                      NMActiveConnectionList *out);

/**
 * Activate a saved connection on a device.
 * @param uuid  Connection profile UUID (NULL to auto-pick).
 * @param iface Device interface name.
 */
bool nm_client_activate_connection(NMClient *client, const char *uuid,
                                   const char *iface);

/**
 * Deactivate an active connection.
 */
bool nm_client_deactivate_connection(NMClient *client, const char *uuid);

/* ========================================================================= */
/* Settings (saved connection profiles)                                      */
/* ========================================================================= */

/**
 * List all saved connection profiles.
 */
bool nm_settings_list_connections(NMClient *client,
                                  NMSettingsConnectionList *out);

/**
 * Add a new connection profile.
 * Returns the UUID of the new connection on success (static buffer).
 */
const char *nm_settings_add_connection(NMClient *client,
                                       const NMConnectionSettings *settings);

/**
 * Delete a saved connection profile by UUID.
 */
bool nm_settings_delete_connection(NMClient *client, const char *uuid);

/* ========================================================================= */
/* Wi-Fi                                                                     */
/* ========================================================================= */

/**
 * Get cached scan results for a Wi-Fi interface.
 */
bool nm_wifi_get_access_points(NMClient *client, const char *iface,
                               NMAccessPointList *out);

/**
 * Trigger a new Wi-Fi scan on the given interface.
 * Non-blocking; poll nm_wifi_get_access_points() for results.
 */
bool nm_wifi_request_scan(NMClient *client, const char *iface);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* NM_VERIDIAN_H */

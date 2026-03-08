/*
 * VeridianOS -- bluez-veridian.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * BlueZ D-Bus API shim for VeridianOS.  Provides a BlueZ 5.x-compatible
 * interface that KDE's Bluedevil (Plasma Bluetooth applet) expects.
 *
 * Responsibilities:
 *   - Bluetooth adapter management (power, discovery, pairing mode)
 *   - Device discovery, pairing, and connection lifecycle
 *   - GATT service registration (stub for BLE applications)
 *   - D-Bus service at org.bluez with ObjectManager interface
 *   - Pairing agent delegation for PIN/passkey entry
 *
 * On VeridianOS, Bluetooth hardware access uses the kernel HCI driver
 * (kernel/src/drivers/bluetooth/hci.rs) via the /dev/bluetooth/hci0
 * device node (kernel/src/drivers/bluetooth/device_node.rs).
 *
 * Object tree:
 *   /org/bluez                                 (AgentManager1, ProfileManager1)
 *   /org/bluez/hci0                            (Adapter1, GattManager1)
 *   /org/bluez/hci0/dev_XX_XX_XX_XX_XX_XX      (Device1 per device)
 */

#ifndef BLUEZ_VERIDIAN_H
#define BLUEZ_VERIDIAN_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

#define BT_MAX_NAME_LEN       249  /* Bluetooth spec max name length + NUL */
#define BT_MAX_ALIAS_LEN      128
#define BT_MAX_DEVICES        32
#define BT_MAX_UUIDS          16
#define BT_UUID_STR_LEN       37   /* "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx\0" */
#define BT_MAX_GATT_SERVICES  8
#define BT_MAX_GATT_CHARS     16
#define BT_MAX_GATT_VALUE_LEN 512
#define BT_ADDR_LEN           6

/* ========================================================================= */
/* BtAdapterState -- Bluetooth adapter power/discovery state                 */
/* ========================================================================= */

typedef enum {
    BT_ADAPTER_OFF         = 0,
    BT_ADAPTER_ON          = 1,
    BT_ADAPTER_DISCOVERING = 2
} BtAdapterState;

/* ========================================================================= */
/* BtDeviceType -- Bluetooth transport classification                        */
/* ========================================================================= */

typedef enum {
    BT_DEVICE_TYPE_UNKNOWN = 0,
    BT_DEVICE_TYPE_BR_EDR  = 1,   /* Classic Bluetooth (BR/EDR) */
    BT_DEVICE_TYPE_LE      = 2,   /* Bluetooth Low Energy */
    BT_DEVICE_TYPE_DUAL    = 3    /* Dual-mode (BR/EDR + LE) */
} BtDeviceType;

/* ========================================================================= */
/* BtPairState -- device pairing state                                       */
/* ========================================================================= */

typedef enum {
    BT_PAIR_NONE    = 0,
    BT_PAIR_PAIRING = 1,
    BT_PAIR_PAIRED  = 2,
    BT_PAIR_BONDED  = 3
} BtPairState;

/* ========================================================================= */
/* BtTransportState -- connection transport state                            */
/* ========================================================================= */

typedef enum {
    BT_TRANSPORT_IDLE    = 0,
    BT_TRANSPORT_PENDING = 1,
    BT_TRANSPORT_ACTIVE  = 2
} BtTransportState;

/* ========================================================================= */
/* BtAgentCapability -- I/O capabilities for pairing                         */
/* ========================================================================= */

typedef enum {
    BT_AGENT_CAP_DISPLAY_ONLY      = 0,
    BT_AGENT_CAP_DISPLAY_YES_NO    = 1,
    BT_AGENT_CAP_KEYBOARD_ONLY     = 2,
    BT_AGENT_CAP_NO_INPUT_NO_OUTPUT = 3,
    BT_AGENT_CAP_KEYBOARD_DISPLAY  = 4
} BtAgentCapability;

/* ========================================================================= */
/* BtAdapter -- local Bluetooth adapter                                      */
/* ========================================================================= */

typedef struct {
    uint8_t address[BT_ADDR_LEN];
    char name[BT_MAX_NAME_LEN];
    char alias[BT_MAX_ALIAS_LEN];
    bool powered;
    bool discovering;
    bool pairable;
    uint32_t pairable_timeout;     /* seconds, 0 = infinite */
    uint32_t discoverable_timeout; /* seconds, 0 = infinite */
    uint32_t device_class;         /* CoD (Class of Device) */
    BtAdapterState state;
} BtAdapter;

/* ========================================================================= */
/* BtDevice -- remote Bluetooth device                                       */
/* ========================================================================= */

typedef struct {
    uint8_t address[BT_ADDR_LEN];
    char name[BT_MAX_NAME_LEN];
    char alias[BT_MAX_ALIAS_LEN];
    bool paired;
    bool connected;
    bool trusted;
    bool blocked;
    bool legacy_pairing;
    int16_t rssi;                 /* signal strength in dBm */
    int16_t tx_power;             /* transmit power in dBm */
    BtDeviceType device_type;
    BtPairState pair_state;
    BtTransportState transport_state;
    uint32_t device_class;        /* CoD */
    uint16_t appearance;          /* BLE appearance */
    char uuids[BT_MAX_UUIDS][BT_UUID_STR_LEN];
    uint32_t uuid_count;
    char adapter_path[64];        /* D-Bus path of owning adapter */
} BtDevice;

/* ========================================================================= */
/* BtAgent -- pairing agent descriptor                                       */
/* ========================================================================= */

typedef struct {
    BtAgentCapability capability;
    bool registered;
    char object_path[128];        /* D-Bus object path of the agent */
} BtAgent;

/* ========================================================================= */
/* BtGattCharacteristic -- GATT characteristic                               */
/* ========================================================================= */

typedef struct {
    char uuid[BT_UUID_STR_LEN];
    uint32_t flags;               /* bitmask: read, write, notify, indicate */
    uint8_t value[BT_MAX_GATT_VALUE_LEN];
    uint32_t value_len;
} BtGattCharacteristic;

/* GATT characteristic flags */
#define BT_GATT_FLAG_READ      0x01
#define BT_GATT_FLAG_WRITE     0x02
#define BT_GATT_FLAG_NOTIFY    0x04
#define BT_GATT_FLAG_INDICATE  0x08
#define BT_GATT_FLAG_WRITE_NO_RESP 0x10

/* ========================================================================= */
/* BtGattService -- GATT service                                             */
/* ========================================================================= */

typedef struct {
    char uuid[BT_UUID_STR_LEN];
    bool primary;
    BtGattCharacteristic characteristics[BT_MAX_GATT_CHARS];
    uint32_t char_count;
} BtGattService;

/* ========================================================================= */
/* Device and service list result types                                      */
/* ========================================================================= */

typedef struct {
    BtDevice devices[BT_MAX_DEVICES];
    uint32_t count;
} BtDeviceList;

typedef struct {
    BtGattService services[BT_MAX_GATT_SERVICES];
    uint32_t count;
} BtGattServiceList;

/* ========================================================================= */
/* Pairing agent callback types                                              */
/* ========================================================================= */

/**
 * Callback: request a PIN code from the user.
 * @param address  Remote device BD_ADDR (6 bytes).
 * @param pin_out  Buffer to write the PIN string.
 * @param maxlen   Maximum PIN length.
 * @return true if PIN was provided, false to cancel.
 */
typedef bool (*bt_pin_request_fn)(const uint8_t *address, char *pin_out,
                                   uint32_t maxlen);

/**
 * Callback: confirm a 6-digit numeric passkey.
 * @param address  Remote device BD_ADDR (6 bytes).
 * @param passkey  The passkey to confirm.
 * @return true to accept, false to reject.
 */
typedef bool (*bt_confirm_passkey_fn)(const uint8_t *address, uint32_t passkey);

/**
 * Callback: display a passkey for the user to enter on the remote device.
 * @param address  Remote device BD_ADDR (6 bytes).
 * @param passkey  The passkey to display.
 */
typedef void (*bt_display_passkey_fn)(const uint8_t *address, uint32_t passkey);

/* ========================================================================= */
/* Adapter1 interface -- adapter management                                  */
/* ========================================================================= */

/**
 * Get adapter information (address, name, state, etc.).
 * Fills the provided BtAdapter structure.
 * Returns true on success.
 */
bool bt_adapter_get(BtAdapter *out);

/**
 * Power the adapter on or off.
 */
bool bt_adapter_set_powered(bool powered);

/**
 * Start device discovery (inquiry scan).
 * Discovered devices are collected in the device registry.
 */
bool bt_adapter_start_discovery(void);

/**
 * Stop device discovery.
 */
bool bt_adapter_stop_discovery(void);

/**
 * Enable or disable pairable mode.
 */
bool bt_adapter_set_pairable(bool pairable);

/**
 * Remove a device from the adapter's known device list.
 * Also removes stored bonding keys.
 * @param address  BD_ADDR of the device to remove (6 bytes).
 */
bool bt_adapter_remove_device(const uint8_t *address);

/* ========================================================================= */
/* Device1 interface -- remote device management                             */
/* ========================================================================= */

/**
 * Initiate a connection to a remote device.
 * @param address  BD_ADDR (6 bytes).
 */
bool bt_device_connect(const uint8_t *address);

/**
 * Disconnect from a remote device.
 * @param address  BD_ADDR (6 bytes).
 */
bool bt_device_disconnect(const uint8_t *address);

/**
 * Initiate pairing with a remote device.
 * @param address  BD_ADDR (6 bytes).
 */
bool bt_device_pair(const uint8_t *address);

/**
 * Cancel an in-progress pairing attempt.
 * @param address  BD_ADDR (6 bytes).
 */
bool bt_device_cancel_pairing(const uint8_t *address);

/**
 * Mark a device as trusted (auto-connect allowed).
 * @param address  BD_ADDR (6 bytes).
 */
bool bt_device_set_trusted(const uint8_t *address, bool trusted);

/**
 * Block or unblock a device.
 * @param address  BD_ADDR (6 bytes).
 */
bool bt_device_set_blocked(const uint8_t *address, bool blocked);

/* ========================================================================= */
/* AgentManager1 interface -- pairing agent registration                     */
/* ========================================================================= */

/**
 * Register a pairing agent with the given I/O capability.
 * @param capability  Agent's I/O capability.
 * @param object_path D-Bus object path for the agent.
 */
bool bt_agent_register(BtAgentCapability capability, const char *object_path);

/**
 * Unregister the previously registered agent.
 * @param object_path D-Bus object path of the agent.
 */
bool bt_agent_unregister(const char *object_path);

/**
 * Request that the registered agent become the default agent.
 * @param object_path D-Bus object path of the agent.
 */
bool bt_agent_request_default(const char *object_path);

/* ========================================================================= */
/* GattManager1 interface -- BLE GATT application registration (stub)        */
/* ========================================================================= */

/**
 * Register a GATT application (set of services).
 * @param object_path D-Bus object path root for the application.
 * @return true on success (currently a stub, always returns true).
 */
bool bt_gatt_register_application(const char *object_path);

/* ========================================================================= */
/* ObjectManager interface -- device enumeration                             */
/* ========================================================================= */

/**
 * Get all managed objects (adapter + devices).
 * Returns the number of objects written to device_list.
 * The adapter is implicitly included.
 */
bool bt_get_managed_objects(BtDeviceList *out);

/**
 * Get the current list of known (discovered or paired) devices.
 */
bool bt_get_devices(BtDeviceList *out);

/* ========================================================================= */
/* Daemon lifecycle                                                          */
/* ========================================================================= */

/**
 * Initialize the BlueZ shim daemon.
 * Opens HCI device, registers D-Bus service, and starts event processing.
 * Returns true on success.
 */
bool bt_init(void);

/**
 * Shut down the BlueZ shim daemon and release resources.
 */
void bt_cleanup(void);

/**
 * Set the pairing agent callbacks.
 * Must be called before pairing operations.
 */
void bt_set_agent_callbacks(bt_pin_request_fn pin_cb,
                             bt_confirm_passkey_fn confirm_cb,
                             bt_display_passkey_fn display_cb);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* BLUEZ_VERIDIAN_H */

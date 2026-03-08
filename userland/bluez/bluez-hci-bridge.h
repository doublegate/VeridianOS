/*
 * VeridianOS -- bluez-hci-bridge.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * HCI bridge -- thin abstraction over the kernel's /dev/bluetooth/hci0
 * device node.  Constructs HCI command packets, sends them to the kernel
 * HCI layer, and parses HCI event responses.
 *
 * Used by the BlueZ shim (bluez-veridian.cpp) to communicate with the
 * Bluetooth controller without needing raw socket access.
 */

#ifndef BLUEZ_HCI_BRIDGE_H
#define BLUEZ_HCI_BRIDGE_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Opaque handle                                                             */
/* ========================================================================= */

typedef struct HciBridge HciBridge;

/* ========================================================================= */
/* HCI command opcodes (subset used by the BlueZ shim)                       */
/* ========================================================================= */

#define HCI_OP_INQUIRY              0x0401
#define HCI_OP_INQUIRY_CANCEL       0x0402
#define HCI_OP_CREATE_CONNECTION    0x0405
#define HCI_OP_DISCONNECT           0x0406
#define HCI_OP_LINK_KEY_REQ_REPLY   0x040B
#define HCI_OP_PIN_CODE_REQ_REPLY   0x040D
#define HCI_OP_READ_LOCAL_NAME      0x0C14
#define HCI_OP_WRITE_SCAN_ENABLE    0x0C1A
#define HCI_OP_READ_BD_ADDR         0x1009

/* ========================================================================= */
/* HCI event codes                                                           */
/* ========================================================================= */

#define HCI_EVT_INQUIRY_COMPLETE       0x01
#define HCI_EVT_INQUIRY_RESULT         0x02
#define HCI_EVT_CONNECTION_COMPLETE    0x03
#define HCI_EVT_DISCONNECTION_COMPLETE 0x05
#define HCI_EVT_PIN_CODE_REQUEST       0x16
#define HCI_EVT_LINK_KEY_REQUEST       0x17
#define HCI_EVT_LINK_KEY_NOTIFICATION  0x18
#define HCI_EVT_COMMAND_COMPLETE       0x0E
#define HCI_EVT_COMMAND_STATUS         0x0F

/* ========================================================================= */
/* Lifecycle                                                                 */
/* ========================================================================= */

/**
 * Allocate a new HCI bridge instance.
 * Returns NULL on allocation failure.
 */
HciBridge *hci_bridge_new(void);

/**
 * Open the kernel HCI device.
 * @param bridge    Bridge instance.
 * @param dev_path  Device path (e.g. "/dev/bluetooth/hci0").
 * @return true on success.
 */
bool hci_bridge_open(HciBridge *bridge, const char *dev_path);

/**
 * Close the HCI device and release resources.
 */
void hci_bridge_close(HciBridge *bridge);

/**
 * Destroy a bridge instance (calls close if needed).
 */
void hci_bridge_destroy(HciBridge *bridge);

/* ========================================================================= */
/* Command / Event                                                           */
/* ========================================================================= */

/**
 * Send an HCI command packet to the controller.
 * @param bridge   Bridge instance.
 * @param opcode   HCI opcode (OGF:OCF).
 * @param params   Command parameters (may be NULL if len == 0).
 * @param len      Parameter length in bytes.
 * @return true on success.
 */
bool hci_bridge_send_command(HciBridge *bridge, uint16_t opcode,
                              const uint8_t *params, uint8_t len);

/**
 * Receive the next HCI event from the controller.
 * Blocks until an event is available or the timeout expires.
 * @param bridge   Bridge instance.
 * @param buf      Buffer to receive the event (event_code + param_len + params).
 * @param maxlen   Maximum buffer size.
 * @return Number of bytes written to buf, or -1 on error/timeout.
 */
int hci_bridge_recv_event(HciBridge *bridge, uint8_t *buf, uint32_t maxlen);

/* ========================================================================= */
/* Convenience wrappers                                                      */
/* ========================================================================= */

/**
 * Start inquiry scan.
 * @param bridge         Bridge instance.
 * @param duration_secs  Inquiry duration (clamped to 1-30).
 * @return true if the inquiry command was accepted.
 */
bool hci_bridge_start_inquiry(HciBridge *bridge, uint8_t duration_secs);

/**
 * Cancel a running inquiry scan.
 */
bool hci_bridge_cancel_inquiry(HciBridge *bridge);

/**
 * Read the local BD_ADDR from the controller.
 * @param bridge    Bridge instance.
 * @param addr_out  6-byte buffer for the address.
 * @return true on success.
 */
bool hci_bridge_get_local_address(HciBridge *bridge, uint8_t *addr_out);

/**
 * Read the local device name from the controller.
 * @param bridge    Bridge instance.
 * @param name_out  Buffer for the NUL-terminated name string.
 * @param maxlen    Maximum buffer size.
 * @return true on success.
 */
bool hci_bridge_get_local_name(HciBridge *bridge, char *name_out,
                                uint32_t maxlen);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* BLUEZ_HCI_BRIDGE_H */

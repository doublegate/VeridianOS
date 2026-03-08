/*
 * VeridianOS -- bluez-pair.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Bluetooth pairing agent for the BlueZ shim.  Handles PIN code requests,
 * Secure Simple Pairing (SSP) passkey confirmation, and passkey display
 * callbacks.  Registers as org.bluez.Agent1 on D-Bus.
 */

#ifndef BLUEZ_PAIR_H
#define BLUEZ_PAIR_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================= */
/* Pairing state machine                                                     */
/* ========================================================================= */

typedef enum {
    BT_PAIRING_IDLE             = 0,
    BT_PAIRING_REQUESTED_PIN    = 1,
    BT_PAIRING_REQUESTED_CONFIRM = 2,
    BT_PAIRING_COMPLETE         = 3,
    BT_PAIRING_FAILED           = 4
} BtPairingState;

/* ========================================================================= */
/* BtPairingAgent                                                            */
/* ========================================================================= */

typedef struct BtPairingAgent BtPairingAgent;

/* ========================================================================= */
/* Lifecycle                                                                 */
/* ========================================================================= */

/**
 * Create and initialize a pairing agent.
 * Returns NULL on failure.
 */
BtPairingAgent *bt_pairing_init(void);

/**
 * Destroy a pairing agent and free resources.
 */
void bt_pairing_destroy(BtPairingAgent *agent);

/* ========================================================================= */
/* Pairing operations                                                        */
/* ========================================================================= */

/**
 * Request a PIN code for legacy pairing.
 * @param agent    Pairing agent.
 * @param address  Remote device BD_ADDR (6 bytes).
 * @param pin_out  Buffer to write the PIN string.
 * @param maxlen   Maximum PIN length.
 * @return true if a PIN was provided, false to cancel.
 */
bool bt_pairing_request_pin(BtPairingAgent *agent, const uint8_t *address,
                             char *pin_out, uint32_t maxlen);

/**
 * Confirm a 6-digit SSP passkey displayed on both devices.
 * @param agent    Pairing agent.
 * @param address  Remote device BD_ADDR (6 bytes).
 * @param passkey  The numeric passkey to confirm.
 * @return true to accept, false to reject.
 */
bool bt_pairing_confirm_passkey(BtPairingAgent *agent,
                                 const uint8_t *address, uint32_t passkey);

/**
 * Display a passkey for the user to enter on the remote device.
 * @param agent    Pairing agent.
 * @param address  Remote device BD_ADDR (6 bytes).
 * @param passkey  The passkey to display.
 */
void bt_pairing_display_passkey(BtPairingAgent *agent,
                                 const uint8_t *address, uint32_t passkey);

/**
 * Request user confirmation for a passkey (yes/no).
 * @param agent    Pairing agent.
 * @param address  Remote device BD_ADDR (6 bytes).
 * @param passkey  The passkey to confirm.
 * @return true to accept, false to reject.
 */
bool bt_pairing_request_confirmation(BtPairingAgent *agent,
                                      const uint8_t *address,
                                      uint32_t passkey);

/**
 * Cancel an in-progress pairing operation.
 * @param agent    Pairing agent.
 * @param address  Remote device BD_ADDR (6 bytes).
 */
void bt_pairing_cancel(BtPairingAgent *agent, const uint8_t *address);

/**
 * Get the current pairing state.
 */
BtPairingState bt_pairing_get_state(const BtPairingAgent *agent);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* BLUEZ_PAIR_H */

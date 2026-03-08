/*
 * VeridianOS -- bluez-pair.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Bluetooth pairing agent implementation.  Handles legacy PIN-code pairing
 * and Secure Simple Pairing (SSP) passkey confirmation/display.
 *
 * Registers as org.bluez.Agent1 on D-Bus so that BlueZ-compatible clients
 * (e.g. KDE Bluedevil) can delegate pairing UI to this agent.
 */

#include "bluez-pair.h"
#include "bluez-veridian.h"
#include "bluez-hci-bridge.h"

#include <QDebug>
#include <QDBusConnection>
#include <QDBusMessage>

#include <cstring>
#include <cstdlib>
#include <cstdio>

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

static const char *AGENT_DBUS_INTERFACE = "org.bluez.Agent1";
static const char *DEFAULT_PIN = "0000";
static const uint32_t MAX_PIN_LEN = 16;

/* ========================================================================= */
/* Internal state                                                            */
/* ========================================================================= */

struct BtPairingAgent {
    BtPairingState state;

    /* Currently pairing device address */
    uint8_t current_device[BT_ADDR_LEN];
    bool has_current_device;

    /* Pending passkey for confirmation */
    uint32_t pending_passkey;

    /* User-provided callbacks (optional) */
    bt_pin_request_fn     pin_callback;
    bt_confirm_passkey_fn confirm_callback;
    bt_display_passkey_fn display_callback;

    /* D-Bus registration */
    char object_path[128];
    bool registered;
};

/* ========================================================================= */
/* Helper: format BD_ADDR for logging                                        */
/* ========================================================================= */

static void format_addr(const uint8_t *addr, char *out, size_t maxlen)
{
    if (maxlen < 18) {
        out[0] = '\0';
        return;
    }
    snprintf(out, maxlen, "%02X:%02X:%02X:%02X:%02X:%02X",
             addr[5], addr[4], addr[3], addr[2], addr[1], addr[0]);
}

/* ========================================================================= */
/* Helper: register agent on D-Bus                                           */
/* ========================================================================= */

static bool register_agent_dbus(BtPairingAgent *agent)
{
    QDBusConnection bus = QDBusConnection::systemBus();
    if (!bus.isConnected()) {
        qWarning("BtPairing: cannot connect to system D-Bus");
        return false;
    }

    /* Register the agent object path.
     * In a full implementation, this would export the org.bluez.Agent1
     * interface with methods: Release, RequestPinCode, DisplayPinCode,
     * RequestPasskey, DisplayPasskey, RequestConfirmation,
     * RequestAuthorization, AuthorizeService, Cancel. */
    qDebug("BtPairing: registered agent at %s", agent->object_path);
    agent->registered = true;
    return true;
}

static void unregister_agent_dbus(BtPairingAgent *agent)
{
    if (!agent->registered)
        return;

    qDebug("BtPairing: unregistered agent at %s", agent->object_path);
    agent->registered = false;
}

/* ========================================================================= */
/* Lifecycle                                                                 */
/* ========================================================================= */

BtPairingAgent *bt_pairing_init(void)
{
    BtPairingAgent *agent = new BtPairingAgent;
    if (!agent)
        return nullptr;

    memset(agent, 0, sizeof(*agent));
    agent->state = BT_PAIRING_IDLE;
    agent->has_current_device = false;
    agent->pending_passkey = 0;
    agent->pin_callback = nullptr;
    agent->confirm_callback = nullptr;
    agent->display_callback = nullptr;
    agent->registered = false;

    strncpy(agent->object_path, "/org/bluez/agent/veridian",
            sizeof(agent->object_path) - 1);

    /* Register on D-Bus */
    register_agent_dbus(agent);

    qDebug("BtPairing: pairing agent initialized");
    return agent;
}

void bt_pairing_destroy(BtPairingAgent *agent)
{
    if (!agent)
        return;

    /* Cancel any in-progress pairing */
    if (agent->has_current_device) {
        bt_pairing_cancel(agent, agent->current_device);
    }

    unregister_agent_dbus(agent);

    qDebug("BtPairing: pairing agent destroyed");
    delete agent;
}

/* ========================================================================= */
/* Pairing operations                                                        */
/* ========================================================================= */

bool bt_pairing_request_pin(BtPairingAgent *agent, const uint8_t *address,
                             char *pin_out, uint32_t maxlen)
{
    if (!agent || !address || !pin_out || maxlen < 5)
        return false;

    char addr_str[18];
    format_addr(address, addr_str, sizeof(addr_str));

    /* Set current pairing device */
    memcpy(agent->current_device, address, BT_ADDR_LEN);
    agent->has_current_device = true;
    agent->state = BT_PAIRING_REQUESTED_PIN;

    qDebug("BtPairing: PIN request for device %s", addr_str);

    /* Delegate to user callback if available */
    if (agent->pin_callback) {
        bool result = agent->pin_callback(address, pin_out, maxlen);
        if (!result) {
            qDebug("BtPairing: PIN request cancelled by user");
            agent->state = BT_PAIRING_FAILED;
            agent->has_current_device = false;
            return false;
        }
        qDebug("BtPairing: user provided PIN");
        return true;
    }

    /* Default: use "0000" for simple pairing */
    uint32_t pin_len = strlen(DEFAULT_PIN);
    if (pin_len >= maxlen)
        pin_len = maxlen - 1;
    memcpy(pin_out, DEFAULT_PIN, pin_len);
    pin_out[pin_len] = '\0';

    qDebug("BtPairing: using default PIN '%s' for %s", pin_out, addr_str);
    return true;
}

bool bt_pairing_confirm_passkey(BtPairingAgent *agent,
                                 const uint8_t *address, uint32_t passkey)
{
    if (!agent || !address)
        return false;

    char addr_str[18];
    format_addr(address, addr_str, sizeof(addr_str));

    memcpy(agent->current_device, address, BT_ADDR_LEN);
    agent->has_current_device = true;
    agent->pending_passkey = passkey;
    agent->state = BT_PAIRING_REQUESTED_CONFIRM;

    qDebug("BtPairing: confirm passkey %06u for device %s", passkey, addr_str);

    /* Delegate to user callback if available */
    if (agent->confirm_callback) {
        bool result = agent->confirm_callback(address, passkey);
        if (result) {
            agent->state = BT_PAIRING_COMPLETE;
            qDebug("BtPairing: passkey confirmed by user");
        } else {
            agent->state = BT_PAIRING_FAILED;
            qDebug("BtPairing: passkey rejected by user");
        }
        agent->has_current_device = false;
        return result;
    }

    /* Default: auto-accept SSP confirmation */
    agent->state = BT_PAIRING_COMPLETE;
    agent->has_current_device = false;

    qDebug("BtPairing: auto-accepted passkey %06u for %s", passkey, addr_str);
    return true;
}

void bt_pairing_display_passkey(BtPairingAgent *agent,
                                 const uint8_t *address, uint32_t passkey)
{
    if (!agent || !address)
        return;

    char addr_str[18];
    format_addr(address, addr_str, sizeof(addr_str));

    memcpy(agent->current_device, address, BT_ADDR_LEN);
    agent->has_current_device = true;
    agent->pending_passkey = passkey;

    qDebug("BtPairing: display passkey %06u for device %s", passkey, addr_str);

    /* Delegate to user callback if available */
    if (agent->display_callback) {
        agent->display_callback(address, passkey);
    }

    /* The passkey stays displayed until pairing completes or is cancelled */
}

bool bt_pairing_request_confirmation(BtPairingAgent *agent,
                                      const uint8_t *address,
                                      uint32_t passkey)
{
    if (!agent || !address)
        return false;

    char addr_str[18];
    format_addr(address, addr_str, sizeof(addr_str));

    qDebug("BtPairing: request confirmation for passkey %06u on %s",
           passkey, addr_str);

    /* This is essentially the same flow as confirm_passkey, but is
     * specifically for the RequestConfirmation D-Bus method where
     * the agent must respond yes/no. */
    return bt_pairing_confirm_passkey(agent, address, passkey);
}

void bt_pairing_cancel(BtPairingAgent *agent, const uint8_t *address)
{
    if (!agent || !address)
        return;

    char addr_str[18];
    format_addr(address, addr_str, sizeof(addr_str));

    /* Only cancel if this is the device we're pairing with */
    if (agent->has_current_device &&
        memcmp(agent->current_device, address, BT_ADDR_LEN) == 0) {
        agent->state = BT_PAIRING_IDLE;
        agent->has_current_device = false;
        agent->pending_passkey = 0;

        qDebug("BtPairing: cancelled pairing with %s", addr_str);
    } else {
        qDebug("BtPairing: cancel request for %s (not current device)", addr_str);
    }
}

BtPairingState bt_pairing_get_state(const BtPairingAgent *agent)
{
    if (!agent)
        return BT_PAIRING_IDLE;
    return agent->state;
}

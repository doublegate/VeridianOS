/*
 * VeridianOS -- bluez-veridian.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * BlueZ D-Bus API shim implementation for VeridianOS.
 *
 * Provides the core BlueZ daemon logic: adapter management, device
 * discovery, pairing, connection lifecycle, and D-Bus service
 * registration.  Communicates with the kernel HCI driver via the
 * HCI bridge (bluez-hci-bridge.cpp) and delegates pairing UI to
 * the pairing agent (bluez-pair.cpp).
 */

#include "bluez-veridian.h"
#include "bluez-hci-bridge.h"
#include "bluez-pair.h"

#include <QDebug>
#include <QDir>
#include <QFile>
#include <QTextStream>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QTimer>

#include <cstring>
#include <cstdlib>
#include <cstdio>

#include <unistd.h>

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

static const char *BLUEZ_DBUS_SERVICE     = "org.bluez";
static const char *BLUEZ_DBUS_PATH        = "/org/bluez";
static const char *BLUEZ_ADAPTER_PATH     = "/org/bluez/hci0";
static const char *BLUEZ_ADAPTER_IFACE    = "org.bluez.Adapter1";
static const char *BLUEZ_DEVICE_IFACE     = "org.bluez.Device1";
static const char *BLUEZ_AGENT_MGR_IFACE  = "org.bluez.AgentManager1";
static const char *BLUEZ_GATT_MGR_IFACE   = "org.bluez.GattManager1";
static const char *BLUEZ_OBJMGR_IFACE     = "org.freedesktop.DBus.ObjectManager";

static const char *HCI_DEVICE_PATH        = "/dev/bluetooth/hci0";
static const char *BT_KEYS_DIR            = "/etc/veridian/bluetooth";

/* Discovery scan interval (milliseconds) */
static const int DISCOVERY_POLL_MS = 2000;

/* Event polling interval (milliseconds) */
static const int EVENT_POLL_MS = 100;

/* ========================================================================= */
/* Global daemon state                                                       */
/* ========================================================================= */

static struct {
    /* HCI bridge to kernel */
    HciBridge *hci;

    /* Pairing agent */
    BtPairingAgent *pairing_agent;

    /* Local adapter */
    BtAdapter adapter;

    /* Device registry */
    BtDevice devices[BT_MAX_DEVICES];
    uint32_t device_count;

    /* Registered agent */
    BtAgent agent;

    /* Agent callbacks */
    bt_pin_request_fn     pin_cb;
    bt_confirm_passkey_fn confirm_cb;
    bt_display_passkey_fn display_cb;

    /* Discovery timer */
    QTimer *discovery_timer;

    /* Event polling timer */
    QTimer *event_timer;

    /* Initialization flag */
    bool initialized;
} g_bt;

/* ========================================================================= */
/* Helper: format BD_ADDR as string                                          */
/* ========================================================================= */

static void addr_to_str(const uint8_t *addr, char *out, size_t maxlen)
{
    if (maxlen < 18) {
        out[0] = '\0';
        return;
    }
    snprintf(out, maxlen, "%02X:%02X:%02X:%02X:%02X:%02X",
             addr[5], addr[4], addr[3], addr[2], addr[1], addr[0]);
}

/* ========================================================================= */
/* Helper: D-Bus device object path from BD_ADDR                             */
/* ========================================================================= */

static void addr_to_dbus_path(const uint8_t *addr, char *out, size_t maxlen)
{
    snprintf(out, maxlen,
             "/org/bluez/hci0/dev_%02X_%02X_%02X_%02X_%02X_%02X",
             addr[5], addr[4], addr[3], addr[2], addr[1], addr[0]);
}

/* ========================================================================= */
/* Helper: find device by address                                            */
/* ========================================================================= */

static BtDevice *find_device(const uint8_t *address)
{
    for (uint32_t i = 0; i < g_bt.device_count; ++i) {
        if (memcmp(g_bt.devices[i].address, address, BT_ADDR_LEN) == 0)
            return &g_bt.devices[i];
    }
    return nullptr;
}

static int find_device_index(const uint8_t *address)
{
    for (uint32_t i = 0; i < g_bt.device_count; ++i) {
        if (memcmp(g_bt.devices[i].address, address, BT_ADDR_LEN) == 0)
            return (int)i;
    }
    return -1;
}

/* ========================================================================= */
/* Helper: add or update a discovered device                                 */
/* ========================================================================= */

static BtDevice *add_or_update_device(const uint8_t *address,
                                        uint32_t device_class,
                                        int16_t rssi)
{
    BtDevice *dev = find_device(address);
    if (dev) {
        /* Update existing entry */
        dev->rssi = rssi;
        if (device_class != 0)
            dev->device_class = device_class;
        return dev;
    }

    if (g_bt.device_count >= BT_MAX_DEVICES) {
        qWarning("BlueZ: device registry full (%u)", BT_MAX_DEVICES);
        return nullptr;
    }

    dev = &g_bt.devices[g_bt.device_count];
    memset(dev, 0, sizeof(*dev));
    memcpy(dev->address, address, BT_ADDR_LEN);
    dev->rssi = rssi;
    dev->device_class = device_class;
    dev->device_type = BT_DEVICE_TYPE_BR_EDR;
    dev->pair_state = BT_PAIR_NONE;
    dev->transport_state = BT_TRANSPORT_IDLE;
    strncpy(dev->adapter_path, BLUEZ_ADAPTER_PATH,
            sizeof(dev->adapter_path) - 1);

    g_bt.device_count++;

    char addr_str[18];
    addr_to_str(address, addr_str, sizeof(addr_str));
    qDebug("BlueZ: discovered device %s class=0x%06X rssi=%d",
           addr_str, device_class, rssi);

    return dev;
}

/* ========================================================================= */
/* D-Bus signal emission                                                     */
/* ========================================================================= */

static void emit_interfaces_added(const uint8_t *address)
{
    char path[128];
    addr_to_dbus_path(address, path, sizeof(path));

    QDBusConnection bus = QDBusConnection::systemBus();
    QDBusMessage signal = QDBusMessage::createSignal(
        QString::fromLatin1(BLUEZ_DBUS_PATH),
        QString::fromLatin1(BLUEZ_OBJMGR_IFACE),
        QStringLiteral("InterfacesAdded"));
    signal << QString::fromLatin1(path);
    bus.send(signal);
}

static void emit_interfaces_removed(const uint8_t *address)
{
    char path[128];
    addr_to_dbus_path(address, path, sizeof(path));

    QDBusConnection bus = QDBusConnection::systemBus();
    QDBusMessage signal = QDBusMessage::createSignal(
        QString::fromLatin1(BLUEZ_DBUS_PATH),
        QString::fromLatin1(BLUEZ_OBJMGR_IFACE),
        QStringLiteral("InterfacesRemoved"));
    signal << QString::fromLatin1(path);
    bus.send(signal);
}

static void emit_property_changed(const char *iface_path,
                                    const char *iface_name,
                                    const char *property,
                                    bool value)
{
    QDBusConnection bus = QDBusConnection::systemBus();
    QDBusMessage signal = QDBusMessage::createSignal(
        QString::fromLatin1(iface_path),
        QStringLiteral("org.freedesktop.DBus.Properties"),
        QStringLiteral("PropertiesChanged"));
    signal << QString::fromLatin1(iface_name)
           << QStringLiteral("%1=%2").arg(property, value ? "true" : "false");
    bus.send(signal);
}

/* ========================================================================= */
/* Bonding key storage                                                       */
/* ========================================================================= */

static void ensure_keys_dir(void)
{
    QDir dir(QString::fromLatin1(BT_KEYS_DIR));
    if (!dir.exists())
        dir.mkpath(QStringLiteral("."));
}

static bool save_bonding_key(const uint8_t *address, const uint8_t *key,
                               uint32_t key_len)
{
    ensure_keys_dir();

    char addr_str[18];
    addr_to_str(address, addr_str, sizeof(addr_str));

    QString path = QStringLiteral("%1/%2.key")
                       .arg(BT_KEYS_DIR, addr_str);
    QFile file(path);
    if (!file.open(QIODevice::WriteOnly)) {
        qWarning("BlueZ: cannot save bonding key for %s", addr_str);
        return false;
    }

    file.write(reinterpret_cast<const char *>(key), key_len);
    file.close();

    qDebug("BlueZ: saved bonding key for %s", addr_str);
    return true;
}

static bool load_bonding_key(const uint8_t *address, uint8_t *key_out,
                               uint32_t max_key_len)
{
    char addr_str[18];
    addr_to_str(address, addr_str, sizeof(addr_str));

    QString path = QStringLiteral("%1/%2.key")
                       .arg(BT_KEYS_DIR, addr_str);
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly))
        return false;

    QByteArray data = file.readAll();
    uint32_t copy_len = (uint32_t)data.size();
    if (copy_len > max_key_len)
        copy_len = max_key_len;

    memcpy(key_out, data.constData(), copy_len);
    return true;
}

static bool delete_bonding_key(const uint8_t *address)
{
    char addr_str[18];
    addr_to_str(address, addr_str, sizeof(addr_str));

    QString path = QStringLiteral("%1/%2.key")
                       .arg(BT_KEYS_DIR, addr_str);
    return QFile::remove(path);
}

/* ========================================================================= */
/* Load paired devices from stored keys                                      */
/* ========================================================================= */

static void load_paired_devices(void)
{
    QDir dir(QString::fromLatin1(BT_KEYS_DIR));
    if (!dir.exists())
        return;

    QStringList entries = dir.entryList(
        QStringList{QStringLiteral("*.key")}, QDir::Files);

    for (const QString &entry : entries) {
        /* Parse address from filename: "XX:XX:XX:XX:XX:XX.key" */
        QString addrStr = entry.left(17);  /* "XX:XX:XX:XX:XX:XX" */
        QStringList parts = addrStr.split(':');
        if (parts.size() != 6)
            continue;

        uint8_t addr[BT_ADDR_LEN];
        for (int i = 0; i < 6; ++i) {
            bool ok;
            addr[5 - i] = (uint8_t)parts[i].toUInt(&ok, 16);
            if (!ok) break;
        }

        BtDevice *dev = add_or_update_device(addr, 0, 0);
        if (dev) {
            dev->paired = true;
            dev->pair_state = BT_PAIR_BONDED;
            qDebug("BlueZ: loaded bonded device %s", qPrintable(addrStr));
        }
    }
}

/* ========================================================================= */
/* HCI event processing                                                      */
/* ========================================================================= */

static void process_inquiry_result(const uint8_t *params, uint8_t param_len)
{
    if (param_len < 1)
        return;

    uint8_t num_responses = params[0];
    uint32_t offset = 1;

    /* Each inquiry result is 14 bytes:
     * BD_ADDR(6) + page_scan_rep_mode(1) + reserved(2) +
     * class_of_device(3) + clock_offset(2) */
    for (uint8_t i = 0; i < num_responses; ++i) {
        if (offset + 14 > param_len)
            break;

        uint8_t addr[BT_ADDR_LEN];
        memcpy(addr, &params[offset], 6);
        offset += 6;

        /* Skip page_scan_rep_mode and reserved */
        offset += 3;

        /* Class of Device (3 bytes, little-endian) */
        uint32_t cod = (uint32_t)params[offset]
                     | ((uint32_t)params[offset + 1] << 8)
                     | ((uint32_t)params[offset + 2] << 16);
        offset += 3;

        /* Clock offset (2 bytes) -- skip */
        offset += 2;

        BtDevice *dev = add_or_update_device(addr, cod, -70);
        if (dev) {
            emit_interfaces_added(addr);
        }
    }
}

static void process_connection_complete(const uint8_t *params,
                                          uint8_t param_len)
{
    if (param_len < 11)
        return;

    uint8_t status = params[0];
    /* uint16_t handle = params[1] | (params[2] << 8); */
    uint8_t addr[BT_ADDR_LEN];
    memcpy(addr, &params[3], 6);

    char addr_str[18];
    addr_to_str(addr, addr_str, sizeof(addr_str));

    BtDevice *dev = find_device(addr);
    if (!dev)
        return;

    if (status == 0) {
        dev->connected = true;
        dev->transport_state = BT_TRANSPORT_ACTIVE;
        qDebug("BlueZ: device %s connected", addr_str);
    } else {
        dev->connected = false;
        dev->transport_state = BT_TRANSPORT_IDLE;
        qWarning("BlueZ: connection to %s failed (status=0x%02X)",
                 addr_str, status);
    }
}

static void process_disconnection_complete(const uint8_t *params,
                                             uint8_t param_len)
{
    if (param_len < 4)
        return;

    /* uint8_t status = params[0]; */
    /* uint16_t handle = params[1] | (params[2] << 8); */
    /* uint8_t reason = params[3]; */

    /* We need to find the device by connection handle, but since we
     * don't track handles in the shim, mark all pending disconnects.
     * In a full implementation, maintain a handle -> address map. */
    for (uint32_t i = 0; i < g_bt.device_count; ++i) {
        if (g_bt.devices[i].transport_state == BT_TRANSPORT_PENDING) {
            g_bt.devices[i].connected = false;
            g_bt.devices[i].transport_state = BT_TRANSPORT_IDLE;

            char addr_str[18];
            addr_to_str(g_bt.devices[i].address, addr_str, sizeof(addr_str));
            qDebug("BlueZ: device %s disconnected", addr_str);
        }
    }
}

static void process_pin_code_request(const uint8_t *params,
                                       uint8_t param_len)
{
    if (param_len < 6)
        return;

    uint8_t addr[BT_ADDR_LEN];
    memcpy(addr, params, 6);

    char addr_str[18];
    addr_to_str(addr, addr_str, sizeof(addr_str));
    qDebug("BlueZ: PIN code request from %s", addr_str);

    /* Delegate to pairing agent */
    char pin[MAX_PIN_LEN + 1];
    memset(pin, 0, sizeof(pin));

    bool have_pin = false;
    if (g_bt.pairing_agent) {
        have_pin = bt_pairing_request_pin(g_bt.pairing_agent, addr,
                                            pin, MAX_PIN_LEN);
    } else if (g_bt.pin_cb) {
        have_pin = g_bt.pin_cb(addr, pin, MAX_PIN_LEN);
    }

    if (have_pin) {
        /* Send PIN Code Request Reply */
        uint8_t reply_params[23];
        memset(reply_params, 0, sizeof(reply_params));
        memcpy(reply_params, addr, 6);
        uint8_t pin_len = (uint8_t)strlen(pin);
        reply_params[6] = pin_len;
        memcpy(&reply_params[7], pin, pin_len);

        hci_bridge_send_command(g_bt.hci, HCI_OP_PIN_CODE_REQ_REPLY,
                                 reply_params, 23);
    }
}

static void process_link_key_request(const uint8_t *params,
                                       uint8_t param_len)
{
    if (param_len < 6)
        return;

    uint8_t addr[BT_ADDR_LEN];
    memcpy(addr, params, 6);

    char addr_str[18];
    addr_to_str(addr, addr_str, sizeof(addr_str));

    /* Try to load stored link key */
    uint8_t key[16];
    if (load_bonding_key(addr, key, 16)) {
        qDebug("BlueZ: sending stored link key for %s", addr_str);

        uint8_t reply_params[22];
        memcpy(reply_params, addr, 6);
        memcpy(&reply_params[6], key, 16);

        hci_bridge_send_command(g_bt.hci, HCI_OP_LINK_KEY_REQ_REPLY,
                                 reply_params, 22);
    } else {
        qDebug("BlueZ: no stored link key for %s", addr_str);
        /* In a full implementation, send Link Key Request Negative Reply */
    }
}

static void process_link_key_notification(const uint8_t *params,
                                            uint8_t param_len)
{
    if (param_len < 23)
        return;

    uint8_t addr[BT_ADDR_LEN];
    memcpy(addr, params, 6);
    const uint8_t *key = &params[6];
    /* uint8_t key_type = params[22]; */

    char addr_str[18];
    addr_to_str(addr, addr_str, sizeof(addr_str));

    /* Store the link key */
    save_bonding_key(addr, key, 16);

    /* Mark device as bonded */
    BtDevice *dev = find_device(addr);
    if (dev) {
        dev->paired = true;
        dev->pair_state = BT_PAIR_BONDED;
    }

    qDebug("BlueZ: link key notification for %s (stored)", addr_str);
}

static void poll_hci_events(void)
{
    if (!g_bt.hci)
        return;

    uint8_t buf[259];
    int len = hci_bridge_recv_event(g_bt.hci, buf, sizeof(buf));
    if (len <= 0)
        return;

    uint8_t event_code = buf[0];
    uint8_t param_len = (len >= 2) ? buf[1] : 0;
    const uint8_t *params = (len > 2) ? &buf[2] : nullptr;

    switch (event_code) {
    case HCI_EVT_INQUIRY_RESULT:
        process_inquiry_result(params, param_len);
        break;
    case HCI_EVT_INQUIRY_COMPLETE:
        qDebug("BlueZ: inquiry complete");
        g_bt.adapter.discovering = false;
        g_bt.adapter.state = BT_ADAPTER_ON;
        emit_property_changed(BLUEZ_ADAPTER_PATH, BLUEZ_ADAPTER_IFACE,
                               "Discovering", false);
        break;
    case HCI_EVT_CONNECTION_COMPLETE:
        process_connection_complete(params, param_len);
        break;
    case HCI_EVT_DISCONNECTION_COMPLETE:
        process_disconnection_complete(params, param_len);
        break;
    case HCI_EVT_PIN_CODE_REQUEST:
        process_pin_code_request(params, param_len);
        break;
    case HCI_EVT_LINK_KEY_REQUEST:
        process_link_key_request(params, param_len);
        break;
    case HCI_EVT_LINK_KEY_NOTIFICATION:
        process_link_key_notification(params, param_len);
        break;
    case HCI_EVT_COMMAND_COMPLETE:
    case HCI_EVT_COMMAND_STATUS:
        /* Handled internally by HCI bridge convenience wrappers */
        break;
    default:
        qDebug("BlueZ: unhandled HCI event 0x%02X", event_code);
        break;
    }
}

/* ========================================================================= */
/* D-Bus service registration                                                */
/* ========================================================================= */

static bool register_dbus_service(void)
{
    QDBusConnection bus = QDBusConnection::systemBus();
    if (!bus.isConnected()) {
        qWarning("BlueZ: cannot connect to system D-Bus");
        return false;
    }

    if (!bus.registerService(QString::fromLatin1(BLUEZ_DBUS_SERVICE))) {
        qWarning("BlueZ: cannot register D-Bus service %s", BLUEZ_DBUS_SERVICE);
        return false;
    }

    qDebug("BlueZ: registered D-Bus service %s", BLUEZ_DBUS_SERVICE);
    return true;
}

/* ========================================================================= */
/* Daemon lifecycle                                                          */
/* ========================================================================= */

bool bt_init(void)
{
    if (g_bt.initialized) {
        qWarning("BlueZ: already initialized");
        return false;
    }

    memset(&g_bt, 0, sizeof(g_bt));

    qDebug("BlueZ: initializing BlueZ shim daemon");

    /* Open HCI device via bridge */
    g_bt.hci = hci_bridge_new();
    if (!g_bt.hci) {
        qWarning("BlueZ: cannot allocate HCI bridge");
        return false;
    }

    if (!hci_bridge_open(g_bt.hci, HCI_DEVICE_PATH)) {
        qWarning("BlueZ: cannot open HCI device %s", HCI_DEVICE_PATH);
        hci_bridge_destroy(g_bt.hci);
        g_bt.hci = nullptr;
        return false;
    }

    /* Read local adapter info */
    hci_bridge_get_local_address(g_bt.hci, g_bt.adapter.address);
    hci_bridge_get_local_name(g_bt.hci, g_bt.adapter.name,
                               sizeof(g_bt.adapter.name));

    if (g_bt.adapter.name[0] == '\0')
        strncpy(g_bt.adapter.name, "VeridianOS", sizeof(g_bt.adapter.name) - 1);

    strncpy(g_bt.adapter.alias, g_bt.adapter.name,
            sizeof(g_bt.adapter.alias) - 1);

    g_bt.adapter.powered = true;
    g_bt.adapter.discovering = false;
    g_bt.adapter.pairable = true;
    g_bt.adapter.pairable_timeout = 0;
    g_bt.adapter.discoverable_timeout = 180;
    g_bt.adapter.device_class = 0x001F00;  /* Computer / uncategorized */
    g_bt.adapter.state = BT_ADAPTER_ON;

    /* Initialize pairing agent */
    g_bt.pairing_agent = bt_pairing_init();

    /* Load previously bonded devices */
    load_paired_devices();

    /* Register D-Bus service */
    register_dbus_service();

    /* Start event polling timer */
    g_bt.event_timer = new QTimer();
    QObject::connect(g_bt.event_timer, &QTimer::timeout,
                     []() { poll_hci_events(); });
    g_bt.event_timer->start(EVENT_POLL_MS);

    g_bt.initialized = true;

    char addr_str[18];
    addr_to_str(g_bt.adapter.address, addr_str, sizeof(addr_str));
    qDebug("BlueZ: initialized -- adapter %s name='%s' %u paired devices",
           addr_str, g_bt.adapter.name, g_bt.device_count);

    return true;
}

void bt_cleanup(void)
{
    if (!g_bt.initialized)
        return;

    qDebug("BlueZ: shutting down");

    /* Stop discovery if active */
    if (g_bt.adapter.discovering) {
        bt_adapter_stop_discovery();
    }

    /* Stop timers */
    if (g_bt.event_timer) {
        g_bt.event_timer->stop();
        delete g_bt.event_timer;
        g_bt.event_timer = nullptr;
    }
    if (g_bt.discovery_timer) {
        g_bt.discovery_timer->stop();
        delete g_bt.discovery_timer;
        g_bt.discovery_timer = nullptr;
    }

    /* Destroy pairing agent */
    if (g_bt.pairing_agent) {
        bt_pairing_destroy(g_bt.pairing_agent);
        g_bt.pairing_agent = nullptr;
    }

    /* Close HCI bridge */
    if (g_bt.hci) {
        hci_bridge_destroy(g_bt.hci);
        g_bt.hci = nullptr;
    }

    g_bt.initialized = false;
    qDebug("BlueZ: shutdown complete");
}

void bt_set_agent_callbacks(bt_pin_request_fn pin_cb,
                             bt_confirm_passkey_fn confirm_cb,
                             bt_display_passkey_fn display_cb)
{
    g_bt.pin_cb = pin_cb;
    g_bt.confirm_cb = confirm_cb;
    g_bt.display_cb = display_cb;
}

/* ========================================================================= */
/* Adapter1 interface                                                        */
/* ========================================================================= */

bool bt_adapter_get(BtAdapter *out)
{
    if (!g_bt.initialized || !out)
        return false;

    memcpy(out, &g_bt.adapter, sizeof(BtAdapter));
    return true;
}

bool bt_adapter_set_powered(bool powered)
{
    if (!g_bt.initialized)
        return false;

    if (g_bt.adapter.powered == powered)
        return true;

    if (!powered) {
        /* Power off: stop discovery, disconnect all devices */
        if (g_bt.adapter.discovering)
            bt_adapter_stop_discovery();

        for (uint32_t i = 0; i < g_bt.device_count; ++i) {
            if (g_bt.devices[i].connected)
                bt_device_disconnect(g_bt.devices[i].address);
        }

        g_bt.adapter.state = BT_ADAPTER_OFF;
    } else {
        g_bt.adapter.state = BT_ADAPTER_ON;
    }

    g_bt.adapter.powered = powered;

    emit_property_changed(BLUEZ_ADAPTER_PATH, BLUEZ_ADAPTER_IFACE,
                           "Powered", powered);

    qDebug("BlueZ: adapter powered %s", powered ? "on" : "off");
    return true;
}

bool bt_adapter_start_discovery(void)
{
    if (!g_bt.initialized || !g_bt.adapter.powered)
        return false;

    if (g_bt.adapter.discovering) {
        qDebug("BlueZ: discovery already active");
        return true;
    }

    /* Enable inquiry + page scan on the adapter */
    uint8_t scan_enable = 0x03;  /* inquiry + page scan */
    hci_bridge_send_command(g_bt.hci, HCI_OP_WRITE_SCAN_ENABLE,
                             &scan_enable, 1);

    /* Start HCI inquiry */
    if (!hci_bridge_start_inquiry(g_bt.hci, 10)) {
        qWarning("BlueZ: failed to start inquiry");
        return false;
    }

    g_bt.adapter.discovering = true;
    g_bt.adapter.state = BT_ADAPTER_DISCOVERING;

    emit_property_changed(BLUEZ_ADAPTER_PATH, BLUEZ_ADAPTER_IFACE,
                           "Discovering", true);

    /* Set up periodic re-inquiry for continuous discovery */
    if (!g_bt.discovery_timer) {
        g_bt.discovery_timer = new QTimer();
        QObject::connect(g_bt.discovery_timer, &QTimer::timeout, []() {
            if (g_bt.adapter.discovering && g_bt.adapter.powered) {
                hci_bridge_start_inquiry(g_bt.hci, 10);
            }
        });
    }
    g_bt.discovery_timer->start(12000);  /* re-inquiry every 12s */

    qDebug("BlueZ: discovery started");
    return true;
}

bool bt_adapter_stop_discovery(void)
{
    if (!g_bt.initialized)
        return false;

    if (!g_bt.adapter.discovering)
        return true;

    /* Cancel HCI inquiry */
    hci_bridge_cancel_inquiry(g_bt.hci);

    /* Stop periodic re-inquiry */
    if (g_bt.discovery_timer)
        g_bt.discovery_timer->stop();

    g_bt.adapter.discovering = false;
    g_bt.adapter.state = BT_ADAPTER_ON;

    emit_property_changed(BLUEZ_ADAPTER_PATH, BLUEZ_ADAPTER_IFACE,
                           "Discovering", false);

    qDebug("BlueZ: discovery stopped");
    return true;
}

bool bt_adapter_set_pairable(bool pairable)
{
    if (!g_bt.initialized)
        return false;

    g_bt.adapter.pairable = pairable;

    /* Update scan enable accordingly */
    uint8_t scan_enable = pairable ? 0x03 : 0x01;  /* page only vs page+inquiry */
    hci_bridge_send_command(g_bt.hci, HCI_OP_WRITE_SCAN_ENABLE,
                             &scan_enable, 1);

    emit_property_changed(BLUEZ_ADAPTER_PATH, BLUEZ_ADAPTER_IFACE,
                           "Pairable", pairable);

    qDebug("BlueZ: adapter pairable=%s", pairable ? "true" : "false");
    return true;
}

bool bt_adapter_remove_device(const uint8_t *address)
{
    if (!g_bt.initialized || !address)
        return false;

    int idx = find_device_index(address);
    if (idx < 0)
        return false;

    char addr_str[18];
    addr_to_str(address, addr_str, sizeof(addr_str));

    /* Disconnect if connected */
    if (g_bt.devices[idx].connected)
        bt_device_disconnect(address);

    /* Delete bonding key */
    delete_bonding_key(address);

    /* Emit InterfacesRemoved */
    emit_interfaces_removed(address);

    /* Remove from registry (shift remaining) */
    for (uint32_t j = (uint32_t)idx; j < g_bt.device_count - 1; ++j)
        g_bt.devices[j] = g_bt.devices[j + 1];
    g_bt.device_count--;

    qDebug("BlueZ: removed device %s", addr_str);
    return true;
}

/* ========================================================================= */
/* Device1 interface                                                         */
/* ========================================================================= */

bool bt_device_connect(const uint8_t *address)
{
    if (!g_bt.initialized || !address)
        return false;

    BtDevice *dev = find_device(address);
    if (!dev) {
        qWarning("BlueZ: connect to unknown device");
        return false;
    }

    if (dev->connected) {
        qDebug("BlueZ: device already connected");
        return true;
    }

    char addr_str[18];
    addr_to_str(address, addr_str, sizeof(addr_str));

    /* Build HCI_Create_Connection parameters:
     * BD_ADDR(6) + packet_type(2) + page_scan_rep_mode(1) +
     * reserved(1) + clock_offset(2) + allow_role_switch(1) */
    uint8_t params[13];
    memset(params, 0, sizeof(params));
    memcpy(params, address, 6);
    /* DM1 + DH1 + DM3 + DH3 + DM5 + DH5 */
    params[6] = 0xFF;
    params[7] = 0xFF;
    /* page_scan_rep_mode = R1 */
    params[8] = 0x01;
    /* reserved */
    params[9] = 0x00;
    /* clock_offset = 0 */
    params[10] = 0x00;
    params[11] = 0x00;
    /* allow_role_switch = yes */
    params[12] = 0x01;

    dev->transport_state = BT_TRANSPORT_PENDING;

    bool sent = hci_bridge_send_command(g_bt.hci, HCI_OP_CREATE_CONNECTION,
                                          params, 13);
    if (!sent) {
        dev->transport_state = BT_TRANSPORT_IDLE;
        qWarning("BlueZ: failed to send connect command for %s", addr_str);
        return false;
    }

    qDebug("BlueZ: connecting to %s", addr_str);
    return true;
}

bool bt_device_disconnect(const uint8_t *address)
{
    if (!g_bt.initialized || !address)
        return false;

    BtDevice *dev = find_device(address);
    if (!dev || !dev->connected)
        return false;

    char addr_str[18];
    addr_to_str(address, addr_str, sizeof(addr_str));

    /* We need a connection handle to disconnect.  In a full implementation,
     * maintain a handle -> address map.  For the shim, send disconnect with
     * handle 0x0000 and let the kernel figure it out via address matching. */
    uint8_t params[3];
    params[0] = 0x00;  /* handle low */
    params[1] = 0x00;  /* handle high */
    params[2] = 0x13;  /* reason: Remote User Terminated Connection */

    dev->transport_state = BT_TRANSPORT_PENDING;

    hci_bridge_send_command(g_bt.hci, HCI_OP_DISCONNECT, params, 3);

    qDebug("BlueZ: disconnecting from %s", addr_str);
    return true;
}

bool bt_device_pair(const uint8_t *address)
{
    if (!g_bt.initialized || !address)
        return false;

    BtDevice *dev = find_device(address);
    if (!dev)
        return false;

    if (dev->paired) {
        qDebug("BlueZ: device already paired");
        return true;
    }

    char addr_str[18];
    addr_to_str(address, addr_str, sizeof(addr_str));

    /* Initiate connection first (pairing happens during connection) */
    dev->pair_state = BT_PAIR_PAIRING;

    if (!dev->connected) {
        if (!bt_device_connect(address)) {
            dev->pair_state = BT_PAIR_NONE;
            return false;
        }
    }

    qDebug("BlueZ: pairing with %s", addr_str);
    return true;
}

bool bt_device_cancel_pairing(const uint8_t *address)
{
    if (!g_bt.initialized || !address)
        return false;

    BtDevice *dev = find_device(address);
    if (!dev)
        return false;

    if (dev->pair_state != BT_PAIR_PAIRING)
        return false;

    dev->pair_state = BT_PAIR_NONE;

    /* Cancel in pairing agent */
    if (g_bt.pairing_agent)
        bt_pairing_cancel(g_bt.pairing_agent, address);

    char addr_str[18];
    addr_to_str(address, addr_str, sizeof(addr_str));
    qDebug("BlueZ: cancelled pairing with %s", addr_str);

    return true;
}

bool bt_device_set_trusted(const uint8_t *address, bool trusted)
{
    if (!g_bt.initialized || !address)
        return false;

    BtDevice *dev = find_device(address);
    if (!dev)
        return false;

    dev->trusted = trusted;

    char path[128];
    addr_to_dbus_path(address, path, sizeof(path));
    emit_property_changed(path, BLUEZ_DEVICE_IFACE, "Trusted", trusted);

    return true;
}

bool bt_device_set_blocked(const uint8_t *address, bool blocked)
{
    if (!g_bt.initialized || !address)
        return false;

    BtDevice *dev = find_device(address);
    if (!dev)
        return false;

    dev->blocked = blocked;

    /* If blocking, disconnect first */
    if (blocked && dev->connected)
        bt_device_disconnect(address);

    char path[128];
    addr_to_dbus_path(address, path, sizeof(path));
    emit_property_changed(path, BLUEZ_DEVICE_IFACE, "Blocked", blocked);

    return true;
}

/* ========================================================================= */
/* AgentManager1 interface                                                   */
/* ========================================================================= */

bool bt_agent_register(BtAgentCapability capability, const char *object_path)
{
    if (!g_bt.initialized || !object_path)
        return false;

    g_bt.agent.capability = capability;
    g_bt.agent.registered = true;
    strncpy(g_bt.agent.object_path, object_path,
            sizeof(g_bt.agent.object_path) - 1);

    qDebug("BlueZ: registered agent at %s (capability=%d)",
           object_path, capability);
    return true;
}

bool bt_agent_unregister(const char *object_path)
{
    if (!g_bt.initialized || !object_path)
        return false;

    if (!g_bt.agent.registered ||
        strcmp(g_bt.agent.object_path, object_path) != 0) {
        qWarning("BlueZ: agent %s not registered", object_path);
        return false;
    }

    g_bt.agent.registered = false;
    g_bt.agent.object_path[0] = '\0';

    qDebug("BlueZ: unregistered agent at %s", object_path);
    return true;
}

bool bt_agent_request_default(const char *object_path)
{
    if (!g_bt.initialized || !object_path)
        return false;

    if (!g_bt.agent.registered ||
        strcmp(g_bt.agent.object_path, object_path) != 0)
        return false;

    qDebug("BlueZ: agent %s set as default", object_path);
    return true;
}

/* ========================================================================= */
/* GattManager1 interface (stub)                                             */
/* ========================================================================= */

bool bt_gatt_register_application(const char *object_path)
{
    if (!g_bt.initialized || !object_path)
        return false;

    qDebug("BlueZ: registered GATT application at %s (stub)", object_path);
    return true;
}

/* ========================================================================= */
/* ObjectManager interface                                                   */
/* ========================================================================= */

bool bt_get_managed_objects(BtDeviceList *out)
{
    if (!g_bt.initialized || !out)
        return false;

    memcpy(out->devices, g_bt.devices,
           sizeof(BtDevice) * g_bt.device_count);
    out->count = g_bt.device_count;
    return true;
}

bool bt_get_devices(BtDeviceList *out)
{
    return bt_get_managed_objects(out);
}

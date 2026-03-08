/*
 * VeridianOS -- nm-veridian.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * NetworkManager D-Bus API shim implementation for VeridianOS.
 *
 * Provides the core NM daemon logic: device enumeration, connection
 * profile management, state machine transitions, and D-Bus service
 * registration.  Delegates Wi-Fi to nm-wifi, Ethernet to nm-ethernet,
 * and DNS to nm-dns.
 */

#include "nm-veridian.h"
#include "nm-wifi.h"
#include "nm-ethernet.h"
#include "nm-dns.h"

#include <QDebug>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QTextStream>
#include <QUuid>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QTimer>

#include <cstring>
#include <cstdlib>
#include <cstdio>

#include <unistd.h>
#include <sys/ioctl.h>

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

static const char *NM_DBUS_SERVICE    = "org.freedesktop.NetworkManager";
static const char *NM_DBUS_PATH       = "/org/freedesktop/NetworkManager";
static const char *NM_DBUS_INTERFACE  = "org.freedesktop.NetworkManager";
static const char *NM_CONN_DIR        = "/etc/veridian/connections";
static const char *NM_SYSFS_NET       = "/sys/class/net";

/* Auto-connect scan interval (milliseconds) */
static const int AUTOCONNECT_INTERVAL_MS = 10000;

/* ========================================================================= */
/* Internal state                                                            */
/* ========================================================================= */

struct NMClient {
    /* Devices */
    NMDevice devices[NM_MAX_DEVICES];
    uint32_t device_count;

    /* Active connections */
    NMActiveConnection active[NM_MAX_CONNECTIONS];
    uint32_t active_count;

    /* Saved profiles */
    NMSettingsConnection profiles[NM_MAX_CONNECTIONS];
    uint32_t profile_count;

    /* Wi-Fi backends (one per Wi-Fi device) */
    NMWifiBackend *wifi_backends[NM_MAX_DEVICES];

    /* Global state */
    NMState state;
    bool networking_enabled;
    bool wireless_enabled;

    /* Auto-connect timer */
    QTimer *autoconnect_timer;

    /* Next ID counters for D-Bus object paths */
    uint32_t next_active_id;
};

/* ========================================================================= */
/* Helper: read a sysfs attribute                                            */
/* ========================================================================= */

static QString read_sysfs(const QString &path)
{
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text))
        return QString();
    return QString::fromUtf8(file.readAll()).trimmed();
}

static int read_sysfs_int(const QString &path)
{
    bool ok;
    int val = read_sysfs(path).toInt(&ok);
    return ok ? val : -1;
}

/* ========================================================================= */
/* Helper: detect device type from sysfs                                     */
/* ========================================================================= */

static NMDeviceType detect_device_type(const QString &iface)
{
    if (iface == QStringLiteral("lo"))
        return NM_DEVICE_TYPE_LOOPBACK;

    /* Check /sys/class/net/<iface>/type -- 1=Ethernet, 801=Wi-Fi */
    QString typePath = QStringLiteral("%1/%2/type").arg(NM_SYSFS_NET, iface);
    int arpType = read_sysfs_int(typePath);

    if (arpType == 801)
        return NM_DEVICE_TYPE_WIFI;

    /* Check for wireless directory as secondary heuristic */
    if (QDir(QStringLiteral("%1/%2/wireless").arg(NM_SYSFS_NET, iface)).exists())
        return NM_DEVICE_TYPE_WIFI;

    /* Check for bridge/bond directories */
    if (QDir(QStringLiteral("%1/%2/bridge").arg(NM_SYSFS_NET, iface)).exists())
        return NM_DEVICE_TYPE_BRIDGE;
    if (QDir(QStringLiteral("%1/%2/bonding").arg(NM_SYSFS_NET, iface)).exists())
        return NM_DEVICE_TYPE_BOND;

    /* Default to Ethernet for ARPHRD_ETHER (type 1) */
    if (arpType == 1)
        return NM_DEVICE_TYPE_ETHERNET;

    return NM_DEVICE_TYPE_UNKNOWN;
}

/* ========================================================================= */
/* Helper: detect device state from sysfs                                    */
/* ========================================================================= */

static NMDeviceState detect_device_state(const QString &iface)
{
    QString operPath = QStringLiteral("%1/%2/operstate").arg(NM_SYSFS_NET, iface);
    QString operState = read_sysfs(operPath);

    if (operState == QStringLiteral("up"))
        return NM_DEVICE_STATE_ACTIVATED;
    if (operState == QStringLiteral("dormant"))
        return NM_DEVICE_STATE_DISCONNECTED;
    if (operState == QStringLiteral("down"))
        return NM_DEVICE_STATE_UNAVAILABLE;

    /* Check carrier for Ethernet */
    QString carrierPath = QStringLiteral("%1/%2/carrier").arg(NM_SYSFS_NET, iface);
    int carrier = read_sysfs_int(carrierPath);
    if (carrier == 1)
        return NM_DEVICE_STATE_DISCONNECTED;  /* link up but no IP */

    return NM_DEVICE_STATE_UNAVAILABLE;
}

/* ========================================================================= */
/* Helper: read MAC address from sysfs                                       */
/* ========================================================================= */

static void read_hw_address(const QString &iface, char *buf, size_t len)
{
    QString path = QStringLiteral("%1/%2/address").arg(NM_SYSFS_NET, iface);
    QString mac = read_sysfs(path);
    if (!mac.isEmpty()) {
        QByteArray macBytes = mac.toUtf8();
        size_t copyLen = (size_t)macBytes.size();
        if (copyLen >= len)
            copyLen = len - 1;
        memcpy(buf, macBytes.constData(), copyLen);
        buf[copyLen] = '\0';
    } else {
        buf[0] = '\0';
    }
}

/* ========================================================================= */
/* Helper: read driver name from sysfs                                       */
/* ========================================================================= */

static void read_driver_name(const QString &iface, char *buf, size_t len)
{
    QString symlink = QStringLiteral("%1/%2/device/driver").arg(NM_SYSFS_NET, iface);
    QFileInfo fi(symlink);
    if (fi.isSymLink()) {
        QByteArray name = fi.symLinkTarget().section('/', -1).toUtf8();
        size_t copyLen = (size_t)name.size();
        if (copyLen >= len)
            copyLen = len - 1;
        memcpy(buf, name.constData(), copyLen);
        buf[copyLen] = '\0';
    } else {
        strncpy(buf, "unknown", len - 1);
        buf[len - 1] = '\0';
    }
}

/* ========================================================================= */
/* Helper: read link speed from sysfs                                        */
/* ========================================================================= */

static uint32_t read_link_speed(const QString &iface)
{
    QString path = QStringLiteral("%1/%2/speed").arg(NM_SYSFS_NET, iface);
    int speed = read_sysfs_int(path);
    return (speed > 0) ? (uint32_t)speed : 0;
}

/* ========================================================================= */
/* Device enumeration                                                        */
/* ========================================================================= */

static void enumerate_devices(NMClient *client)
{
    client->device_count = 0;
    memset(client->devices, 0, sizeof(client->devices));

    QDir netDir(QString::fromLatin1(NM_SYSFS_NET));
    if (!netDir.exists()) {
        qWarning("NM/Veridian: %s does not exist, no devices found", NM_SYSFS_NET);
        return;
    }

    QStringList interfaces = netDir.entryList(QDir::Dirs | QDir::NoDotAndDotDot);

    for (const QString &iface : interfaces) {
        if (client->device_count >= NM_MAX_DEVICES)
            break;

        NMDevice *dev = &client->devices[client->device_count];

        QByteArray ifaceBytes = iface.toUtf8();
        strncpy(dev->interface_name, ifaceBytes.constData(),
                NM_MAX_IFACE_NAME - 1);
        dev->interface_name[NM_MAX_IFACE_NAME - 1] = '\0';

        dev->type = detect_device_type(iface);
        dev->state = detect_device_state(iface);
        dev->device_index = client->device_count;
        dev->speed_mbps = read_link_speed(iface);
        dev->managed = (dev->type != NM_DEVICE_TYPE_LOOPBACK);
        dev->autoconnect = dev->managed;
        dev->active_connection = -1;

        read_hw_address(iface, dev->hw_address, NM_MAX_HWADDR_LEN);
        read_driver_name(iface, dev->driver, NM_MAX_DRIVER_NAME);

        /* Create Wi-Fi backend if applicable */
        if (dev->type == NM_DEVICE_TYPE_WIFI) {
            client->wifi_backends[client->device_count] =
                nm_wifi_backend_new(dev->interface_name);
        }

        qDebug("NM/Veridian: found device %s type=%d state=%d mac=%s",
               dev->interface_name, dev->type, dev->state, dev->hw_address);

        client->device_count++;
    }
}

/* ========================================================================= */
/* Connection profile I/O                                                    */
/* ========================================================================= */

static void ensure_conn_dir(void)
{
    QDir dir(QString::fromLatin1(NM_CONN_DIR));
    if (!dir.exists())
        dir.mkpath(QStringLiteral("."));
}

static bool save_profile(const NMSettingsConnection *conn)
{
    ensure_conn_dir();

    QString path = QStringLiteral("%1/%2.conf").arg(NM_CONN_DIR, conn->uuid);
    QFile file(path);
    if (!file.open(QIODevice::WriteOnly | QIODevice::Text)) {
        qWarning("NM/Veridian: cannot write profile %s", qPrintable(path));
        return false;
    }

    QTextStream out(&file);
    out << QStringLiteral("[connection]\n");
    out << QStringLiteral("id=%1\n").arg(conn->id);
    out << QStringLiteral("uuid=%1\n").arg(conn->uuid);
    out << QStringLiteral("type=%1\n").arg(conn->type);
    out << QStringLiteral("autoconnect=%1\n").arg(conn->autoconnect ? "true" : "false");
    out << QStringLiteral("timestamp=%1\n").arg(conn->timestamp);

    out << QStringLiteral("\n[settings]\n");
    for (uint32_t i = 0; i < conn->entry_count; ++i) {
        out << QStringLiteral("%1=%2\n")
                   .arg(conn->entries[i].key, conn->entries[i].value);
    }

    qDebug("NM/Veridian: saved profile %s (%s)", conn->id, conn->uuid);
    return true;
}

static bool load_profile(const QString &path, NMSettingsConnection *conn)
{
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text))
        return false;

    memset(conn, 0, sizeof(*conn));

    QTextStream in(&file);
    bool in_settings = false;

    while (!in.atEnd()) {
        QString line = in.readLine().trimmed();
        if (line.isEmpty() || line.startsWith('#'))
            continue;

        if (line == QStringLiteral("[connection]")) {
            in_settings = false;
            continue;
        }
        if (line == QStringLiteral("[settings]")) {
            in_settings = true;
            continue;
        }

        int eq = line.indexOf('=');
        if (eq < 0)
            continue;

        QString key = line.left(eq).trimmed();
        QString val = line.mid(eq + 1).trimmed();

        if (!in_settings) {
            /* [connection] section */
            if (key == QStringLiteral("id")) {
                QByteArray b = val.toUtf8();
                strncpy(conn->id, b.constData(), NM_MAX_CONN_ID - 1);
            } else if (key == QStringLiteral("uuid")) {
                QByteArray b = val.toUtf8();
                strncpy(conn->uuid, b.constData(), NM_MAX_UUID_LEN - 1);
            } else if (key == QStringLiteral("type")) {
                QByteArray b = val.toUtf8();
                strncpy(conn->type, b.constData(), NM_MAX_CONN_TYPE - 1);
            } else if (key == QStringLiteral("autoconnect")) {
                conn->autoconnect = (val == QStringLiteral("true"));
            } else if (key == QStringLiteral("timestamp")) {
                conn->timestamp = val.toUInt();
            }
        } else {
            /* [settings] section */
            if (conn->entry_count < NM_MAX_SETTINGS_KV) {
                NMSettingEntry *entry = &conn->entries[conn->entry_count];
                QByteArray kb = key.toUtf8();
                QByteArray vb = val.toUtf8();
                strncpy(entry->key, kb.constData(), NM_MAX_SETTINGS_KEY - 1);
                strncpy(entry->value, vb.constData(), NM_MAX_SETTINGS_VAL - 1);
                conn->entry_count++;
            }
        }
    }

    return conn->uuid[0] != '\0';
}

static void load_all_profiles(NMClient *client)
{
    client->profile_count = 0;

    QDir dir(QString::fromLatin1(NM_CONN_DIR));
    if (!dir.exists())
        return;

    QStringList entries = dir.entryList(
        QStringList{QStringLiteral("*.conf")}, QDir::Files);

    for (const QString &entry : entries) {
        if (client->profile_count >= NM_MAX_CONNECTIONS)
            break;

        QString path = QStringLiteral("%1/%2").arg(NM_CONN_DIR, entry);
        if (load_profile(path, &client->profiles[client->profile_count])) {
            qDebug("NM/Veridian: loaded profile '%s' uuid=%s",
                   client->profiles[client->profile_count].id,
                   client->profiles[client->profile_count].uuid);
            client->profile_count++;
        }
    }

    qDebug("NM/Veridian: loaded %u connection profiles", client->profile_count);
}

/* ========================================================================= */
/* D-Bus service registration                                                */
/* ========================================================================= */

static bool register_dbus_service(void)
{
    QDBusConnection bus = QDBusConnection::systemBus();
    if (!bus.isConnected()) {
        qWarning("NM/Veridian: cannot connect to system D-Bus");
        return false;
    }

    if (!bus.registerService(QString::fromLatin1(NM_DBUS_SERVICE))) {
        qWarning("NM/Veridian: cannot register D-Bus service %s", NM_DBUS_SERVICE);
        return false;
    }

    qDebug("NM/Veridian: registered D-Bus service %s", NM_DBUS_SERVICE);
    return true;
}

/* ========================================================================= */
/* D-Bus signal emission                                                     */
/* ========================================================================= */

static void emit_state_changed(NMState new_state)
{
    QDBusConnection bus = QDBusConnection::systemBus();
    QDBusMessage signal = QDBusMessage::createSignal(
        QString::fromLatin1(NM_DBUS_PATH),
        QString::fromLatin1(NM_DBUS_INTERFACE),
        QStringLiteral("StateChanged"));
    signal << static_cast<uint32_t>(new_state);
    bus.send(signal);
}

static void emit_device_state_changed(uint32_t device_index,
                                       NMDeviceState new_state,
                                       NMDeviceState old_state)
{
    QDBusConnection bus = QDBusConnection::systemBus();
    QString path = QStringLiteral("/org/freedesktop/NetworkManager/Devices/%1")
                       .arg(device_index);
    QDBusMessage signal = QDBusMessage::createSignal(
        path,
        QStringLiteral("org.freedesktop.NetworkManager.Device"),
        QStringLiteral("StateChanged"));
    signal << static_cast<uint32_t>(new_state)
           << static_cast<uint32_t>(old_state)
           << static_cast<uint32_t>(0);  /* reason: none */
    bus.send(signal);
}

/* ========================================================================= */
/* State machine helpers                                                     */
/* ========================================================================= */

static void update_device_state(NMClient *client, uint32_t idx,
                                 NMDeviceState new_state)
{
    if (idx >= client->device_count)
        return;

    NMDeviceState old_state = client->devices[idx].state;
    if (old_state == new_state)
        return;

    client->devices[idx].state = new_state;
    emit_device_state_changed(idx, new_state, old_state);

    qDebug("NM/Veridian: device %s state %d -> %d",
           client->devices[idx].interface_name, old_state, new_state);
}

static void update_global_state(NMClient *client)
{
    NMState new_state = NM_STATE_DISCONNECTED;

    if (!client->networking_enabled) {
        new_state = NM_STATE_ASLEEP;
    } else {
        /* Find best connectivity among active connections */
        for (uint32_t i = 0; i < client->active_count; ++i) {
            if (client->active[i].state == NM_ACTIVE_STATE_ACTIVATED) {
                if (client->active[i].is_default) {
                    new_state = NM_STATE_CONNECTED_GLOBAL;
                    break;
                }
                if (new_state < NM_STATE_CONNECTED_LOCAL)
                    new_state = NM_STATE_CONNECTED_LOCAL;
            } else if (client->active[i].state == NM_ACTIVE_STATE_ACTIVATING) {
                if (new_state < NM_STATE_CONNECTING)
                    new_state = NM_STATE_CONNECTING;
            }
        }
    }

    if (client->state != new_state) {
        qDebug("NM/Veridian: global state %d -> %d", client->state, new_state);
        client->state = new_state;
        emit_state_changed(new_state);
    }
}

/* ========================================================================= */
/* Connection activation                                                     */
/* ========================================================================= */

static NMSettingsConnection *find_profile_by_uuid(NMClient *client,
                                                    const char *uuid)
{
    for (uint32_t i = 0; i < client->profile_count; ++i) {
        if (strcmp(client->profiles[i].uuid, uuid) == 0)
            return &client->profiles[i];
    }
    return nullptr;
}

static NMDevice *find_device_by_name(NMClient *client, const char *iface)
{
    for (uint32_t i = 0; i < client->device_count; ++i) {
        if (strcmp(client->devices[i].interface_name, iface) == 0)
            return &client->devices[i];
    }
    return nullptr;
}

static int find_device_index(NMClient *client, const char *iface)
{
    for (uint32_t i = 0; i < client->device_count; ++i) {
        if (strcmp(client->devices[i].interface_name, iface) == 0)
            return (int)i;
    }
    return -1;
}

static NMWifiBackend *find_wifi_backend(NMClient *client, const char *iface)
{
    int idx = find_device_index(client, iface);
    if (idx < 0)
        return nullptr;
    return client->wifi_backends[idx];
}

static const char *get_profile_setting(const NMSettingsConnection *conn,
                                        const char *key)
{
    for (uint32_t i = 0; i < conn->entry_count; ++i) {
        if (strcmp(conn->entries[i].key, key) == 0)
            return conn->entries[i].value;
    }
    return nullptr;
}

static bool activate_ethernet(NMClient *client, NMDevice *dev,
                               const NMSettingsConnection *conn)
{
    int idx = find_device_index(client, dev->interface_name);
    update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_PREPARE);

    /* Check link */
    if (!nm_ethernet_detect_link(dev->interface_name)) {
        qWarning("NM/Veridian: no carrier on %s", dev->interface_name);
        update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_UNAVAILABLE);
        return false;
    }

    update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_CONFIG);

    /* Check for static IP or DHCP */
    const char *method = get_profile_setting(conn, "ipv4.method");
    bool ip_ok = false;

    if (method && strcmp(method, "manual") == 0) {
        const char *addr = get_profile_setting(conn, "ipv4.address");
        const char *mask = get_profile_setting(conn, "ipv4.netmask");
        const char *gw   = get_profile_setting(conn, "ipv4.gateway");
        ip_ok = nm_ethernet_set_ip(dev->interface_name,
                                    addr ? addr : "0.0.0.0",
                                    mask ? mask : "255.255.255.0",
                                    gw);
    } else {
        /* Default: DHCP */
        update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_IP_CONFIG);
        ip_ok = nm_ethernet_trigger_dhcp(dev->interface_name, 30);
    }

    if (!ip_ok) {
        qWarning("NM/Veridian: IP configuration failed on %s",
                 dev->interface_name);
        update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_FAILED);
        return false;
    }

    /* Configure DNS */
    const char *dns1 = get_profile_setting(conn, "ipv4.dns1");
    const char *dns2 = get_profile_setting(conn, "ipv4.dns2");
    const char *search = get_profile_setting(conn, "ipv4.dns-search");

    if (dns1) {
        const char *servers[2] = { dns1, dns2 };
        uint32_t server_count = dns2 ? 2 : 1;
        const char *domains[1] = { search };
        uint32_t domain_count = search ? 1 : 0;
        nm_dns_set_servers(servers, server_count, domains, domain_count);
    }

    update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_IP_CHECK);
    update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_ACTIVATED);

    return true;
}

static bool activate_wifi(NMClient *client, NMDevice *dev,
                           const NMSettingsConnection *conn)
{
    NMWifiBackend *wifi = find_wifi_backend(client, dev->interface_name);
    if (!wifi) {
        qWarning("NM/Veridian: no Wi-Fi backend for %s", dev->interface_name);
        return false;
    }

    int idx = find_device_index(client, dev->interface_name);
    update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_PREPARE);

    const char *ssid = get_profile_setting(conn, "wifi.ssid");
    const char *psk  = get_profile_setting(conn, "wifi.psk");

    if (!ssid) {
        qWarning("NM/Veridian: Wi-Fi profile %s has no SSID", conn->id);
        update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_FAILED);
        return false;
    }

    update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_CONFIG);

    if (!nm_wifi_connect(wifi, ssid, psk)) {
        qWarning("NM/Veridian: Wi-Fi connect to '%s' failed", ssid);
        update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_FAILED);
        return false;
    }

    update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_IP_CONFIG);

    /* DHCP for Wi-Fi */
    if (!nm_ethernet_trigger_dhcp(dev->interface_name, 30)) {
        qWarning("NM/Veridian: DHCP failed on Wi-Fi %s", dev->interface_name);
        update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_FAILED);
        return false;
    }

    /* DNS */
    const char *dns1 = get_profile_setting(conn, "ipv4.dns1");
    if (dns1) {
        const char *servers[1] = { dns1 };
        nm_dns_set_servers(servers, 1, nullptr, 0);
    }

    update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_IP_CHECK);
    update_device_state(client, (uint32_t)idx, NM_DEVICE_STATE_ACTIVATED);

    return true;
}

static bool create_active_connection(NMClient *client, const char *uuid,
                                      const char *iface)
{
    if (client->active_count >= NM_MAX_CONNECTIONS)
        return false;

    NMSettingsConnection *profile = find_profile_by_uuid(client, uuid);
    if (!profile)
        return false;

    NMActiveConnection *ac = &client->active[client->active_count];
    memset(ac, 0, sizeof(*ac));

    strncpy(ac->uuid, profile->uuid, NM_MAX_UUID_LEN - 1);
    strncpy(ac->type, profile->type, NM_MAX_CONN_TYPE - 1);
    strncpy(ac->id, profile->id, NM_MAX_CONN_ID - 1);
    ac->device_index = (uint32_t)find_device_index(client, iface);
    ac->state = NM_ACTIVE_STATE_ACTIVATING;
    ac->is_default = (client->active_count == 0);  /* first = default */
    ac->is_vpn = false;

    client->active_count++;
    return true;
}

/* ========================================================================= */
/* Auto-connect logic                                                        */
/* ========================================================================= */

static void try_autoconnect(NMClient *client)
{
    if (!client->networking_enabled)
        return;

    for (uint32_t p = 0; p < client->profile_count; ++p) {
        NMSettingsConnection *profile = &client->profiles[p];
        if (!profile->autoconnect)
            continue;

        /* Check if already active */
        bool already_active = false;
        for (uint32_t a = 0; a < client->active_count; ++a) {
            if (strcmp(client->active[a].uuid, profile->uuid) == 0) {
                already_active = true;
                break;
            }
        }
        if (already_active)
            continue;

        /* Find a suitable device */
        const char *target_iface = get_profile_setting(profile, "connection.interface-name");

        for (uint32_t d = 0; d < client->device_count; ++d) {
            NMDevice *dev = &client->devices[d];

            if (!dev->managed || dev->active_connection >= 0)
                continue;

            /* Match type */
            bool type_match = false;
            if (strcmp(profile->type, "802-3-ethernet") == 0 &&
                dev->type == NM_DEVICE_TYPE_ETHERNET)
                type_match = true;
            else if (strcmp(profile->type, "802-11-wireless") == 0 &&
                     dev->type == NM_DEVICE_TYPE_WIFI)
                type_match = true;

            if (!type_match)
                continue;

            /* Match interface name if specified */
            if (target_iface && strcmp(target_iface, dev->interface_name) != 0)
                continue;

            qDebug("NM/Veridian: auto-connecting '%s' on %s",
                   profile->id, dev->interface_name);

            nm_client_activate_connection(client, profile->uuid,
                                          dev->interface_name);
            break;
        }
    }
}

/* ========================================================================= */
/* Client lifecycle                                                          */
/* ========================================================================= */

NMClient *nm_client_new(void)
{
    NMClient *client = new NMClient;
    memset(client, 0, sizeof(*client));

    client->state = NM_STATE_DISCONNECTED;
    client->networking_enabled = true;
    client->wireless_enabled = true;
    client->next_active_id = 0;

    qDebug("NM/Veridian: initializing NetworkManager shim");

    /* Register D-Bus service */
    register_dbus_service();

    /* Enumerate devices */
    enumerate_devices(client);

    /* Load saved profiles */
    load_all_profiles(client);

    /* Start Wi-Fi background scanning for disconnected Wi-Fi devices */
    for (uint32_t i = 0; i < client->device_count; ++i) {
        if (client->wifi_backends[i]) {
            nm_wifi_start_background_scan(client->wifi_backends[i], 30);
        }
    }

    /* Set up auto-connect timer */
    client->autoconnect_timer = new QTimer();
    QObject::connect(client->autoconnect_timer, &QTimer::timeout,
                     [client]() { try_autoconnect(client); });
    client->autoconnect_timer->start(AUTOCONNECT_INTERVAL_MS);

    /* Initial auto-connect attempt */
    try_autoconnect(client);

    qDebug("NM/Veridian: initialized -- %u devices, %u profiles",
           client->device_count, client->profile_count);

    return client;
}

void nm_client_destroy(NMClient *client)
{
    if (!client)
        return;

    qDebug("NM/Veridian: shutting down");

    /* Stop auto-connect */
    if (client->autoconnect_timer) {
        client->autoconnect_timer->stop();
        delete client->autoconnect_timer;
    }

    /* Destroy Wi-Fi backends */
    for (uint32_t i = 0; i < client->device_count; ++i) {
        if (client->wifi_backends[i]) {
            nm_wifi_backend_destroy(client->wifi_backends[i]);
            client->wifi_backends[i] = nullptr;
        }
    }

    /* Restore DNS */
    nm_dns_restore_resolv_conf();

    delete client;
}

/* ========================================================================= */
/* Global state                                                              */
/* ========================================================================= */

NMState nm_client_get_state(NMClient *client)
{
    if (!client)
        return NM_STATE_UNKNOWN;
    return client->state;
}

bool nm_client_set_networking_enabled(NMClient *client, bool enabled)
{
    if (!client)
        return false;

    client->networking_enabled = enabled;

    if (!enabled) {
        /* Deactivate all connections */
        for (uint32_t i = 0; i < client->active_count; ++i) {
            nm_client_deactivate_connection(client, client->active[i].uuid);
        }
    }

    update_global_state(client);
    return true;
}

bool nm_client_get_networking_enabled(NMClient *client)
{
    return client ? client->networking_enabled : false;
}

bool nm_client_set_wireless_enabled(NMClient *client, bool enabled)
{
    if (!client)
        return false;

    client->wireless_enabled = enabled;

    if (!enabled) {
        /* Stop Wi-Fi scanning and disconnect Wi-Fi */
        for (uint32_t i = 0; i < client->device_count; ++i) {
            if (client->wifi_backends[i]) {
                nm_wifi_stop_background_scan(client->wifi_backends[i]);
                nm_wifi_disconnect(client->wifi_backends[i]);
                update_device_state(client, i, NM_DEVICE_STATE_UNAVAILABLE);
            }
        }
    } else {
        /* Resume scanning */
        for (uint32_t i = 0; i < client->device_count; ++i) {
            if (client->wifi_backends[i]) {
                update_device_state(client, i, NM_DEVICE_STATE_DISCONNECTED);
                nm_wifi_start_background_scan(client->wifi_backends[i], 30);
            }
        }
    }

    return true;
}

bool nm_client_get_wireless_enabled(NMClient *client)
{
    return client ? client->wireless_enabled : false;
}

/* ========================================================================= */
/* Device queries                                                            */
/* ========================================================================= */

bool nm_client_get_devices(NMClient *client, NMDeviceList *out)
{
    if (!client || !out)
        return false;

    memcpy(out->devices, client->devices,
           sizeof(NMDevice) * client->device_count);
    out->count = client->device_count;
    return true;
}

NMDeviceType nm_device_get_type(NMClient *client, const char *iface)
{
    NMDevice *dev = find_device_by_name(client, iface);
    return dev ? dev->type : NM_DEVICE_TYPE_UNKNOWN;
}

NMDeviceState nm_device_get_state(NMClient *client, const char *iface)
{
    NMDevice *dev = find_device_by_name(client, iface);
    return dev ? dev->state : NM_DEVICE_STATE_UNKNOWN;
}

bool nm_device_get_ip_config(NMClient *client, const char *iface,
                             NMIPConfig *out)
{
    if (!client || !iface || !out)
        return false;

    NMDevice *dev = find_device_by_name(client, iface);
    if (!dev || dev->state != NM_DEVICE_STATE_ACTIVATED)
        return false;

    memset(out, 0, sizeof(*out));

    /* Read IP config from sysfs/netlink
     * On VeridianOS, we read from /proc/net or netlink socket */
    /* TODO: populate from kernel netlink interface */
    return true;
}

/* ========================================================================= */
/* Active connections                                                        */
/* ========================================================================= */

bool nm_client_get_active_connections(NMClient *client,
                                      NMActiveConnectionList *out)
{
    if (!client || !out)
        return false;

    memcpy(out->connections, client->active,
           sizeof(NMActiveConnection) * client->active_count);
    out->count = client->active_count;
    return true;
}

bool nm_client_activate_connection(NMClient *client, const char *uuid,
                                   const char *iface)
{
    if (!client || !iface)
        return false;

    NMDevice *dev = find_device_by_name(client, iface);
    if (!dev) {
        qWarning("NM/Veridian: unknown device %s", iface);
        return false;
    }

    NMSettingsConnection *profile = nullptr;

    if (uuid) {
        profile = find_profile_by_uuid(client, uuid);
        if (!profile) {
            qWarning("NM/Veridian: unknown profile %s", uuid);
            return false;
        }
    } else {
        /* Auto-pick first matching profile */
        for (uint32_t i = 0; i < client->profile_count; ++i) {
            bool match = false;
            if (dev->type == NM_DEVICE_TYPE_ETHERNET &&
                strcmp(client->profiles[i].type, "802-3-ethernet") == 0)
                match = true;
            else if (dev->type == NM_DEVICE_TYPE_WIFI &&
                     strcmp(client->profiles[i].type, "802-11-wireless") == 0)
                match = true;

            if (match) {
                profile = &client->profiles[i];
                break;
            }
        }
        if (!profile) {
            qWarning("NM/Veridian: no matching profile for %s", iface);
            return false;
        }
    }

    /* Create active connection entry */
    create_active_connection(client, profile->uuid, iface);

    bool success = false;
    switch (dev->type) {
    case NM_DEVICE_TYPE_ETHERNET:
        success = activate_ethernet(client, dev, profile);
        break;
    case NM_DEVICE_TYPE_WIFI:
        success = activate_wifi(client, dev, profile);
        break;
    default:
        qWarning("NM/Veridian: unsupported device type %d", dev->type);
        break;
    }

    if (success) {
        dev->active_connection = (int32_t)(client->active_count - 1);
        client->active[client->active_count - 1].state =
            NM_ACTIVE_STATE_ACTIVATED;
        update_global_state(client);
    } else {
        /* Remove failed active connection */
        if (client->active_count > 0)
            client->active_count--;
    }

    return success;
}

bool nm_client_deactivate_connection(NMClient *client, const char *uuid)
{
    if (!client || !uuid)
        return false;

    for (uint32_t i = 0; i < client->active_count; ++i) {
        if (strcmp(client->active[i].uuid, uuid) != 0)
            continue;

        NMActiveConnection *ac = &client->active[i];
        ac->state = NM_ACTIVE_STATE_DEACTIVATING;

        /* Find device and bring it down */
        int dev_idx = (int)ac->device_index;
        if (dev_idx >= 0 && (uint32_t)dev_idx < client->device_count) {
            NMDevice *dev = &client->devices[dev_idx];

            if (dev->type == NM_DEVICE_TYPE_WIFI) {
                NMWifiBackend *wifi = client->wifi_backends[dev_idx];
                if (wifi)
                    nm_wifi_disconnect(wifi);
            }

            update_device_state(client, (uint32_t)dev_idx,
                                 NM_DEVICE_STATE_DEACTIVATING);
            dev->active_connection = -1;
            update_device_state(client, (uint32_t)dev_idx,
                                 NM_DEVICE_STATE_DISCONNECTED);
        }

        /* Flush DNS on disconnect */
        nm_dns_flush_cache();

        /* Remove from active list (shift remaining) */
        for (uint32_t j = i; j < client->active_count - 1; ++j)
            client->active[j] = client->active[j + 1];
        client->active_count--;

        update_global_state(client);
        return true;
    }

    qWarning("NM/Veridian: no active connection with uuid %s", uuid);
    return false;
}

/* ========================================================================= */
/* Settings (saved profiles)                                                 */
/* ========================================================================= */

bool nm_settings_list_connections(NMClient *client,
                                  NMSettingsConnectionList *out)
{
    if (!client || !out)
        return false;

    memcpy(out->connections, client->profiles,
           sizeof(NMSettingsConnection) * client->profile_count);
    out->count = client->profile_count;
    return true;
}

static char s_uuid_buf[NM_MAX_UUID_LEN];

const char *nm_settings_add_connection(NMClient *client,
                                       const NMConnectionSettings *settings)
{
    if (!client || !settings)
        return nullptr;

    if (client->profile_count >= NM_MAX_CONNECTIONS) {
        qWarning("NM/Veridian: profile limit reached");
        return nullptr;
    }

    NMSettingsConnection *conn = &client->profiles[client->profile_count];
    memset(conn, 0, sizeof(*conn));

    /* Generate UUID */
    QByteArray uuid = QUuid::createUuid().toString(QUuid::WithoutBraces).toUtf8();
    strncpy(conn->uuid, uuid.constData(), NM_MAX_UUID_LEN - 1);

    /* Copy settings entries */
    conn->entry_count = settings->count;
    if (conn->entry_count > NM_MAX_SETTINGS_KV)
        conn->entry_count = NM_MAX_SETTINGS_KV;
    memcpy(conn->entries, settings->entries,
           sizeof(NMSettingEntry) * conn->entry_count);

    /* Extract well-known keys */
    for (uint32_t i = 0; i < settings->count; ++i) {
        if (strcmp(settings->entries[i].key, "connection.id") == 0)
            strncpy(conn->id, settings->entries[i].value, NM_MAX_CONN_ID - 1);
        else if (strcmp(settings->entries[i].key, "connection.type") == 0)
            strncpy(conn->type, settings->entries[i].value, NM_MAX_CONN_TYPE - 1);
        else if (strcmp(settings->entries[i].key, "connection.autoconnect") == 0)
            conn->autoconnect = (strcmp(settings->entries[i].value, "true") == 0);
    }

    if (conn->id[0] == '\0')
        snprintf(conn->id, NM_MAX_CONN_ID, "Connection %u", client->profile_count + 1);
    if (conn->type[0] == '\0')
        strncpy(conn->type, "802-3-ethernet", NM_MAX_CONN_TYPE - 1);

    /* Save to disk */
    save_profile(conn);

    client->profile_count++;

    /* Return UUID */
    strncpy(s_uuid_buf, conn->uuid, NM_MAX_UUID_LEN);
    return s_uuid_buf;
}

bool nm_settings_delete_connection(NMClient *client, const char *uuid)
{
    if (!client || !uuid)
        return false;

    for (uint32_t i = 0; i < client->profile_count; ++i) {
        if (strcmp(client->profiles[i].uuid, uuid) != 0)
            continue;

        /* Remove file */
        QString path = QStringLiteral("%1/%2.conf").arg(NM_CONN_DIR, uuid);
        QFile::remove(path);

        /* Shift remaining */
        for (uint32_t j = i; j < client->profile_count - 1; ++j)
            client->profiles[j] = client->profiles[j + 1];
        client->profile_count--;

        qDebug("NM/Veridian: deleted profile %s", uuid);
        return true;
    }

    return false;
}

/* ========================================================================= */
/* Wi-Fi                                                                     */
/* ========================================================================= */

bool nm_wifi_get_access_points(NMClient *client, const char *iface,
                               NMAccessPointList *out)
{
    if (!client || !iface || !out)
        return false;

    NMWifiBackend *wifi = find_wifi_backend(client, iface);
    if (!wifi)
        return false;

    return nm_wifi_get_scan_results(wifi, out);
}

bool nm_wifi_request_scan(NMClient *client, const char *iface)
{
    if (!client || !iface)
        return false;

    if (!client->wireless_enabled) {
        qWarning("NM/Veridian: Wi-Fi disabled, cannot scan");
        return false;
    }

    NMWifiBackend *wifi = find_wifi_backend(client, iface);
    if (!wifi)
        return false;

    return nm_wifi_scan(wifi);
}

/*
 * VeridianOS -- plasma-nm-applet.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Plasma network management applet implementation for VeridianOS.
 *
 * Communicates with the NM shim via D-Bus to display network status,
 * available connections, and Wi-Fi access points in the Plasma system
 * tray.
 */

#include "plasma-nm-applet.h"

#include <QDebug>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusReply>
#include <QDBusVariant>

namespace PlasmaNetworkApplet {

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

static const char *NM_SERVICE   = "org.freedesktop.NetworkManager";
static const char *NM_PATH      = "/org/freedesktop/NetworkManager";
static const char *NM_IFACE     = "org.freedesktop.NetworkManager";

/* NM state values (mirror nm-veridian.h NMState) */
static const uint NM_STATE_CONNECTED_GLOBAL = 70;
static const uint NM_STATE_CONNECTING       = 40;
static const uint NM_STATE_DISCONNECTED     = 20;

/* NM device types */
static const uint NM_DEVICE_TYPE_ETHERNET = 1;
static const uint NM_DEVICE_TYPE_WIFI     = 2;

/* ========================================================================= */
/* PlasmaNetworkApplet                                                       */
/* ========================================================================= */

PlasmaNetworkApplet::PlasmaNetworkApplet(QObject *parent)
    : QObject(parent)
    , m_connected(false)
    , m_wifiEnabled(true)
    , m_activeSignalStrength(-1)
    , m_iconName(QStringLiteral("network-disconnect"))
    , m_nmInterface(nullptr)
    , m_pollTimer(new QTimer(this))
{
    /* Connect to NM D-Bus service */
    m_nmInterface = new QDBusInterface(
        QString::fromLatin1(NM_SERVICE),
        QString::fromLatin1(NM_PATH),
        QString::fromLatin1(NM_IFACE),
        QDBusConnection::systemBus(),
        this);

    if (!m_nmInterface->isValid()) {
        qWarning("PlasmaNetworkApplet: NM D-Bus service not available");
    }

    /* Listen for NM state changes */
    QDBusConnection::systemBus().connect(
        QString::fromLatin1(NM_SERVICE),
        QString::fromLatin1(NM_PATH),
        QString::fromLatin1(NM_IFACE),
        QStringLiteral("StateChanged"),
        this,
        SLOT(onNMStateChanged(uint)));

    connect(m_pollTimer, &QTimer::timeout,
            this, &PlasmaNetworkApplet::pollNMState);

    /* Initial query */
    pollNMState();

    qDebug("PlasmaNetworkApplet: initialized");
}

PlasmaNetworkApplet::~PlasmaNetworkApplet()
{
    stopPolling();
}

/* ========================================================================= */
/* Connectivity status                                                       */
/* ========================================================================= */

bool PlasmaNetworkApplet::isConnected() const
{
    return m_connected;
}

bool PlasmaNetworkApplet::isWifiEnabled() const
{
    return m_wifiEnabled;
}

QString PlasmaNetworkApplet::activeConnectionName() const
{
    return m_activeConnectionName;
}

int PlasmaNetworkApplet::activeSignalStrength() const
{
    return m_activeSignalStrength;
}

/* ========================================================================= */
/* Connection list                                                           */
/* ========================================================================= */

QVector<ConnectionEntry> PlasmaNetworkApplet::connectionList() const
{
    return m_connections;
}

int PlasmaNetworkApplet::connectionCount() const
{
    return m_connections.size();
}

/* ========================================================================= */
/* Actions                                                                   */
/* ========================================================================= */

bool PlasmaNetworkApplet::connectTo(const QString &uuid)
{
    if (!m_nmInterface || !m_nmInterface->isValid())
        return false;

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(NM_SERVICE),
        QString::fromLatin1(NM_PATH),
        QString::fromLatin1(NM_IFACE),
        QStringLiteral("ActivateConnection"));
    msg << uuid;

    QDBusConnection::systemBus().call(msg);

    qDebug("PlasmaNetworkApplet: requested connect to %s", qPrintable(uuid));

    /* Refresh after a short delay */
    QTimer::singleShot(2000, this, &PlasmaNetworkApplet::pollNMState);
    return true;
}

bool PlasmaNetworkApplet::disconnectFrom(const QString &uuid)
{
    if (!m_nmInterface || !m_nmInterface->isValid())
        return false;

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(NM_SERVICE),
        QString::fromLatin1(NM_PATH),
        QString::fromLatin1(NM_IFACE),
        QStringLiteral("DeactivateConnection"));
    msg << uuid;

    QDBusConnection::systemBus().call(msg);

    qDebug("PlasmaNetworkApplet: requested disconnect from %s", qPrintable(uuid));

    QTimer::singleShot(1000, this, &PlasmaNetworkApplet::pollNMState);
    return true;
}

bool PlasmaNetworkApplet::requestWifiScan()
{
    if (!m_nmInterface || !m_nmInterface->isValid())
        return false;

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(NM_SERVICE),
        QString::fromLatin1(NM_PATH),
        QString::fromLatin1(NM_IFACE),
        QStringLiteral("RequestScan"));

    QDBusConnection::systemBus().call(msg);

    /* Refresh after scan completes */
    QTimer::singleShot(6000, this, &PlasmaNetworkApplet::pollNMState);
    return true;
}

bool PlasmaNetworkApplet::setWifiEnabled(bool enabled)
{
    if (!m_nmInterface || !m_nmInterface->isValid())
        return false;

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(NM_SERVICE),
        QString::fromLatin1(NM_PATH),
        QString::fromLatin1(NM_IFACE),
        QStringLiteral("SetWirelessEnabled"));
    msg << enabled;

    QDBusConnection::systemBus().call(msg);

    m_wifiEnabled = enabled;
    Q_EMIT wifiEnabledChanged(enabled);

    QTimer::singleShot(1000, this, &PlasmaNetworkApplet::pollNMState);
    return true;
}

/* ========================================================================= */
/* Icon and tooltip                                                          */
/* ========================================================================= */

QString PlasmaNetworkApplet::iconName() const
{
    return m_iconName;
}

QString PlasmaNetworkApplet::toolTipText() const
{
    if (!m_connected)
        return QStringLiteral("Not connected");

    if (m_activeSignalStrength >= 0) {
        return QStringLiteral("%1 (%2%)")
                   .arg(m_activeConnectionName)
                   .arg(m_activeSignalStrength);
    }

    return m_activeConnectionName;
}

/* ========================================================================= */
/* Polling                                                                   */
/* ========================================================================= */

void PlasmaNetworkApplet::startPolling(int intervalMs)
{
    m_pollTimer->start(intervalMs);
    qDebug("PlasmaNetworkApplet: polling started (%d ms)", intervalMs);
}

void PlasmaNetworkApplet::stopPolling()
{
    m_pollTimer->stop();
}

void PlasmaNetworkApplet::pollNMState()
{
    queryActiveConnections();
    queryWifiAccessPoints();
    updateIcon();
}

void PlasmaNetworkApplet::onNMStateChanged(uint state)
{
    bool wasConnected = m_connected;
    m_connected = (state >= NM_STATE_CONNECTED_GLOBAL);

    if (m_connected != wasConnected) {
        Q_EMIT connectivityChanged(m_connected);
        pollNMState();
    }
}

/* ========================================================================= */
/* D-Bus queries                                                             */
/* ========================================================================= */

void PlasmaNetworkApplet::queryDevices()
{
    /* Query device list from NM */
    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(NM_SERVICE),
        QString::fromLatin1(NM_PATH),
        QString::fromLatin1(NM_IFACE),
        QStringLiteral("GetDevices"));

    QDBusMessage reply = QDBusConnection::systemBus().call(msg);
    if (reply.type() == QDBusMessage::ErrorMessage) {
        qDebug("PlasmaNetworkApplet: GetDevices failed: %s",
               qPrintable(reply.errorMessage()));
    }
}

void PlasmaNetworkApplet::queryActiveConnections()
{
    /* Query active connections from NM */
    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(NM_SERVICE),
        QString::fromLatin1(NM_PATH),
        QString::fromLatin1(NM_IFACE),
        QStringLiteral("GetActiveConnections"));

    QDBusMessage reply = QDBusConnection::systemBus().call(msg);

    /* Build connection list from active connections */
    QVector<ConnectionEntry> newList;

    /* Parse reply arguments into ConnectionEntry structures.
     * The NM shim returns active connections as an array of structs
     * serialized via D-Bus. */
    if (reply.type() == QDBusMessage::ReplyMessage && !reply.arguments().isEmpty()) {
        /* Active connections present -- mark as connected */
        m_connected = true;

        /* In the full implementation, each argument maps to an
         * NMActiveConnection.  For now, create entries from the
         * D-Bus reply. */
        QList<QVariant> args = reply.arguments();
        for (const QVariant &arg : args) {
            ConnectionEntry entry;
            entry.name = arg.toString();
            entry.isActive = true;
            entry.isWifi = false;
            entry.isVpn = false;
            entry.signalStrength = -1;
            entry.hasPassword = false;

            if (!entry.name.isEmpty()) {
                newList.append(entry);
                m_activeConnectionName = entry.name;
            }
        }
    } else {
        m_connected = false;
        m_activeConnectionName.clear();
        m_activeSignalStrength = -1;
    }

    /* Merge with existing Wi-Fi entries (non-active) */
    for (const ConnectionEntry &existing : m_connections) {
        if (!existing.isActive && existing.isWifi) {
            /* Check if already in new list */
            bool found = false;
            for (const ConnectionEntry &ne : newList) {
                if (ne.name == existing.name) {
                    found = true;
                    break;
                }
            }
            if (!found)
                newList.append(existing);
        }
    }

    if (newList != m_connections) {
        m_connections = newList;
        Q_EMIT connectionListChanged();
    }
}

void PlasmaNetworkApplet::queryWifiAccessPoints()
{
    if (!m_wifiEnabled)
        return;

    /* Query Wi-Fi scan results from NM */
    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(NM_SERVICE),
        QString::fromLatin1(NM_PATH),
        QString::fromLatin1(NM_IFACE),
        QStringLiteral("GetAccessPoints"));

    QDBusMessage reply = QDBusConnection::systemBus().call(msg);
    if (reply.type() == QDBusMessage::ErrorMessage)
        return;

    /* Parse AP entries and add to connection list as non-active Wi-Fi entries.
     * In the full implementation, each AP has ssid, bssid, signal, security. */
    if (!reply.arguments().isEmpty()) {
        QList<QVariant> args = reply.arguments();
        for (const QVariant &arg : args) {
            QVariantMap ap = arg.toMap();

            ConnectionEntry entry;
            entry.name = ap.value(QStringLiteral("ssid")).toString();
            entry.uuid.clear();  /* no UUID for unconnected APs */
            entry.isActive = false;
            entry.isWifi = true;
            entry.isVpn = false;
            entry.signalStrength = ap.value(QStringLiteral("signal"), -1).toInt();
            entry.hasPassword = (ap.value(QStringLiteral("security"), 0).toUInt() != 0);

            if (!entry.name.isEmpty()) {
                /* Check if already active */
                bool already = false;
                for (const ConnectionEntry &existing : m_connections) {
                    if (existing.name == entry.name && existing.isActive) {
                        already = true;
                        /* Update signal strength for active connection */
                        m_activeSignalStrength = entry.signalStrength;
                        break;
                    }
                }
                if (!already) {
                    m_connections.append(entry);
                }
            }
        }
    }
}

void PlasmaNetworkApplet::updateIcon()
{
    QString oldIcon = m_iconName;

    if (!m_connected) {
        m_iconName = QStringLiteral("network-disconnect");
    } else if (m_activeSignalStrength >= 0) {
        /* Wi-Fi -- choose icon based on signal strength */
        if (m_activeSignalStrength >= 80)
            m_iconName = QStringLiteral("network-wireless-signal-excellent");
        else if (m_activeSignalStrength >= 55)
            m_iconName = QStringLiteral("network-wireless-signal-good");
        else if (m_activeSignalStrength >= 30)
            m_iconName = QStringLiteral("network-wireless-signal-ok");
        else if (m_activeSignalStrength >= 5)
            m_iconName = QStringLiteral("network-wireless-signal-weak");
        else
            m_iconName = QStringLiteral("network-wireless-signal-none");
    } else {
        /* Wired connection */
        m_iconName = QStringLiteral("network-wired");
    }

    if (m_iconName != oldIcon) {
        qDebug("PlasmaNetworkApplet: icon -> %s", qPrintable(m_iconName));
    }

    /* Check for VPN overlay */
    for (const ConnectionEntry &entry : m_connections) {
        if (entry.isVpn && entry.isActive) {
            m_iconName = QStringLiteral("network-vpn");
            break;
        }
    }
}

/* ========================================================================= */
/* ConnectionEntry comparison (for change detection)                         */
/* ========================================================================= */

} /* namespace PlasmaNetworkApplet */

/* Equality operator for ConnectionEntry (needed for QVector != comparison) */
bool operator==(const PlasmaNetworkApplet::ConnectionEntry &a,
                const PlasmaNetworkApplet::ConnectionEntry &b)
{
    return a.name == b.name
        && a.uuid == b.uuid
        && a.isActive == b.isActive
        && a.isWifi == b.isWifi
        && a.signalStrength == b.signalStrength;
}

bool operator!=(const PlasmaNetworkApplet::ConnectionEntry &a,
                const PlasmaNetworkApplet::ConnectionEntry &b)
{
    return !(a == b);
}

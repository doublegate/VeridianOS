/*
 * VeridianOS -- plasma-bluetooth-applet.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Plasma Bluetooth applet implementation for VeridianOS.
 *
 * Communicates with the BlueZ shim via D-Bus to display adapter state,
 * device list, and provide Bluetooth management actions in the Plasma
 * system tray.
 */

#include "plasma-bluetooth-applet.h"

#include <QDebug>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusReply>
#include <QDBusVariant>

namespace PlasmaBluetoothApplet {

/* ========================================================================= */
/* Constants                                                                 */
/* ========================================================================= */

static const char *BLUEZ_SERVICE        = "org.bluez";
static const char *BLUEZ_ADAPTER_PATH   = "/org/bluez/hci0";
static const char *BLUEZ_ADAPTER_IFACE  = "org.bluez.Adapter1";
static const char *BLUEZ_DEVICE_IFACE   = "org.bluez.Device1";
static const char *BLUEZ_OBJMGR_PATH    = "/org/bluez";
static const char *BLUEZ_OBJMGR_IFACE   = "org.freedesktop.DBus.ObjectManager";
static const char *DBUS_PROPS_IFACE     = "org.freedesktop.DBus.Properties";

/* ========================================================================= */
/* PlasmaBluetoothApplet                                                     */
/* ========================================================================= */

PlasmaBluetoothApplet::PlasmaBluetoothApplet(QObject *parent)
    : QObject(parent)
    , m_powered(false)
    , m_scanning(false)
    , m_iconName(QStringLiteral("bluetooth-disabled"))
    , m_connectedCount(0)
    , m_bluezInterface(nullptr)
    , m_pollTimer(new QTimer(this))
{
    /* Connect to BlueZ adapter D-Bus interface */
    m_bluezInterface = new QDBusInterface(
        QString::fromLatin1(BLUEZ_SERVICE),
        QString::fromLatin1(BLUEZ_ADAPTER_PATH),
        QString::fromLatin1(BLUEZ_ADAPTER_IFACE),
        QDBusConnection::systemBus(),
        this);

    if (!m_bluezInterface->isValid()) {
        qWarning("PlasmaBluetoothApplet: BlueZ D-Bus service not available");
    }

    /* Listen for property changes on adapter */
    QDBusConnection::systemBus().connect(
        QString::fromLatin1(BLUEZ_SERVICE),
        QString::fromLatin1(BLUEZ_ADAPTER_PATH),
        QString::fromLatin1(DBUS_PROPS_IFACE),
        QStringLiteral("PropertiesChanged"),
        this,
        SLOT(onPropertiesChanged(QString, QVariantMap)));

    connect(m_pollTimer, &QTimer::timeout,
            this, &PlasmaBluetoothApplet::pollBlueZState);

    /* Initial query */
    pollBlueZState();

    qDebug("PlasmaBluetoothApplet: initialized");
}

PlasmaBluetoothApplet::~PlasmaBluetoothApplet()
{
    stopPolling();
}

/* ========================================================================= */
/* Adapter status                                                            */
/* ========================================================================= */

bool PlasmaBluetoothApplet::isPowered() const
{
    return m_powered;
}

bool PlasmaBluetoothApplet::isScanning() const
{
    return m_scanning;
}

QString PlasmaBluetoothApplet::adapterName() const
{
    return m_adapterName;
}

/* ========================================================================= */
/* Device list                                                               */
/* ========================================================================= */

QVector<BtDeviceEntry> PlasmaBluetoothApplet::getDevices() const
{
    return m_devices;
}

int PlasmaBluetoothApplet::deviceCount() const
{
    return m_devices.size();
}

/* ========================================================================= */
/* Actions                                                                   */
/* ========================================================================= */

bool PlasmaBluetoothApplet::setAdapterPowered(bool powered)
{
    if (!m_bluezInterface || !m_bluezInterface->isValid())
        return false;

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(BLUEZ_SERVICE),
        QString::fromLatin1(BLUEZ_ADAPTER_PATH),
        QString::fromLatin1(DBUS_PROPS_IFACE),
        QStringLiteral("Set"));
    msg << QString::fromLatin1(BLUEZ_ADAPTER_IFACE)
        << QStringLiteral("Powered")
        << QVariant::fromValue(QDBusVariant(powered));

    QDBusConnection::systemBus().call(msg);

    m_powered = powered;
    Q_EMIT adapterChanged(powered);
    updateIcon();

    qDebug("PlasmaBluetoothApplet: adapter powered %s",
           powered ? "on" : "off");

    QTimer::singleShot(1000, this, &PlasmaBluetoothApplet::pollBlueZState);
    return true;
}

bool PlasmaBluetoothApplet::startScan()
{
    if (!m_bluezInterface || !m_bluezInterface->isValid() || !m_powered)
        return false;

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(BLUEZ_SERVICE),
        QString::fromLatin1(BLUEZ_ADAPTER_PATH),
        QString::fromLatin1(BLUEZ_ADAPTER_IFACE),
        QStringLiteral("StartDiscovery"));

    QDBusConnection::systemBus().call(msg);

    m_scanning = true;
    Q_EMIT scanningChanged(true);

    qDebug("PlasmaBluetoothApplet: started scanning");

    /* Poll more frequently during scan */
    QTimer::singleShot(3000, this, &PlasmaBluetoothApplet::pollBlueZState);
    return true;
}

bool PlasmaBluetoothApplet::stopScan()
{
    if (!m_bluezInterface || !m_bluezInterface->isValid())
        return false;

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(BLUEZ_SERVICE),
        QString::fromLatin1(BLUEZ_ADAPTER_PATH),
        QString::fromLatin1(BLUEZ_ADAPTER_IFACE),
        QStringLiteral("StopDiscovery"));

    QDBusConnection::systemBus().call(msg);

    m_scanning = false;
    Q_EMIT scanningChanged(false);

    qDebug("PlasmaBluetoothApplet: stopped scanning");
    return true;
}

bool PlasmaBluetoothApplet::connectDevice(const QString &address)
{
    if (!m_bluezInterface || !m_bluezInterface->isValid())
        return false;

    /* Build device object path from address */
    QString devPath = QStringLiteral("/org/bluez/hci0/dev_%1")
                          .arg(QString(address).replace(':', '_'));

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(BLUEZ_SERVICE),
        devPath,
        QString::fromLatin1(BLUEZ_DEVICE_IFACE),
        QStringLiteral("Connect"));

    QDBusConnection::systemBus().call(msg);

    qDebug("PlasmaBluetoothApplet: connecting to %s", qPrintable(address));
    Q_EMIT deviceConnected(address);

    QTimer::singleShot(3000, this, &PlasmaBluetoothApplet::pollBlueZState);
    return true;
}

bool PlasmaBluetoothApplet::disconnectDevice(const QString &address)
{
    if (!m_bluezInterface || !m_bluezInterface->isValid())
        return false;

    QString devPath = QStringLiteral("/org/bluez/hci0/dev_%1")
                          .arg(QString(address).replace(':', '_'));

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(BLUEZ_SERVICE),
        devPath,
        QString::fromLatin1(BLUEZ_DEVICE_IFACE),
        QStringLiteral("Disconnect"));

    QDBusConnection::systemBus().call(msg);

    qDebug("PlasmaBluetoothApplet: disconnecting from %s", qPrintable(address));
    Q_EMIT deviceDisconnected(address);

    QTimer::singleShot(1000, this, &PlasmaBluetoothApplet::pollBlueZState);
    return true;
}

bool PlasmaBluetoothApplet::pairDevice(const QString &address)
{
    if (!m_bluezInterface || !m_bluezInterface->isValid())
        return false;

    QString devPath = QStringLiteral("/org/bluez/hci0/dev_%1")
                          .arg(QString(address).replace(':', '_'));

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(BLUEZ_SERVICE),
        devPath,
        QString::fromLatin1(BLUEZ_DEVICE_IFACE),
        QStringLiteral("Pair"));

    QDBusConnection::systemBus().call(msg);

    qDebug("PlasmaBluetoothApplet: pairing with %s", qPrintable(address));

    QTimer::singleShot(5000, this, &PlasmaBluetoothApplet::pollBlueZState);
    return true;
}

bool PlasmaBluetoothApplet::removeDevice(const QString &address)
{
    if (!m_bluezInterface || !m_bluezInterface->isValid())
        return false;

    QString devPath = QStringLiteral("/org/bluez/hci0/dev_%1")
                          .arg(QString(address).replace(':', '_'));

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(BLUEZ_SERVICE),
        QString::fromLatin1(BLUEZ_ADAPTER_PATH),
        QString::fromLatin1(BLUEZ_ADAPTER_IFACE),
        QStringLiteral("RemoveDevice"));
    msg << devPath;

    QDBusConnection::systemBus().call(msg);

    qDebug("PlasmaBluetoothApplet: removing device %s", qPrintable(address));

    QTimer::singleShot(1000, this, &PlasmaBluetoothApplet::pollBlueZState);
    return true;
}

/* ========================================================================= */
/* Icon and tooltip                                                          */
/* ========================================================================= */

QString PlasmaBluetoothApplet::iconName() const
{
    return m_iconName;
}

QString PlasmaBluetoothApplet::toolTipText() const
{
    if (!m_powered)
        return QStringLiteral("Bluetooth Off");

    if (m_connectedCount > 0) {
        return QStringLiteral("Bluetooth: %1 device(s) connected")
                   .arg(m_connectedCount);
    }

    if (m_scanning)
        return QStringLiteral("Bluetooth: Scanning...");

    return QStringLiteral("Bluetooth: No devices connected");
}

/* ========================================================================= */
/* Polling                                                                   */
/* ========================================================================= */

void PlasmaBluetoothApplet::startPolling(int intervalMs)
{
    m_pollTimer->start(intervalMs);
    qDebug("PlasmaBluetoothApplet: polling started (%d ms)", intervalMs);
}

void PlasmaBluetoothApplet::stopPolling()
{
    m_pollTimer->stop();
}

void PlasmaBluetoothApplet::pollBlueZState()
{
    queryAdapter();
    queryDevices();
    updateIcon();
}

void PlasmaBluetoothApplet::onPropertiesChanged(const QString &interface,
                                                   const QVariantMap &changed)
{
    if (interface == QString::fromLatin1(BLUEZ_ADAPTER_IFACE)) {
        if (changed.contains(QStringLiteral("Powered"))) {
            bool powered = changed.value(QStringLiteral("Powered")).toBool();
            if (m_powered != powered) {
                m_powered = powered;
                Q_EMIT adapterChanged(powered);
                updateIcon();
            }
        }
        if (changed.contains(QStringLiteral("Discovering"))) {
            bool scanning = changed.value(QStringLiteral("Discovering")).toBool();
            if (m_scanning != scanning) {
                m_scanning = scanning;
                Q_EMIT scanningChanged(scanning);
            }
        }
    }
}

/* ========================================================================= */
/* D-Bus queries                                                             */
/* ========================================================================= */

void PlasmaBluetoothApplet::queryAdapter()
{
    if (!m_bluezInterface || !m_bluezInterface->isValid())
        return;

    /* Read adapter properties */
    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(BLUEZ_SERVICE),
        QString::fromLatin1(BLUEZ_ADAPTER_PATH),
        QString::fromLatin1(DBUS_PROPS_IFACE),
        QStringLiteral("GetAll"));
    msg << QString::fromLatin1(BLUEZ_ADAPTER_IFACE);

    QDBusMessage reply = QDBusConnection::systemBus().call(msg);
    if (reply.type() == QDBusMessage::ErrorMessage)
        return;

    if (!reply.arguments().isEmpty()) {
        QVariantMap props = reply.arguments().first().toMap();
        m_powered = props.value(QStringLiteral("Powered"), false).toBool();
        m_scanning = props.value(QStringLiteral("Discovering"), false).toBool();
        m_adapterName = props.value(QStringLiteral("Alias"),
                                     QStringLiteral("Bluetooth")).toString();
    }
}

void PlasmaBluetoothApplet::queryDevices()
{
    /* Use ObjectManager.GetManagedObjects to enumerate all devices */
    QDBusMessage msg = QDBusMessage::createMethodCall(
        QString::fromLatin1(BLUEZ_SERVICE),
        QString::fromLatin1(BLUEZ_OBJMGR_PATH),
        QString::fromLatin1(BLUEZ_OBJMGR_IFACE),
        QStringLiteral("GetManagedObjects"));

    QDBusMessage reply = QDBusConnection::systemBus().call(msg);
    if (reply.type() == QDBusMessage::ErrorMessage)
        return;

    QVector<BtDeviceEntry> newList;
    m_connectedCount = 0;

    /* Parse managed objects response.
     * Each object has a path and a dict of interfaces with properties. */
    if (!reply.arguments().isEmpty()) {
        QVariantMap objects = reply.arguments().first().toMap();

        for (auto it = objects.constBegin(); it != objects.constEnd(); ++it) {
            QString path = it.key();

            /* Skip non-device paths */
            if (!path.startsWith(QStringLiteral("/org/bluez/hci0/dev_")))
                continue;

            QVariantMap interfaces = it.value().toMap();
            QVariantMap devProps = interfaces.value(
                QString::fromLatin1(BLUEZ_DEVICE_IFACE)).toMap();

            if (devProps.isEmpty())
                continue;

            BtDeviceEntry entry;
            entry.name = devProps.value(QStringLiteral("Alias"),
                                         QStringLiteral("Unknown")).toString();
            entry.address = devProps.value(QStringLiteral("Address")).toString();
            entry.paired = devProps.value(QStringLiteral("Paired"), false).toBool();
            entry.connected = devProps.value(QStringLiteral("Connected"), false).toBool();
            entry.deviceType = devProps.value(QStringLiteral("AddressType"), 0).toInt();
            entry.rssi = devProps.value(QStringLiteral("RSSI"), 0).toInt();
            entry.signalBars = rssiToBars(entry.rssi);

            uint32_t devClass = devProps.value(QStringLiteral("Class"), 0u).toUInt();
            entry.iconName = deviceTypeToIcon(entry.deviceType, devClass);

            if (entry.connected)
                m_connectedCount++;

            newList.append(entry);
        }
    }

    /* Sort: paired+connected first, then paired, then discovered */
    std::sort(newList.begin(), newList.end(),
              [](const BtDeviceEntry &a, const BtDeviceEntry &b) {
        if (a.connected != b.connected) return a.connected > b.connected;
        if (a.paired != b.paired) return a.paired > b.paired;
        return a.name < b.name;
    });

    if (newList != m_devices) {
        m_devices = newList;
        Q_EMIT deviceListChanged();
    }
}

void PlasmaBluetoothApplet::updateIcon()
{
    QString oldIcon = m_iconName;

    if (!m_powered) {
        m_iconName = QStringLiteral("bluetooth-disabled");
    } else if (m_connectedCount > 0) {
        m_iconName = QStringLiteral("bluetooth-active");
    } else {
        m_iconName = QStringLiteral("bluetooth-inactive");
    }

    if (m_iconName != oldIcon) {
        qDebug("PlasmaBluetoothApplet: icon -> %s", qPrintable(m_iconName));
    }
}

int PlasmaBluetoothApplet::rssiToBars(int rssi) const
{
    /* Convert RSSI (dBm) to 0-4 signal bars.
     * Typical range: -30 (excellent) to -90 (very weak). */
    if (rssi == 0) return 0;       /* unknown */
    if (rssi >= -40) return 4;     /* excellent */
    if (rssi >= -55) return 3;     /* good */
    if (rssi >= -70) return 2;     /* fair */
    if (rssi >= -85) return 1;     /* weak */
    return 0;                      /* very weak */
}

QString PlasmaBluetoothApplet::deviceTypeToIcon(int type,
                                                  uint32_t deviceClass) const
{
    /* Map Bluetooth Class of Device major classes to icon names.
     * CoD bits [12:8] = major device class. */
    uint8_t majorClass = (deviceClass >> 8) & 0x1F;

    switch (majorClass) {
    case 0x01:  return QStringLiteral("computer");
    case 0x02:  return QStringLiteral("phone");
    case 0x03:  return QStringLiteral("network-wireless");
    case 0x04:  /* Audio/Video */
        /* Minor class [7:2] distinguishes headset/speaker/etc. */
        {
            uint8_t minorClass = (deviceClass >> 2) & 0x3F;
            if (minorClass == 0x01 || minorClass == 0x02)
                return QStringLiteral("audio-headset");
            if (minorClass == 0x06)
                return QStringLiteral("audio-headphones");
            return QStringLiteral("audio-speakers");
        }
    case 0x05:  /* Peripheral */
        {
            uint8_t minorClass = (deviceClass >> 2) & 0x3F;
            if (minorClass & 0x10)
                return QStringLiteral("input-keyboard");
            if (minorClass & 0x20)
                return QStringLiteral("input-mouse");
            if (minorClass & 0x01)
                return QStringLiteral("input-gaming");
            return QStringLiteral("input-keyboard");
        }
    case 0x06:  return QStringLiteral("camera-photo");
    case 0x07:  return QStringLiteral("printer");
    default:    return QStringLiteral("bluetooth-active");
    }
}

/* ========================================================================= */
/* BtDeviceEntry comparison (for change detection)                           */
/* ========================================================================= */

} /* namespace PlasmaBluetoothApplet */

/* Equality operator for BtDeviceEntry (needed for QVector != comparison) */
bool operator==(const PlasmaBluetoothApplet::BtDeviceEntry &a,
                const PlasmaBluetoothApplet::BtDeviceEntry &b)
{
    return a.address == b.address
        && a.name == b.name
        && a.paired == b.paired
        && a.connected == b.connected
        && a.rssi == b.rssi;
}

bool operator!=(const PlasmaBluetoothApplet::BtDeviceEntry &a,
                const PlasmaBluetoothApplet::BtDeviceEntry &b)
{
    return !(a == b);
}

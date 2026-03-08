/*
 * VeridianOS -- plasma-bluetooth-applet.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Plasma Bluetooth applet for the VeridianOS system tray.
 * Displays adapter status, discovered/paired devices, and provides
 * connect/disconnect/pair actions via the BlueZ D-Bus interface.
 *
 * Integrates with Plasma's system tray as a StatusNotifierItem and
 * queries the VeridianOS BlueZ shim at org.bluez.
 */

#ifndef PLASMA_BLUETOOTH_APPLET_H
#define PLASMA_BLUETOOTH_APPLET_H

#include <QObject>
#include <QString>
#include <QTimer>
#include <QDBusInterface>
#include <QVector>

namespace PlasmaBluetoothApplet {

/* ========================================================================= */
/* BtDeviceEntry -- displayed in the applet's device list                    */
/* ========================================================================= */

struct BtDeviceEntry
{
    QString name;
    QString address;            /* "XX:XX:XX:XX:XX:XX" */
    bool paired;
    bool connected;
    int deviceType;             /* 0=unknown, 1=BR/EDR, 2=LE, 3=dual */
    int rssi;                   /* signal strength in dBm, or 0 if unknown */
    int signalBars;             /* 0-4 bars derived from RSSI */
    QString iconName;           /* device type icon name */
};

/* ========================================================================= */
/* PlasmaBluetoothApplet -- system tray Bluetooth indicator                  */
/* ========================================================================= */

/**
 * Bluetooth applet for Plasma's system tray.
 *
 * Shows:
 *   - Adapter power state (on/off toggle)
 *   - Scanning state (start/stop discovery)
 *   - List of devices (paired first, then discovered)
 *   - Connect/Disconnect/Pair actions per device
 *   - Signal strength bars for discovered devices
 *   - Device type icons (phone, headset, keyboard, etc.)
 *
 * Periodically polls the BlueZ D-Bus service for state changes
 * and refreshes the display.
 */
class PlasmaBluetoothApplet : public QObject
{
    Q_OBJECT

public:
    explicit PlasmaBluetoothApplet(QObject *parent = nullptr);
    ~PlasmaBluetoothApplet() override;

    /* ----- Adapter status ----- */
    bool isPowered() const;
    bool isScanning() const;
    QString adapterName() const;

    /* ----- Device list ----- */
    QVector<BtDeviceEntry> getDevices() const;
    int deviceCount() const;

    /* ----- Actions ----- */
    bool setAdapterPowered(bool powered);
    bool startScan();
    bool stopScan();
    bool connectDevice(const QString &address);
    bool disconnectDevice(const QString &address);
    bool pairDevice(const QString &address);
    bool removeDevice(const QString &address);

    /* ----- Icon (for StatusNotifierItem) ----- */
    QString iconName() const;
    QString toolTipText() const;

    /* ----- Polling ----- */
    void startPolling(int intervalMs = 5000);
    void stopPolling();

Q_SIGNALS:
    void adapterChanged(bool powered);
    void deviceFound(const QString &address, const QString &name);
    void deviceConnected(const QString &address);
    void deviceDisconnected(const QString &address);
    void deviceListChanged();
    void scanningChanged(bool scanning);

private Q_SLOTS:
    void pollBlueZState();
    void onPropertiesChanged(const QString &interface,
                              const QVariantMap &changed);

private:
    /* ----- D-Bus helpers ----- */
    void queryAdapter();
    void queryDevices();
    void updateIcon();
    int rssiToBars(int rssi) const;
    QString deviceTypeToIcon(int type, uint32_t deviceClass) const;

    /* ----- State ----- */
    QVector<BtDeviceEntry> m_devices;
    bool m_powered;
    bool m_scanning;
    QString m_adapterName;
    QString m_iconName;
    int m_connectedCount;

    /* ----- D-Bus ----- */
    QDBusInterface *m_bluezInterface;
    QTimer *m_pollTimer;
};

} /* namespace PlasmaBluetoothApplet */

#endif /* PLASMA_BLUETOOTH_APPLET_H */

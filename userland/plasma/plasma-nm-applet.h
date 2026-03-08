/*
 * VeridianOS -- plasma-nm-applet.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Plasma network management applet for the VeridianOS system tray.
 * Displays connection status, Wi-Fi scan results, and provides
 * connect/disconnect actions via the NM D-Bus interface.
 *
 * Integrates with Plasma's system tray as a StatusNotifierItem and
 * queries the VeridianOS NetworkManager shim at
 * org.freedesktop.NetworkManager.
 */

#ifndef PLASMA_NM_APPLET_H
#define PLASMA_NM_APPLET_H

#include <QObject>
#include <QString>
#include <QTimer>
#include <QDBusInterface>
#include <QVector>

namespace PlasmaNetworkApplet {

/* ========================================================================= */
/* Connection entry -- displayed in the applet's list                        */
/* ========================================================================= */

struct ConnectionEntry
{
    QString name;               /* SSID or connection profile name */
    QString uuid;               /* NM connection UUID */
    QString device;             /* interface name */
    int signalStrength;         /* 0-100 (Wi-Fi) or -1 (wired) */
    bool isActive;
    bool isWifi;
    bool isVpn;
    bool hasPassword;           /* requires authentication */
};

/* ========================================================================= */
/* PlasmaNetworkApplet -- system tray network indicator                      */
/* ========================================================================= */

/**
 * Network applet for Plasma's system tray.
 *
 * Shows:
 *   - Current connectivity icon (disconnected/wired/wifi-strength/vpn)
 *   - List of available connections (active first, then Wi-Fi APs)
 *   - Connect/Disconnect/Forget actions per entry
 *   - Global Wi-Fi toggle
 *   - Signal strength bars for Wi-Fi
 *
 * Periodically polls the NM D-Bus service for state changes and
 * refreshes the display.
 */
class PlasmaNetworkApplet : public QObject
{
    Q_OBJECT

public:
    explicit PlasmaNetworkApplet(QObject *parent = nullptr);
    ~PlasmaNetworkApplet() override;

    /* ----- Connectivity status ----- */
    bool isConnected() const;
    bool isWifiEnabled() const;
    QString activeConnectionName() const;
    int activeSignalStrength() const;     /* 0-100 or -1 */

    /* ----- Connection list ----- */
    QVector<ConnectionEntry> connectionList() const;
    int connectionCount() const;

    /* ----- Actions ----- */
    bool connectTo(const QString &uuid);
    bool disconnectFrom(const QString &uuid);
    bool requestWifiScan();
    bool setWifiEnabled(bool enabled);

    /* ----- Icon (for StatusNotifierItem) ----- */
    QString iconName() const;
    QString toolTipText() const;

    /* ----- Polling ----- */
    void startPolling(int intervalMs = 5000);
    void stopPolling();

Q_SIGNALS:
    void connectivityChanged(bool connected);
    void connectionListChanged();
    void wifiEnabledChanged(bool enabled);
    void signalStrengthChanged(int percent);

private Q_SLOTS:
    void pollNMState();
    void onNMStateChanged(uint state);

private:
    /* ----- D-Bus helpers ----- */
    void queryDevices();
    void queryActiveConnections();
    void queryWifiAccessPoints();
    void updateIcon();

    /* ----- State ----- */
    QVector<ConnectionEntry> m_connections;
    bool m_connected;
    bool m_wifiEnabled;
    QString m_activeConnectionName;
    int m_activeSignalStrength;
    QString m_iconName;

    /* ----- D-Bus ----- */
    QDBusInterface *m_nmInterface;
    QTimer *m_pollTimer;
};

} /* namespace PlasmaNetworkApplet */

#endif /* PLASMA_NM_APPLET_H */

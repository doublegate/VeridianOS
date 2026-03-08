/*
 * VeridianOS -- solid-veridian-backend.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Solid device backend for VeridianOS.  Implements Solid::DeviceInterface
 * to provide hardware device enumeration, property queries, and hot-plug
 * monitoring for the KDE Frameworks Solid library.
 *
 * Solid is KDE's hardware abstraction layer.  This backend replaces the
 * Linux-specific UDev/sysfs backend with one that queries VeridianOS
 * device nodes directly.
 */

#ifndef SOLID_VERIDIAN_BACKEND_H
#define SOLID_VERIDIAN_BACKEND_H

#include <Solid/DeviceInterface>
#include <Solid/Device>
#include <Solid/Block>
#include <Solid/StorageDrive>
#include <Solid/StorageVolume>
#include <Solid/StorageAccess>
#include <Solid/NetworkInterface>
#include <Solid/AudioInterface>
#include <Solid/Processor>
#include <Solid/Battery>
#include <Solid/GenericInterface>

#include <QObject>
#include <QString>
#include <QStringList>
#include <QMap>
#include <QVariant>
#include <QSocketNotifier>

namespace Solid {
namespace Backends {
namespace Veridian {

/* ========================================================================= */
/* VeridianDevice                                                            */
/* ========================================================================= */

/**
 * Represents a single hardware device on VeridianOS.
 *
 * Each device has a UDI (Unique Device Identifier) of the form:
 *   /org/veridian/devices/<type>/<name>
 *
 * Device information is gathered from /dev entries, /proc/cpuinfo,
 * and VeridianOS-specific sysfs-like interfaces.
 */
class VeridianDevice : public Solid::Ifaces::Device
{
    Q_OBJECT

public:
    explicit VeridianDevice(const QString &udi, QObject *parent = nullptr);
    ~VeridianDevice() override;

    /* Solid::Ifaces::Device interface */
    QString udi() const override;
    QString parentUdi() const override;
    QString vendor() const override;
    QString product() const override;
    QString description() const override;
    QString icon() const override;
    QStringList emblems() const override;
    bool isValid() const override;

    bool queryDeviceInterface(const Solid::DeviceInterface::Type &type) const override;
    QObject *createDeviceInterface(const Solid::DeviceInterface::Type &type) override;

    /* VeridianOS-specific */
    QString devicePath() const;
    QMap<QString, QVariant> allProperties() const;

private:
    QString m_udi;
    QString m_parentUdi;
    QString m_vendor;
    QString m_product;
    QString m_description;
    QString m_icon;
    QString m_devicePath;
    QList<Solid::DeviceInterface::Type> m_supportedInterfaces;
    QMap<QString, QVariant> m_properties;
    bool m_valid;
};

/* ========================================================================= */
/* VeridianDeviceManager                                                     */
/* ========================================================================= */

/**
 * Device manager backend for VeridianOS.
 *
 * Enumerates hardware devices by scanning:
 *   - /dev/sd*, /dev/vd* -- block devices
 *   - /dev/input/event*  -- input devices
 *   - /dev/snd/*         -- audio devices
 *   - /dev/dri/*         -- display/GPU devices
 *   - /dev/net/*         -- network interfaces
 *   - /proc/cpuinfo      -- processors
 *
 * Hot-plug monitoring via inotify on /dev.
 */
class VeridianManager : public Solid::Ifaces::DeviceManager
{
    Q_OBJECT

public:
    explicit VeridianManager(QObject *parent = nullptr);
    ~VeridianManager() override;

    /* Solid::Ifaces::DeviceManager interface */
    QStringList allDevices() override;
    QStringList devicesFromQuery(const QString &parentUdi,
                                Solid::DeviceInterface::Type type) override;
    QObject *createDevice(const QString &udi) override;

Q_SIGNALS:
    void deviceAdded(const QString &udi);
    void deviceRemoved(const QString &udi);

private Q_SLOTS:
    void onInotifyEvent();

private:
    void enumerateBlockDevices();
    void enumerateNetworkDevices();
    void enumerateAudioDevices();
    void enumerateDisplayDevices();
    void enumerateProcessors();
    void enumerateBatteries();

    void startHotplugMonitor();

    QStringList m_deviceUdis;
    QMap<QString, VeridianDevice *> m_devices;
    int m_inotifyFd;
    QSocketNotifier *m_inotifyNotifier;
};

/* ========================================================================= */
/* Device interface implementations                                          */
/* ========================================================================= */

class VeridianStorageDrive : public QObject, virtual public Solid::Ifaces::StorageDrive
{
    Q_OBJECT
    Q_INTERFACES(Solid::Ifaces::StorageDrive)

public:
    explicit VeridianStorageDrive(VeridianDevice *device);

    Solid::StorageDrive::Bus bus() const override;
    Solid::StorageDrive::DriveType driveType() const override;
    bool isRemovable() const override;
    bool isHotpluggable() const override;
    qulonglong size() const override;

private:
    VeridianDevice *m_device;
};

class VeridianStorageVolume : public QObject, virtual public Solid::Ifaces::StorageVolume
{
    Q_OBJECT
    Q_INTERFACES(Solid::Ifaces::StorageVolume)

public:
    explicit VeridianStorageVolume(VeridianDevice *device);

    bool isIgnored() const override;
    Solid::StorageVolume::UsageType usage() const override;
    QString fsType() const override;
    QString label() const override;
    QString uuid() const override;
    qulonglong size() const override;
    QString device() const;

private:
    VeridianDevice *m_device;
};

class VeridianNetworkInterface : public QObject, virtual public Solid::Ifaces::NetworkInterface
{
    Q_OBJECT
    Q_INTERFACES(Solid::Ifaces::NetworkInterface)

public:
    explicit VeridianNetworkInterface(VeridianDevice *device);

    QString ifaceName() const override;
    bool isWireless() const override;
    bool isLoopback() const override;
    QString hwAddress() const override;
    qulonglong macAddress() const;

private:
    VeridianDevice *m_device;
};

class VeridianProcessor : public QObject, virtual public Solid::Ifaces::Processor
{
    Q_OBJECT
    Q_INTERFACES(Solid::Ifaces::Processor)

public:
    explicit VeridianProcessor(VeridianDevice *device);

    int number() const override;
    int maxSpeed() const override;
    bool canChangeFrequency() const override;
    Solid::Processor::InstructionSets instructionSets() const override;

private:
    VeridianDevice *m_device;
};

class VeridianBattery : public QObject, virtual public Solid::Ifaces::Battery
{
    Q_OBJECT
    Q_INTERFACES(Solid::Ifaces::Battery)

public:
    explicit VeridianBattery(VeridianDevice *device);

    bool isPresent() const override;
    Solid::Battery::BatteryType type() const override;
    int chargePercent() const override;
    int capacity() const override;
    bool isRechargeable() const override;
    bool isPowerSupply() const override;
    Solid::Battery::ChargeState chargeState() const override;
    qlonglong timeToEmpty() const override;
    qlonglong timeToFull() const override;
    double energy() const override;
    double energyFull() const override;
    double energyFullDesign() const override;
    double energyRate() const override;
    double voltage() const override;
    double temperature() const override;
    QString serial() const override;

Q_SIGNALS:
    void chargePercentChanged(int value, const QString &udi);
    void chargeStateChanged(int newState, const QString &udi);
    void presentStateChanged(bool newState, const QString &udi);

private:
    VeridianDevice *m_device;
};

} /* namespace Veridian */
} /* namespace Backends */
} /* namespace Solid */

#endif /* SOLID_VERIDIAN_BACKEND_H */

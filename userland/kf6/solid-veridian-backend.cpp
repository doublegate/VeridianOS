/*
 * VeridianOS -- solid-veridian-backend.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Solid device backend implementation for VeridianOS.  Enumerates
 * hardware devices by scanning /dev, /proc, and VeridianOS-specific
 * interfaces.  Provides hot-plug monitoring via inotify on /dev.
 */

#include "solid-veridian-backend.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QTextStream>
#include <QDebug>

#include <sys/types.h>
#include <sys/stat.h>
#include <sys/inotify.h>
#include <dirent.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>

namespace Solid {
namespace Backends {
namespace Veridian {

/* ========================================================================= */
/* UDI prefix constants                                                      */
/* ========================================================================= */

static const QString UDI_PREFIX       = QStringLiteral("/org/veridian/devices");
static const QString UDI_BLOCK        = UDI_PREFIX + QStringLiteral("/block");
static const QString UDI_NET          = UDI_PREFIX + QStringLiteral("/net");
static const QString UDI_AUDIO        = UDI_PREFIX + QStringLiteral("/audio");
static const QString UDI_DISPLAY      = UDI_PREFIX + QStringLiteral("/display");
static const QString UDI_PROCESSOR    = UDI_PREFIX + QStringLiteral("/processor");
static const QString UDI_BATTERY      = UDI_PREFIX + QStringLiteral("/battery");

/* ========================================================================= */
/* VeridianDevice                                                            */
/* ========================================================================= */

VeridianDevice::VeridianDevice(const QString &udi, QObject *parent)
    : QObject(parent)
    , m_udi(udi)
    , m_valid(false)
{
    /* Determine device type and properties from UDI */
    if (udi.startsWith(UDI_BLOCK)) {
        QString devName = udi.section(QLatin1Char('/'), -1);
        m_devicePath = QStringLiteral("/dev/") + devName;
        m_vendor = QStringLiteral("VeridianOS");
        m_icon = QStringLiteral("drive-harddisk");

        if (devName.startsWith(QStringLiteral("vd"))) {
            m_product = QStringLiteral("VirtIO Block Device");
            m_description = QStringLiteral("VirtIO disk ") + devName;
        } else if (devName.startsWith(QStringLiteral("sd"))) {
            m_product = QStringLiteral("SCSI Block Device");
            m_description = QStringLiteral("SCSI disk ") + devName;
        } else {
            m_product = QStringLiteral("Block Device");
            m_description = QStringLiteral("Block device ") + devName;
        }

        m_supportedInterfaces << Solid::DeviceInterface::Block
                              << Solid::DeviceInterface::StorageDrive
                              << Solid::DeviceInterface::StorageVolume;
        m_parentUdi = UDI_PREFIX;
        m_valid = true;

    } else if (udi.startsWith(UDI_NET)) {
        QString ifName = udi.section(QLatin1Char('/'), -1);
        m_devicePath = QStringLiteral("/dev/net/") + ifName;
        m_vendor = QStringLiteral("VeridianOS");
        m_product = QStringLiteral("Network Interface");
        m_description = QStringLiteral("Network interface ") + ifName;
        m_icon = QStringLiteral("network-wired");

        if (ifName == QStringLiteral("lo")) {
            m_description = QStringLiteral("Loopback Interface");
            m_icon = QStringLiteral("network-wired");
        } else if (ifName.startsWith(QStringLiteral("wl"))) {
            m_description = QStringLiteral("Wireless Interface ") + ifName;
            m_icon = QStringLiteral("network-wireless");
        }

        m_supportedInterfaces << Solid::DeviceInterface::NetworkInterface;
        m_parentUdi = UDI_PREFIX;
        m_valid = true;

    } else if (udi.startsWith(UDI_AUDIO)) {
        QString devName = udi.section(QLatin1Char('/'), -1);
        m_devicePath = QStringLiteral("/dev/snd/") + devName;
        m_vendor = QStringLiteral("VeridianOS");
        m_product = QStringLiteral("Audio Device");
        m_description = QStringLiteral("Audio output ") + devName;
        m_icon = QStringLiteral("audio-card");
        m_supportedInterfaces << Solid::DeviceInterface::AudioInterface;
        m_parentUdi = UDI_PREFIX;
        m_valid = true;

    } else if (udi.startsWith(UDI_DISPLAY)) {
        QString devName = udi.section(QLatin1Char('/'), -1);
        m_devicePath = QStringLiteral("/dev/dri/") + devName;
        m_vendor = QStringLiteral("VeridianOS");
        m_product = QStringLiteral("VirtIO GPU");
        m_description = QStringLiteral("Display output ") + devName;
        m_icon = QStringLiteral("video-display");
        m_parentUdi = UDI_PREFIX;
        m_valid = true;

    } else if (udi.startsWith(UDI_PROCESSOR)) {
        QString cpuNum = udi.section(QLatin1Char('/'), -1);
        m_devicePath.clear();
        m_vendor = QStringLiteral("CPU");
        m_product = QStringLiteral("Processor");
        m_description = QStringLiteral("CPU ") + cpuNum;
        m_icon = QStringLiteral("cpu");
        m_supportedInterfaces << Solid::DeviceInterface::Processor;
        m_parentUdi = UDI_PREFIX;
        m_valid = true;

    } else if (udi.startsWith(UDI_BATTERY)) {
        m_devicePath.clear();
        m_vendor = QStringLiteral("VeridianOS");
        m_product = QStringLiteral("Battery");
        m_description = QStringLiteral("System Battery");
        m_icon = QStringLiteral("battery");
        m_supportedInterfaces << Solid::DeviceInterface::Battery;
        m_parentUdi = UDI_PREFIX;
        m_valid = true;

    } else if (udi == UDI_PREFIX) {
        /* Root device */
        m_vendor = QStringLiteral("VeridianOS");
        m_product = QStringLiteral("Computer");
        m_description = QStringLiteral("VeridianOS System");
        m_icon = QStringLiteral("computer");
        m_valid = true;
    }
}

VeridianDevice::~VeridianDevice()
{
}

QString VeridianDevice::udi() const { return m_udi; }
QString VeridianDevice::parentUdi() const { return m_parentUdi; }
QString VeridianDevice::vendor() const { return m_vendor; }
QString VeridianDevice::product() const { return m_product; }
QString VeridianDevice::description() const { return m_description; }
QString VeridianDevice::icon() const { return m_icon; }
QStringList VeridianDevice::emblems() const { return QStringList(); }
bool VeridianDevice::isValid() const { return m_valid; }
QString VeridianDevice::devicePath() const { return m_devicePath; }
QMap<QString, QVariant> VeridianDevice::allProperties() const { return m_properties; }

bool VeridianDevice::queryDeviceInterface(const Solid::DeviceInterface::Type &type) const
{
    return m_supportedInterfaces.contains(type);
}

QObject *VeridianDevice::createDeviceInterface(const Solid::DeviceInterface::Type &type)
{
    switch (type) {
    case Solid::DeviceInterface::StorageDrive:
        return new VeridianStorageDrive(this);
    case Solid::DeviceInterface::StorageVolume:
        return new VeridianStorageVolume(this);
    case Solid::DeviceInterface::NetworkInterface:
        return new VeridianNetworkInterface(this);
    case Solid::DeviceInterface::Processor:
        return new VeridianProcessor(this);
    case Solid::DeviceInterface::Battery:
        return new VeridianBattery(this);
    default:
        return nullptr;
    }
}

/* ========================================================================= */
/* VeridianManager                                                           */
/* ========================================================================= */

VeridianManager::VeridianManager(QObject *parent)
    : QObject(parent)
    , m_inotifyFd(-1)
    , m_inotifyNotifier(nullptr)
{
    /* Add root device */
    m_deviceUdis << UDI_PREFIX;

    /* Enumerate all device categories */
    enumerateBlockDevices();
    enumerateNetworkDevices();
    enumerateAudioDevices();
    enumerateDisplayDevices();
    enumerateProcessors();
    enumerateBatteries();

    /* Start hot-plug monitoring */
    startHotplugMonitor();
}

VeridianManager::~VeridianManager()
{
    qDeleteAll(m_devices);
    m_devices.clear();

    delete m_inotifyNotifier;
    if (m_inotifyFd >= 0)
        close(m_inotifyFd);
}

QStringList VeridianManager::allDevices()
{
    return m_deviceUdis;
}

QStringList VeridianManager::devicesFromQuery(const QString &parentUdi,
                                              Solid::DeviceInterface::Type type)
{
    QStringList result;

    for (const QString &udi : m_deviceUdis) {
        /* Skip root */
        if (udi == UDI_PREFIX)
            continue;

        /* Filter by parent if specified */
        if (!parentUdi.isEmpty() && parentUdi != UDI_PREFIX) {
            /* Check if udi starts with parentUdi prefix */
            if (!udi.startsWith(parentUdi))
                continue;
        }

        /* Filter by device type if specified */
        if (type != Solid::DeviceInterface::Unknown) {
            VeridianDevice *dev = m_devices.value(udi);
            if (!dev) {
                dev = new VeridianDevice(udi, this);
                m_devices.insert(udi, dev);
            }
            if (!dev->queryDeviceInterface(type))
                continue;
        }

        result << udi;
    }

    return result;
}

QObject *VeridianManager::createDevice(const QString &udi)
{
    if (!m_deviceUdis.contains(udi))
        return nullptr;

    VeridianDevice *dev = m_devices.value(udi);
    if (!dev) {
        dev = new VeridianDevice(udi, this);
        m_devices.insert(udi, dev);
    }
    return dev;
}

/* ========================================================================= */
/* Device enumeration helpers                                                */
/* ========================================================================= */

void VeridianManager::enumerateBlockDevices()
{
    QDir devDir(QStringLiteral("/dev"));
    if (!devDir.exists())
        return;

    /* Scan for sd*, vd*, nvme* block devices */
    const QStringList filters = {
        QStringLiteral("sd[a-z]"),
        QStringLiteral("sd[a-z][0-9]*"),
        QStringLiteral("vd[a-z]"),
        QStringLiteral("vd[a-z][0-9]*"),
        QStringLiteral("nvme[0-9]*"),
    };

    for (const QString &filter : filters) {
        const QStringList entries = devDir.entryList(QStringList(filter),
                                                     QDir::System);
        for (const QString &entry : entries) {
            QString udi = UDI_BLOCK + QLatin1Char('/') + entry;
            if (!m_deviceUdis.contains(udi))
                m_deviceUdis << udi;
        }
    }
}

void VeridianManager::enumerateNetworkDevices()
{
    /* On VeridianOS, network interfaces are exposed in /dev/net/ or
     * can be enumerated via /proc/net/dev */
    QFile procNetDev(QStringLiteral("/proc/net/dev"));
    if (procNetDev.open(QIODevice::ReadOnly)) {
        QTextStream stream(&procNetDev);
        /* Skip header lines */
        stream.readLine();
        stream.readLine();
        while (!stream.atEnd()) {
            QString line = stream.readLine().trimmed();
            int colonPos = line.indexOf(QLatin1Char(':'));
            if (colonPos > 0) {
                QString ifName = line.left(colonPos).trimmed();
                QString udi = UDI_NET + QLatin1Char('/') + ifName;
                if (!m_deviceUdis.contains(udi))
                    m_deviceUdis << udi;
            }
        }
        procNetDev.close();
    } else {
        /* Fallback: add loopback and eth0 as defaults */
        m_deviceUdis << UDI_NET + QStringLiteral("/lo");
        m_deviceUdis << UDI_NET + QStringLiteral("/eth0");
    }
}

void VeridianManager::enumerateAudioDevices()
{
    QDir sndDir(QStringLiteral("/dev/snd"));
    if (!sndDir.exists())
        return;

    const QStringList entries = sndDir.entryList(
        QStringList(QStringLiteral("pcm*")), QDir::System);
    for (const QString &entry : entries) {
        QString udi = UDI_AUDIO + QLatin1Char('/') + entry;
        if (!m_deviceUdis.contains(udi))
            m_deviceUdis << udi;
    }

    /* If no PCM devices found, add a default */
    if (entries.isEmpty()) {
        m_deviceUdis << UDI_AUDIO + QStringLiteral("/default");
    }
}

void VeridianManager::enumerateDisplayDevices()
{
    QDir driDir(QStringLiteral("/dev/dri"));
    if (!driDir.exists())
        return;

    const QStringList entries = driDir.entryList(
        QStringList(QStringLiteral("card*")), QDir::System);
    for (const QString &entry : entries) {
        QString udi = UDI_DISPLAY + QLatin1Char('/') + entry;
        if (!m_deviceUdis.contains(udi))
            m_deviceUdis << udi;
    }
}

void VeridianManager::enumerateProcessors()
{
    /* Read /proc/cpuinfo to count processors */
    QFile cpuinfo(QStringLiteral("/proc/cpuinfo"));
    int cpuCount = 0;

    if (cpuinfo.open(QIODevice::ReadOnly)) {
        QTextStream stream(&cpuinfo);
        while (!stream.atEnd()) {
            QString line = stream.readLine();
            if (line.startsWith(QStringLiteral("processor")))
                cpuCount++;
        }
        cpuinfo.close();
    }

    /* Default to at least 1 CPU */
    if (cpuCount == 0)
        cpuCount = 1;

    for (int i = 0; i < cpuCount; ++i) {
        QString udi = UDI_PROCESSOR + QLatin1Char('/') + QString::number(i);
        if (!m_deviceUdis.contains(udi))
            m_deviceUdis << udi;
    }
}

void VeridianManager::enumerateBatteries()
{
    /* Check if /sys/class/power_supply/BAT0 exists */
    if (QFileInfo::exists(QStringLiteral("/sys/class/power_supply/BAT0"))) {
        QString udi = UDI_BATTERY + QStringLiteral("/BAT0");
        if (!m_deviceUdis.contains(udi))
            m_deviceUdis << udi;
    }
    /* Virtual machines typically have no battery -- that's fine */
}

/* ========================================================================= */
/* Hot-plug monitoring                                                       */
/* ========================================================================= */

void VeridianManager::startHotplugMonitor()
{
    m_inotifyFd = inotify_init1(IN_NONBLOCK | IN_CLOEXEC);
    if (m_inotifyFd < 0) {
        qWarning("VeridianManager: inotify_init1 failed: %s", strerror(errno));
        return;
    }

    /* Watch /dev for device node creation/removal */
    inotify_add_watch(m_inotifyFd, "/dev",
                      IN_CREATE | IN_DELETE | IN_ATTRIB);

    /* Also watch /dev/dri if it exists */
    if (QDir(QStringLiteral("/dev/dri")).exists()) {
        inotify_add_watch(m_inotifyFd, "/dev/dri",
                          IN_CREATE | IN_DELETE);
    }

    /* Also watch /dev/snd if it exists */
    if (QDir(QStringLiteral("/dev/snd")).exists()) {
        inotify_add_watch(m_inotifyFd, "/dev/snd",
                          IN_CREATE | IN_DELETE);
    }

    m_inotifyNotifier = new QSocketNotifier(m_inotifyFd,
                                            QSocketNotifier::Read, this);
    connect(m_inotifyNotifier, &QSocketNotifier::activated,
            this, &VeridianManager::onInotifyEvent);
}

void VeridianManager::onInotifyEvent()
{
    char buf[4096]
        __attribute__((aligned(__alignof__(struct inotify_event))));

    ssize_t len = read(m_inotifyFd, buf, sizeof(buf));
    if (len <= 0)
        return;

    /* Re-enumerate devices and emit add/remove signals */
    QStringList oldDevices = m_deviceUdis;

    m_deviceUdis.clear();
    m_deviceUdis << UDI_PREFIX;
    enumerateBlockDevices();
    enumerateNetworkDevices();
    enumerateAudioDevices();
    enumerateDisplayDevices();
    enumerateProcessors();
    enumerateBatteries();

    /* Find added devices */
    for (const QString &udi : m_deviceUdis) {
        if (!oldDevices.contains(udi))
            Q_EMIT deviceAdded(udi);
    }

    /* Find removed devices */
    for (const QString &udi : oldDevices) {
        if (!m_deviceUdis.contains(udi)) {
            Q_EMIT deviceRemoved(udi);
            /* Clean up cached device object */
            delete m_devices.take(udi);
        }
    }
}

/* ========================================================================= */
/* VeridianStorageDrive                                                      */
/* ========================================================================= */

VeridianStorageDrive::VeridianStorageDrive(VeridianDevice *device)
    : m_device(device)
{
}

Solid::StorageDrive::Bus VeridianStorageDrive::bus() const
{
    QString path = m_device->devicePath();
    if (path.contains(QStringLiteral("vd")))
        return Solid::StorageDrive::Scsi;  /* VirtIO appears as SCSI */
    if (path.contains(QStringLiteral("nvme")))
        return Solid::StorageDrive::Scsi;
    return Solid::StorageDrive::Scsi;
}

Solid::StorageDrive::DriveType VeridianStorageDrive::driveType() const
{
    return Solid::StorageDrive::HardDisk;
}

bool VeridianStorageDrive::isRemovable() const
{
    return false;
}

bool VeridianStorageDrive::isHotpluggable() const
{
    return false;
}

qulonglong VeridianStorageDrive::size() const
{
    /* Read size from /sys/block/<dev>/size (in 512-byte sectors) */
    QString devName = m_device->udi().section(QLatin1Char('/'), -1);
    QFile sizeFile(QStringLiteral("/sys/block/") + devName +
                   QStringLiteral("/size"));
    if (sizeFile.open(QIODevice::ReadOnly)) {
        bool ok;
        qulonglong sectors = sizeFile.readAll().trimmed().toULongLong(&ok);
        sizeFile.close();
        if (ok)
            return sectors * 512ULL;
    }
    return 0;
}

/* ========================================================================= */
/* VeridianStorageVolume                                                     */
/* ========================================================================= */

VeridianStorageVolume::VeridianStorageVolume(VeridianDevice *device)
    : m_device(device)
{
}

bool VeridianStorageVolume::isIgnored() const
{
    return false;
}

Solid::StorageVolume::UsageType VeridianStorageVolume::usage() const
{
    return Solid::StorageVolume::FileSystem;
}

QString VeridianStorageVolume::fsType() const
{
    /* Default to ext4 for VeridianOS rootfs; could be read from /proc/mounts */
    return QStringLiteral("ext4");
}

QString VeridianStorageVolume::label() const
{
    return QStringLiteral("VeridianOS");
}

QString VeridianStorageVolume::uuid() const
{
    return QStringLiteral("00000000-0000-0000-0000-000000000000");
}

qulonglong VeridianStorageVolume::size() const
{
    /* Read from sysfs or /proc/partitions */
    QString devName = m_device->udi().section(QLatin1Char('/'), -1);
    QFile sizeFile(QStringLiteral("/sys/block/") + devName +
                   QStringLiteral("/size"));
    if (sizeFile.open(QIODevice::ReadOnly)) {
        bool ok;
        qulonglong sectors = sizeFile.readAll().trimmed().toULongLong(&ok);
        sizeFile.close();
        if (ok)
            return sectors * 512ULL;
    }
    return 0;
}

QString VeridianStorageVolume::device() const
{
    return m_device->devicePath();
}

/* ========================================================================= */
/* VeridianNetworkInterface                                                  */
/* ========================================================================= */

VeridianNetworkInterface::VeridianNetworkInterface(VeridianDevice *device)
    : m_device(device)
{
}

QString VeridianNetworkInterface::ifaceName() const
{
    return m_device->udi().section(QLatin1Char('/'), -1);
}

bool VeridianNetworkInterface::isWireless() const
{
    QString name = ifaceName();
    return name.startsWith(QStringLiteral("wl")) ||
           name.startsWith(QStringLiteral("wlan"));
}

bool VeridianNetworkInterface::isLoopback() const
{
    return ifaceName() == QStringLiteral("lo");
}

QString VeridianNetworkInterface::hwAddress() const
{
    /* Read from /sys/class/net/<iface>/address */
    QFile addrFile(QStringLiteral("/sys/class/net/") + ifaceName() +
                   QStringLiteral("/address"));
    if (addrFile.open(QIODevice::ReadOnly)) {
        QString addr = QString::fromUtf8(addrFile.readAll()).trimmed();
        addrFile.close();
        return addr;
    }
    return QStringLiteral("00:00:00:00:00:00");
}

qulonglong VeridianNetworkInterface::macAddress() const
{
    return 0;
}

/* ========================================================================= */
/* VeridianProcessor                                                         */
/* ========================================================================= */

VeridianProcessor::VeridianProcessor(VeridianDevice *device)
    : m_device(device)
{
}

int VeridianProcessor::number() const
{
    bool ok;
    int num = m_device->udi().section(QLatin1Char('/'), -1).toInt(&ok);
    return ok ? num : 0;
}

int VeridianProcessor::maxSpeed() const
{
    /* Read from /proc/cpuinfo "cpu MHz" */
    QFile cpuinfo(QStringLiteral("/proc/cpuinfo"));
    if (cpuinfo.open(QIODevice::ReadOnly)) {
        QTextStream stream(&cpuinfo);
        int currentCpu = -1;
        int targetCpu = number();
        while (!stream.atEnd()) {
            QString line = stream.readLine();
            if (line.startsWith(QStringLiteral("processor"))) {
                currentCpu++;
            }
            if (currentCpu == targetCpu &&
                line.startsWith(QStringLiteral("cpu MHz"))) {
                int colonPos = line.indexOf(QLatin1Char(':'));
                if (colonPos > 0) {
                    bool ok;
                    double mhz = line.mid(colonPos + 1).trimmed().toDouble(&ok);
                    cpuinfo.close();
                    return ok ? static_cast<int>(mhz) : 0;
                }
            }
        }
        cpuinfo.close();
    }
    return 0;
}

bool VeridianProcessor::canChangeFrequency() const
{
    return false; /* Not yet supported on VeridianOS */
}

Solid::Processor::InstructionSets VeridianProcessor::instructionSets() const
{
    return Solid::Processor::NoExtensions;
}

/* ========================================================================= */
/* VeridianBattery                                                           */
/* ========================================================================= */

VeridianBattery::VeridianBattery(VeridianDevice *device)
    : m_device(device)
{
}

bool VeridianBattery::isPresent() const
{
    return QFileInfo::exists(QStringLiteral("/sys/class/power_supply/BAT0"));
}

Solid::Battery::BatteryType VeridianBattery::type() const
{
    return Solid::Battery::PrimaryBattery;
}

int VeridianBattery::chargePercent() const
{
    QFile f(QStringLiteral("/sys/class/power_supply/BAT0/capacity"));
    if (f.open(QIODevice::ReadOnly)) {
        bool ok;
        int pct = f.readAll().trimmed().toInt(&ok);
        f.close();
        return ok ? pct : 0;
    }
    return 0;
}

int VeridianBattery::capacity() const
{
    return chargePercent();
}

bool VeridianBattery::isRechargeable() const
{
    return true;
}

bool VeridianBattery::isPowerSupply() const
{
    return true;
}

Solid::Battery::ChargeState VeridianBattery::chargeState() const
{
    QFile f(QStringLiteral("/sys/class/power_supply/BAT0/status"));
    if (f.open(QIODevice::ReadOnly)) {
        QString status = QString::fromUtf8(f.readAll()).trimmed();
        f.close();
        if (status == QStringLiteral("Charging"))
            return Solid::Battery::Charging;
        if (status == QStringLiteral("Discharging"))
            return Solid::Battery::Discharging;
        if (status == QStringLiteral("Full"))
            return Solid::Battery::FullyCharged;
    }
    return Solid::Battery::NoCharge;
}

qlonglong VeridianBattery::timeToEmpty() const { return 0; }
qlonglong VeridianBattery::timeToFull() const { return 0; }
double VeridianBattery::energy() const { return 0.0; }
double VeridianBattery::energyFull() const { return 0.0; }
double VeridianBattery::energyFullDesign() const { return 0.0; }
double VeridianBattery::energyRate() const { return 0.0; }
double VeridianBattery::voltage() const { return 0.0; }
double VeridianBattery::temperature() const { return 0.0; }
QString VeridianBattery::serial() const { return QString(); }

/* ========================================================================= */
/* udev D-Bus Integration (Sprint 10.7)                                      */
/* ========================================================================= */

static const QString UDI_USB = UDI_PREFIX + QStringLiteral("/usb");
static const QString UDI_INPUT = UDI_PREFIX + QStringLiteral("/input");

/**
 * Subscribe to udev device events via D-Bus.
 *
 * Connects to the org.freedesktop.UDev D-Bus service and listens
 * for DeviceAdded and DeviceRemoved signals.  Maps udev subsystems
 * to Solid device types and updates the device list.
 */
void VeridianManager::subscribeUdevEvents()
{
    QDBusConnection bus = QDBusConnection::systemBus();
    if (!bus.isConnected()) {
        qWarning("VeridianManager: D-Bus system bus not available, "
                 "udev integration disabled");
        return;
    }

    /* Connect to DeviceAdded signal */
    bus.connect(
        QStringLiteral("org.freedesktop.UDev"),   /* service */
        QStringLiteral("/org/freedesktop/UDev"),   /* path */
        QStringLiteral("org.freedesktop.UDev"),    /* interface */
        QStringLiteral("DeviceAdded"),             /* signal */
        this,
        SLOT(onUdevDeviceAdded(QString, QString)));

    /* Connect to DeviceRemoved signal */
    bus.connect(
        QStringLiteral("org.freedesktop.UDev"),
        QStringLiteral("/org/freedesktop/UDev"),
        QStringLiteral("org.freedesktop.UDev"),
        QStringLiteral("DeviceRemoved"),
        this,
        SLOT(onUdevDeviceRemoved(QString, QString)));

    /* Connect to DeviceChanged signal */
    bus.connect(
        QStringLiteral("org.freedesktop.UDev"),
        QStringLiteral("/org/freedesktop/UDev"),
        QStringLiteral("org.freedesktop.UDev"),
        QStringLiteral("DeviceChanged"),
        this,
        SLOT(onUdevDeviceChanged(QString, QString)));

    qDebug("VeridianManager: subscribed to udev D-Bus events");
}

/**
 * Map a udev subsystem name to a Solid UDI prefix.
 *
 * @param subsystem  udev subsystem (e.g., "usb", "block", "net").
 * @return Solid UDI prefix, or empty string if unmapped.
 */
static QString subsystemToUdiPrefix(const QString &subsystem)
{
    if (subsystem == QStringLiteral("usb"))
        return UDI_USB;
    if (subsystem == QStringLiteral("block"))
        return UDI_BLOCK;
    if (subsystem == QStringLiteral("net"))
        return UDI_NET;
    if (subsystem == QStringLiteral("input"))
        return UDI_INPUT;
    if (subsystem == QStringLiteral("drm"))
        return UDI_DISPLAY;
    if (subsystem == QStringLiteral("sound"))
        return UDI_AUDIO;
    return QString();
}

/**
 * Handle a udev DeviceAdded D-Bus signal.
 *
 * Creates a new Solid device entry for the newly attached device.
 *
 * @param devpath    Sysfs device path from udev.
 * @param subsystem  udev subsystem name.
 */
void VeridianManager::onUdevDeviceAdded(const QString &devpath,
                                         const QString &subsystem)
{
    QString prefix = subsystemToUdiPrefix(subsystem);
    if (prefix.isEmpty()) {
        qDebug("VeridianManager: ignoring udev add for unknown subsystem: %s",
               qUtf8Printable(subsystem));
        return;
    }

    /* Extract device name from devpath */
    QString devName = devpath.section(QLatin1Char('/'), -1);
    QString udi = prefix + QLatin1Char('/') + devName;

    if (m_deviceUdis.contains(udi)) {
        qDebug("VeridianManager: device already tracked: %s",
               qUtf8Printable(udi));
        return;
    }

    m_deviceUdis << udi;

    qDebug("VeridianManager: udev device added: %s (subsystem=%s)",
           qUtf8Printable(udi), qUtf8Printable(subsystem));

    Q_EMIT deviceAdded(udi);
}

/**
 * Handle a udev DeviceRemoved D-Bus signal.
 *
 * Removes the Solid device entry and cleans up.
 *
 * @param devpath    Sysfs device path from udev.
 * @param subsystem  udev subsystem name.
 */
void VeridianManager::onUdevDeviceRemoved(const QString &devpath,
                                           const QString &subsystem)
{
    QString prefix = subsystemToUdiPrefix(subsystem);
    if (prefix.isEmpty())
        return;

    QString devName = devpath.section(QLatin1Char('/'), -1);
    QString udi = prefix + QLatin1Char('/') + devName;

    if (!m_deviceUdis.contains(udi))
        return;

    m_deviceUdis.removeAll(udi);

    /* Clean up cached device object */
    delete m_devices.take(udi);

    qDebug("VeridianManager: udev device removed: %s (subsystem=%s)",
           qUtf8Printable(udi), qUtf8Printable(subsystem));

    Q_EMIT deviceRemoved(udi);
}

/**
 * Handle a udev DeviceChanged D-Bus signal.
 *
 * Updates device properties and re-creates the cached device object.
 *
 * @param devpath    Sysfs device path from udev.
 * @param subsystem  udev subsystem name.
 */
void VeridianManager::onUdevDeviceChanged(const QString &devpath,
                                           const QString &subsystem)
{
    QString prefix = subsystemToUdiPrefix(subsystem);
    if (prefix.isEmpty())
        return;

    QString devName = devpath.section(QLatin1Char('/'), -1);
    QString udi = prefix + QLatin1Char('/') + devName;

    if (!m_deviceUdis.contains(udi))
        return;

    /* Invalidate cached device so it gets re-created on next access */
    delete m_devices.take(udi);

    qDebug("VeridianManager: udev device changed: %s (subsystem=%s)",
           qUtf8Printable(udi), qUtf8Printable(subsystem));
}

/**
 * Enumerate USB devices.
 *
 * Scans /dev/bus/usb/ for attached USB devices and adds them
 * to the device list.
 */
void VeridianManager::enumerateUsbDevices()
{
    QDir usbDir(QStringLiteral("/dev/bus/usb"));
    if (!usbDir.exists())
        return;

    /* Scan bus directories (001, 002, ...) */
    const QStringList buses = usbDir.entryList(QDir::Dirs | QDir::NoDotAndDotDot);
    for (const QString &bus : buses) {
        QDir busDir(usbDir.filePath(bus));
        const QStringList devices = busDir.entryList(QDir::Files);
        for (const QString &dev : devices) {
            QString udi = UDI_USB + QLatin1Char('/') + bus +
                          QLatin1Char('-') + dev;
            if (!m_deviceUdis.contains(udi))
                m_deviceUdis << udi;
        }
    }
}

} /* namespace Veridian */
} /* namespace Backends */
} /* namespace Solid */

/*
 * VeridianOS -- powerdevil-veridian-backend.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * PowerDevil power management backend implementation for VeridianOS.
 *
 * Reads battery and power supply status from VeridianOS sysfs virtual
 * files, controls CPU frequency governors, manages DPMS screen power,
 * and handles suspend/hibernate via ACPI.
 */

#include "powerdevil-veridian-backend.h"

#include <QDebug>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QDBusConnection>
#include <QTextStream>

#include <unistd.h>

namespace PowerDevil {

/* ========================================================================= */
/* VeridianPowerBackend                                                      */
/* ========================================================================= */

VeridianPowerBackend::VeridianPowerBackend(QObject *parent)
    : QObject(parent)
    , m_powerSource(PowerSource::Unknown)
    , m_governor(CpuGovernor::Performance)
    , m_profile(BalancedProfile)
    , m_screenTimeout(300)  /* 5 minutes default */
    , m_idle(false)
    , m_idleTime(0)
    , m_pollTimer(new QTimer(this))
    , m_upowerInterface(nullptr)
{
    /* Probe system capabilities */
    probePowerSupply();
    probeBacklight();
    probeCpuFreq();
    probeSleepStates();

    connect(m_pollTimer, &QTimer::timeout,
            this, &VeridianPowerBackend::pollBatteryStatus);

    qDebug("PowerDevil/Veridian: initialized -- %d batteries, %d backlights",
           m_batteries.size(), m_backlights.size());
}

VeridianPowerBackend::~VeridianPowerBackend()
{
    stopPolling();
    delete m_upowerInterface;
}

/* ========================================================================= */
/* Power source                                                              */
/* ========================================================================= */

PowerSource VeridianPowerBackend::currentPowerSource() const
{
    return m_powerSource;
}

bool VeridianPowerBackend::isOnAC() const
{
    return m_powerSource == PowerSource::AC;
}

bool VeridianPowerBackend::hasBattery() const
{
    return !m_batteries.isEmpty();
}

int VeridianPowerBackend::batteryCount() const
{
    return m_batteries.size();
}

BatteryInfo VeridianPowerBackend::batteryInfo(int index) const
{
    if (index >= 0 && index < m_batteries.size())
        return m_batteries[index];
    return BatteryInfo{};
}

int VeridianPowerBackend::totalBatteryPercentage() const
{
    if (m_batteries.isEmpty())
        return -1;

    int totalEnergy = 0;
    int totalFull = 0;
    for (const auto &bat : m_batteries) {
        totalEnergy += bat.energyNow;
        totalFull += bat.energyFull;
    }

    if (totalFull <= 0)
        return 0;
    return (totalEnergy * 100) / totalFull;
}

ChargeState VeridianPowerBackend::chargeState() const
{
    if (m_batteries.isEmpty())
        return ChargeState::Unknown;
    return m_batteries.first().state;
}

int VeridianPowerBackend::timeToEmpty() const
{
    if (m_batteries.isEmpty())
        return -1;
    return m_batteries.first().timeToEmpty;
}

int VeridianPowerBackend::timeToFull() const
{
    if (m_batteries.isEmpty())
        return -1;
    return m_batteries.first().timeToFull;
}

/* ========================================================================= */
/* DPMS / Screen power                                                       */
/* ========================================================================= */

bool VeridianPowerBackend::setDpms(int level)
{
    /* DPMS control is delegated to KWin via D-Bus.
     * KWin's VeridianDrmOutput::setDpms() handles the actual DRM ioctl. */
    QDBusMessage msg = QDBusMessage::createMethodCall(
        QStringLiteral("org.kde.KWin"),
        QStringLiteral("/org/kde/KWin"),
        QStringLiteral("org.kde.KWin"),
        QStringLiteral("setDpms"));
    msg << level;
    QDBusConnection::sessionBus().call(msg);
    return true;
}

int VeridianPowerBackend::dpmsState() const
{
    /* Query KWin for current DPMS state */
    return 0;  /* DRM_MODE_DPMS_ON */
}

bool VeridianPowerBackend::setScreenTimeout(int seconds)
{
    m_screenTimeout = seconds;
    return true;
}

int VeridianPowerBackend::screenTimeout() const
{
    return m_screenTimeout;
}

/* ========================================================================= */
/* Backlight                                                                 */
/* ========================================================================= */

int VeridianPowerBackend::backlightCount() const
{
    return m_backlights.size();
}

BacklightInfo VeridianPowerBackend::backlightInfo(int index) const
{
    if (index >= 0 && index < m_backlights.size())
        return m_backlights[index];
    return BacklightInfo{};
}

bool VeridianPowerBackend::setBrightness(int index, int value)
{
    if (index < 0 || index >= m_backlights.size())
        return false;

    QString path = QStringLiteral("/sys/class/backlight/%1/brightness")
                       .arg(m_backlights[index].name);

    if (!writeSysfs(path, QString::number(value)))
        return false;

    m_backlights[index].brightness = value;
    Q_EMIT brightnessChanged(index, value);
    return true;
}

int VeridianPowerBackend::brightness(int index) const
{
    if (index >= 0 && index < m_backlights.size())
        return m_backlights[index].brightness;
    return -1;
}

int VeridianPowerBackend::maxBrightness(int index) const
{
    if (index >= 0 && index < m_backlights.size())
        return m_backlights[index].maxBrightness;
    return -1;
}

/* ========================================================================= */
/* CPU governor                                                              */
/* ========================================================================= */

CpuGovernor VeridianPowerBackend::cpuGovernor() const
{
    return m_governor;
}

bool VeridianPowerBackend::setCpuGovernor(CpuGovernor governor)
{
    QString govStr;
    switch (governor) {
    case CpuGovernor::Performance:   govStr = QStringLiteral("performance"); break;
    case CpuGovernor::Powersave:     govStr = QStringLiteral("powersave"); break;
    case CpuGovernor::Ondemand:      govStr = QStringLiteral("ondemand"); break;
    case CpuGovernor::Conservative:  govStr = QStringLiteral("conservative"); break;
    case CpuGovernor::Schedutil:     govStr = QStringLiteral("schedutil"); break;
    }

    /* Set governor for all CPUs */
    QDir cpuDir(QStringLiteral("/sys/devices/system/cpu"));
    QStringList cpuList = cpuDir.entryList(QStringList{QStringLiteral("cpu[0-9]*")},
                                           QDir::Dirs);

    bool success = true;
    for (const QString &cpu : cpuList) {
        QString path = QStringLiteral("/sys/devices/system/cpu/%1/cpufreq/scaling_governor")
                           .arg(cpu);
        if (!writeSysfs(path, govStr))
            success = false;
    }

    if (success) {
        m_governor = governor;
        Q_EMIT cpuGovernorChanged(governor);
    }

    return success;
}

QVector<CpuGovernor> VeridianPowerBackend::availableGovernors() const
{
    return m_availableGovernors;
}

int VeridianPowerBackend::cpuFrequency() const
{
    return readSysfsInt(
        QStringLiteral("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq"));
}

int VeridianPowerBackend::cpuFrequencyMin() const
{
    return readSysfsInt(
        QStringLiteral("/sys/devices/system/cpu/cpu0/cpufreq/scaling_min_freq"));
}

int VeridianPowerBackend::cpuFrequencyMax() const
{
    return readSysfsInt(
        QStringLiteral("/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq"));
}

/* ========================================================================= */
/* Suspend / Hibernate                                                       */
/* ========================================================================= */

QVector<SleepState> VeridianPowerBackend::supportedSleepStates() const
{
    return m_sleepStates;
}

bool VeridianPowerBackend::suspend()
{
    Q_EMIT aboutToSuspend();
    bool result = writeSysfs(QStringLiteral("/sys/power/state"),
                              QStringLiteral("mem"));
    if (result)
        Q_EMIT resumedFromSuspend();
    return result;
}

bool VeridianPowerBackend::hibernate()
{
    Q_EMIT aboutToSuspend();
    bool result = writeSysfs(QStringLiteral("/sys/power/state"),
                              QStringLiteral("disk"));
    if (result)
        Q_EMIT resumedFromSuspend();
    return result;
}

bool VeridianPowerBackend::hybridSuspend()
{
    /* Hybrid suspend: save to disk then suspend to RAM */
    writeSysfs(QStringLiteral("/sys/power/disk"), QStringLiteral("suspend"));
    return hibernate();
}

bool VeridianPowerBackend::canSuspend() const
{
    for (const auto &state : m_sleepStates) {
        if (state == SleepState::SuspendToRAM)
            return true;
    }
    return false;
}

bool VeridianPowerBackend::canHibernate() const
{
    for (const auto &state : m_sleepStates) {
        if (state == SleepState::SuspendToDisk)
            return true;
    }
    return false;
}

/* ========================================================================= */
/* Idle detection                                                            */
/* ========================================================================= */

bool VeridianPowerBackend::isIdle() const
{
    return m_idle;
}

int VeridianPowerBackend::idleTime() const
{
    return m_idleTime;
}

/* ========================================================================= */
/* Power profiles                                                            */
/* ========================================================================= */

VeridianPowerBackend::PowerProfile VeridianPowerBackend::currentProfile() const
{
    return m_profile;
}

bool VeridianPowerBackend::setProfile(PowerProfile profile)
{
    CpuGovernor targetGovernor;
    int targetTimeout;

    switch (profile) {
    case PerformanceProfile:
        targetGovernor = CpuGovernor::Performance;
        targetTimeout = 600;  /* 10 minutes */
        break;
    case BalancedProfile:
        targetGovernor = CpuGovernor::Schedutil;
        targetTimeout = 300;  /* 5 minutes */
        break;
    case PowerSaverProfile:
        targetGovernor = CpuGovernor::Powersave;
        targetTimeout = 120;  /* 2 minutes */
        break;
    }

    setCpuGovernor(targetGovernor);
    setScreenTimeout(targetTimeout);

    m_profile = profile;
    Q_EMIT profileChanged(profile);
    return true;
}

QVector<VeridianPowerBackend::PowerProfile>
VeridianPowerBackend::availableProfiles() const
{
    return QVector<PowerProfile>{
        PerformanceProfile, BalancedProfile, PowerSaverProfile};
}

/* ========================================================================= */
/* Polling                                                                   */
/* ========================================================================= */

void VeridianPowerBackend::startPolling(int intervalMs)
{
    m_pollTimer->start(intervalMs);
    qDebug("PowerDevil/Veridian: polling started (%d ms)", intervalMs);
}

void VeridianPowerBackend::stopPolling()
{
    m_pollTimer->stop();
}

void VeridianPowerBackend::pollBatteryStatus()
{
    PowerSource oldSource = m_powerSource;

    /* Re-read AC status */
    int acOnline = readSysfsInt(
        QStringLiteral("/sys/class/power_supply/AC0/online"));
    m_powerSource = (acOnline == 1) ? PowerSource::AC : PowerSource::Battery;

    if (m_powerSource != oldSource)
        Q_EMIT powerSourceChanged(m_powerSource);

    /* Re-read battery status */
    for (int i = 0; i < m_batteries.size(); ++i) {
        BatteryInfo &bat = m_batteries[i];
        QString base = QStringLiteral("/sys/class/power_supply/%1").arg(bat.name);

        bat.percentage = readSysfsInt(base + QStringLiteral("/capacity"));
        bat.energyNow = readSysfsInt(base + QStringLiteral("/energy_now"));
        bat.voltage = readSysfsInt(base + QStringLiteral("/voltage_now")) / 1000;
        bat.currentDraw = readSysfsInt(base + QStringLiteral("/current_now")) / 1000;

        QString statusStr = readSysfs(base + QStringLiteral("/status")).trimmed();
        if (statusStr == QStringLiteral("Charging"))
            bat.state = ChargeState::Charging;
        else if (statusStr == QStringLiteral("Discharging"))
            bat.state = ChargeState::Discharging;
        else if (statusStr == QStringLiteral("Full"))
            bat.state = ChargeState::Full;
        else if (statusStr == QStringLiteral("Not charging"))
            bat.state = ChargeState::NotCharging;
        else
            bat.state = ChargeState::Unknown;

        /* Estimate time remaining */
        if (bat.currentDraw > 0) {
            if (bat.state == ChargeState::Discharging)
                bat.timeToEmpty = (bat.energyNow * 3600) / (bat.currentDraw * bat.voltage / 1000);
            else if (bat.state == ChargeState::Charging)
                bat.timeToFull = ((bat.energyFull - bat.energyNow) * 3600) / (bat.currentDraw * bat.voltage / 1000);
        }

        Q_EMIT batteryChanged(i, bat);

        /* Low battery warnings */
        if (bat.percentage <= 5 && bat.state == ChargeState::Discharging)
            Q_EMIT criticalBattery(bat.percentage);
        else if (bat.percentage <= 15 && bat.state == ChargeState::Discharging)
            Q_EMIT lowBattery(bat.percentage);
    }
}

/* ========================================================================= */
/* Sysfs helpers                                                             */
/* ========================================================================= */

QString VeridianPowerBackend::readSysfs(const QString &path) const
{
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text))
        return QString();

    return QString::fromUtf8(file.readAll());
}

bool VeridianPowerBackend::writeSysfs(const QString &path, const QString &value)
{
    QFile file(path);
    if (!file.open(QIODevice::WriteOnly | QIODevice::Text)) {
        qWarning("PowerDevil/Veridian: cannot write %s: %s",
                 qPrintable(path), qPrintable(file.errorString()));
        return false;
    }

    QTextStream stream(&file);
    stream << value;
    return true;
}

int VeridianPowerBackend::readSysfsInt(const QString &path) const
{
    QString val = readSysfs(path).trimmed();
    bool ok;
    int result = val.toInt(&ok);
    return ok ? result : -1;
}

bool VeridianPowerBackend::probePowerSupply()
{
    /* Check AC adapter */
    int acOnline = readSysfsInt(
        QStringLiteral("/sys/class/power_supply/AC0/online"));
    m_powerSource = (acOnline == 1) ? PowerSource::AC
                    : (acOnline == 0) ? PowerSource::Battery
                    : PowerSource::Unknown;

    /* Enumerate batteries */
    QDir psDir(QStringLiteral("/sys/class/power_supply"));
    QStringList entries = psDir.entryList(QDir::Dirs | QDir::NoDotAndDotDot);

    for (const QString &entry : entries) {
        QString typePath = QStringLiteral("/sys/class/power_supply/%1/type").arg(entry);
        QString type = readSysfs(typePath).trimmed();

        if (type == QStringLiteral("Battery")) {
            BatteryInfo bat;
            bat.name = entry;
            bat.present = (readSysfsInt(
                QStringLiteral("/sys/class/power_supply/%1/present").arg(entry)) == 1);
            bat.percentage = readSysfsInt(
                QStringLiteral("/sys/class/power_supply/%1/capacity").arg(entry));
            bat.energyNow = readSysfsInt(
                QStringLiteral("/sys/class/power_supply/%1/energy_now").arg(entry));
            bat.energyFull = readSysfsInt(
                QStringLiteral("/sys/class/power_supply/%1/energy_full").arg(entry));
            bat.energyFullDesign = readSysfsInt(
                QStringLiteral("/sys/class/power_supply/%1/energy_full_design").arg(entry));
            bat.voltage = readSysfsInt(
                QStringLiteral("/sys/class/power_supply/%1/voltage_now").arg(entry)) / 1000;
            bat.currentDraw = readSysfsInt(
                QStringLiteral("/sys/class/power_supply/%1/current_now").arg(entry)) / 1000;
            bat.technology = readSysfs(
                QStringLiteral("/sys/class/power_supply/%1/technology").arg(entry)).trimmed();
            bat.cycleCount = readSysfsInt(
                QStringLiteral("/sys/class/power_supply/%1/cycle_count").arg(entry));
            bat.state = ChargeState::Unknown;
            bat.timeToEmpty = -1;
            bat.timeToFull = -1;

            m_batteries.append(bat);
        }
    }

    return true;
}

bool VeridianPowerBackend::probeBacklight()
{
    QDir blDir(QStringLiteral("/sys/class/backlight"));
    if (!blDir.exists())
        return false;

    QStringList entries = blDir.entryList(QDir::Dirs | QDir::NoDotAndDotDot);
    for (const QString &entry : entries) {
        BacklightInfo bl;
        bl.name = entry;
        bl.brightness = readSysfsInt(
            QStringLiteral("/sys/class/backlight/%1/brightness").arg(entry));
        bl.maxBrightness = readSysfsInt(
            QStringLiteral("/sys/class/backlight/%1/max_brightness").arg(entry));
        bl.minBrightness = 0;

        if (bl.maxBrightness > 0)
            m_backlights.append(bl);
    }

    return !m_backlights.isEmpty();
}

bool VeridianPowerBackend::probeCpuFreq()
{
    /* Read available governors */
    QString govPath = QStringLiteral(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors");
    QString govStr = readSysfs(govPath).trimmed();

    if (!govStr.isEmpty()) {
        QStringList govList = govStr.split(' ');
        for (const QString &g : govList) {
            if (g == QStringLiteral("performance"))
                m_availableGovernors.append(CpuGovernor::Performance);
            else if (g == QStringLiteral("powersave"))
                m_availableGovernors.append(CpuGovernor::Powersave);
            else if (g == QStringLiteral("ondemand"))
                m_availableGovernors.append(CpuGovernor::Ondemand);
            else if (g == QStringLiteral("conservative"))
                m_availableGovernors.append(CpuGovernor::Conservative);
            else if (g == QStringLiteral("schedutil"))
                m_availableGovernors.append(CpuGovernor::Schedutil);
        }
    }

    /* Read current governor */
    QString currentGov = readSysfs(QStringLiteral(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")).trimmed();

    if (currentGov == QStringLiteral("performance"))
        m_governor = CpuGovernor::Performance;
    else if (currentGov == QStringLiteral("powersave"))
        m_governor = CpuGovernor::Powersave;
    else if (currentGov == QStringLiteral("ondemand"))
        m_governor = CpuGovernor::Ondemand;
    else if (currentGov == QStringLiteral("conservative"))
        m_governor = CpuGovernor::Conservative;
    else if (currentGov == QStringLiteral("schedutil"))
        m_governor = CpuGovernor::Schedutil;

    return !m_availableGovernors.isEmpty();
}

bool VeridianPowerBackend::probeSleepStates()
{
    QString states = readSysfs(QStringLiteral("/sys/power/state")).trimmed();

    if (states.contains(QStringLiteral("mem")))
        m_sleepStates.append(SleepState::SuspendToRAM);
    if (states.contains(QStringLiteral("disk")))
        m_sleepStates.append(SleepState::SuspendToDisk);
    if (states.contains(QStringLiteral("freeze")))
        m_sleepStates.append(SleepState::HybridSuspend);

    return !m_sleepStates.isEmpty();
}

} /* namespace PowerDevil */

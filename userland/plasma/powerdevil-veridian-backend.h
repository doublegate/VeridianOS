/*
 * VeridianOS -- powerdevil-veridian-backend.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * PowerDevil power management backend for VeridianOS.  Provides the
 * interface between KDE's PowerDevil daemon and the VeridianOS kernel's
 * power management subsystems (ACPI, CPU frequency, DPMS).
 *
 * Responsibilities:
 *   - Battery status queries (AC adapter, battery level, charge state)
 *   - DPMS screen power control (on/standby/suspend/off)
 *   - CPU frequency governor management (performance/powersave/ondemand)
 *   - Suspend/hibernate operations (via ACPI S3/S4)
 *   - Backlight brightness control
 *   - Idle detection and power profile switching
 *   - UPower D-Bus interface emulation for Plasma integration
 *
 * On VeridianOS, power management uses the kernel's ACPI subsystem
 * (Sprint 5.5) with sysfs-style virtual files.
 */

#ifndef POWERDEVIL_VERIDIAN_BACKEND_H
#define POWERDEVIL_VERIDIAN_BACKEND_H

#include <QObject>
#include <QString>
#include <QTimer>
#include <QDBusInterface>
#include <QVector>

namespace PowerDevil {

/* ========================================================================= */
/* Power source enumeration                                                  */
/* ========================================================================= */

enum class PowerSource {
    AC,
    Battery,
    UPS,
    Unknown
};

enum class ChargeState {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown
};

enum class SleepState {
    SuspendToRAM,       /* ACPI S3 */
    SuspendToDisk,      /* ACPI S4 (hibernate) */
    HybridSuspend,      /* S3 + S4 */
    Shutdown            /* ACPI S5 */
};

enum class CpuGovernor {
    Performance,
    Powersave,
    Ondemand,
    Conservative,
    Schedutil
};

/* ========================================================================= */
/* BatteryInfo -- battery status data                                        */
/* ========================================================================= */

struct BatteryInfo
{
    QString name;               /* e.g. "BAT0" */
    int percentage;             /* 0-100 */
    ChargeState state;
    int energyNow;              /* mWh */
    int energyFull;             /* mWh */
    int energyFullDesign;       /* mWh */
    int voltage;                /* mV */
    int currentDraw;            /* mA */
    int timeToEmpty;            /* seconds, -1 if unknown */
    int timeToFull;             /* seconds, -1 if unknown */
    QString technology;         /* "Li-ion", "Li-poly", etc. */
    int cycleCount;
    bool present;
};

/* ========================================================================= */
/* BacklightInfo -- display brightness data                                  */
/* ========================================================================= */

struct BacklightInfo
{
    QString name;               /* e.g. "intel_backlight" */
    int brightness;             /* current level */
    int maxBrightness;          /* maximum level */
    int minBrightness;          /* minimum level (usually 0) */
};

/* ========================================================================= */
/* VeridianPowerBackend -- PowerDevil backend                                */
/* ========================================================================= */

/**
 * Power management backend for PowerDevil on VeridianOS.
 *
 * Reads power status from VeridianOS sysfs-style virtual files:
 *   /sys/class/power_supply/AC0/online
 *   /sys/class/power_supply/BAT0/{status,capacity,energy_now,...}
 *   /sys/class/backlight/*/brightness
 *   /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
 *   /sys/power/state
 *
 * Polls battery status periodically and emits signals on changes.
 * Provides a UPower-compatible D-Bus interface for Plasma's battery
 * monitor applet.
 */
class VeridianPowerBackend : public QObject
{
    Q_OBJECT

public:
    explicit VeridianPowerBackend(QObject *parent = nullptr);
    ~VeridianPowerBackend() override;

    /* ----- Power source ----- */
    PowerSource currentPowerSource() const;
    bool isOnAC() const;
    bool hasBattery() const;
    int batteryCount() const;
    BatteryInfo batteryInfo(int index = 0) const;
    int totalBatteryPercentage() const;
    ChargeState chargeState() const;
    int timeToEmpty() const;            /* seconds */
    int timeToFull() const;             /* seconds */

    /* ----- DPMS / Screen power ----- */
    bool setDpms(int level);            /* 0=on, 1=standby, 2=suspend, 3=off */
    int dpmsState() const;
    bool setScreenTimeout(int seconds); /* idle timeout before DPMS */
    int screenTimeout() const;

    /* ----- Backlight ----- */
    int backlightCount() const;
    BacklightInfo backlightInfo(int index = 0) const;
    bool setBrightness(int index, int value);
    int brightness(int index = 0) const;
    int maxBrightness(int index = 0) const;

    /* ----- CPU governor ----- */
    CpuGovernor cpuGovernor() const;
    bool setCpuGovernor(CpuGovernor governor);
    QVector<CpuGovernor> availableGovernors() const;
    int cpuFrequency() const;           /* kHz, current */
    int cpuFrequencyMin() const;        /* kHz */
    int cpuFrequencyMax() const;        /* kHz */

    /* ----- Suspend / Hibernate ----- */
    QVector<SleepState> supportedSleepStates() const;
    bool suspend();                     /* S3 */
    bool hibernate();                   /* S4 */
    bool hybridSuspend();               /* S3 + S4 */
    bool canSuspend() const;
    bool canHibernate() const;

    /* ----- Idle detection ----- */
    bool isIdle() const;
    int idleTime() const;               /* seconds since last input */

    /* ----- Power profiles ----- */
    enum PowerProfile {
        PerformanceProfile,
        BalancedProfile,
        PowerSaverProfile
    };
    PowerProfile currentProfile() const;
    bool setProfile(PowerProfile profile);
    QVector<PowerProfile> availableProfiles() const;

    /* ----- Polling control ----- */
    void startPolling(int intervalMs = 5000);
    void stopPolling();

Q_SIGNALS:
    void batteryChanged(int index, const BatteryInfo &info);
    void powerSourceChanged(PowerSource source);
    void brightnessChanged(int index, int value);
    void cpuGovernorChanged(CpuGovernor governor);
    void profileChanged(PowerProfile profile);
    void idleStateChanged(bool idle);
    void aboutToSuspend();
    void resumedFromSuspend();
    void lowBattery(int percentage);
    void criticalBattery(int percentage);

private Q_SLOTS:
    void pollBatteryStatus();

private:
    /* ----- Sysfs helpers ----- */
    QString readSysfs(const QString &path) const;
    bool writeSysfs(const QString &path, const QString &value);
    int readSysfsInt(const QString &path) const;
    bool probePowerSupply();
    bool probeBacklight();
    bool probeCpuFreq();
    bool probeSleepStates();

    /* ----- State ----- */
    QVector<BatteryInfo> m_batteries;
    QVector<BacklightInfo> m_backlights;
    PowerSource m_powerSource;
    CpuGovernor m_governor;
    PowerProfile m_profile;
    QVector<SleepState> m_sleepStates;
    QVector<CpuGovernor> m_availableGovernors;
    int m_screenTimeout;                /* seconds */
    bool m_idle;
    int m_idleTime;

    /* ----- Polling ----- */
    QTimer *m_pollTimer;

    /* ----- D-Bus ----- */
    QDBusInterface *m_upowerInterface;
};

} /* namespace PowerDevil */

#endif /* POWERDEVIL_VERIDIAN_BACKEND_H */

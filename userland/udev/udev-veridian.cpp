/*
 * VeridianOS -- udev-veridian.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * udev device event daemon implementation for VeridianOS.
 *
 * Monitors kernel device events via /dev/hotplug and inotify, applies
 * rules from /etc/udev/rules.d/, and publishes events on D-Bus for
 * KDE Solid integration.
 */

#include "udev-veridian.h"

#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDebug>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QList>
#include <QMap>
#include <QMutex>
#include <QSocketNotifier>
#include <QString>
#include <QTextStream>
#include <QTimer>

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <sys/inotify.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

/* ========================================================================= */
/* Internal types                                                            */
/* ========================================================================= */

/**
 * Device database entry: tracks a currently attached device and its
 * properties.
 */
struct DeviceEntry {
    QString devpath;
    QString devnode;
    QString subsystem;
    QMap<QString, QString> properties;
};

/**
 * Internal monitor state.
 */
struct UdevMonitor {
    QList<QString> filters;
    int pipeFd[2];          /* pipe for event notification */
    bool active;
};

/* ========================================================================= */
/* Daemon singleton state                                                    */
/* ========================================================================= */

static QMutex s_mutex;
static bool s_running = false;
static uint64_t s_seqnum = 0;
static QMap<QString, DeviceEntry> s_devices;
static QList<UdevRule> s_rules;
static QList<UdevMonitor *> s_monitors;

static int s_inotifyFd = -1;
static QSocketNotifier *s_inotifyNotifier = nullptr;
static QTimer *s_pollTimer = nullptr;

/* ========================================================================= */
/* Forward declarations                                                      */
/* ========================================================================= */

static void loadRules();
static void enumerateExistingDevices();
static void processInotifyEvents();
static void pollKernelHotplug();
static void emitEvent(const UdevEvent &event);
static void applyRules(const UdevEvent &event);
static void notifyMonitors(const UdevEvent &event);
static void sendDbusSignal(const UdevEvent &event);
static void addDeviceEntry(const QString &devpath, const QString &devnode,
                           const QString &subsystem,
                           const QMap<QString, QString> &props);
static void removeDeviceEntry(const QString &devpath);

/* ========================================================================= */
/* Daemon lifecycle                                                          */
/* ========================================================================= */

int udev_daemon_start(void)
{
    QMutexLocker lock(&s_mutex);

    if (s_running) {
        qWarning("udev-veridian: daemon already running");
        return -EALREADY;
    }

    qDebug("udev-veridian: starting device event daemon");

    /* Load rules from /etc/udev/rules.d/ */
    loadRules();

    /* Enumerate devices already present */
    enumerateExistingDevices();

    /* Set up inotify on /dev for device node creation/removal */
    s_inotifyFd = inotify_init1(IN_NONBLOCK | IN_CLOEXEC);
    if (s_inotifyFd >= 0) {
        inotify_add_watch(s_inotifyFd, "/dev",
                          IN_CREATE | IN_DELETE | IN_ATTRIB);

        if (QDir(QStringLiteral("/dev/input")).exists())
            inotify_add_watch(s_inotifyFd, "/dev/input",
                              IN_CREATE | IN_DELETE);

        if (QDir(QStringLiteral("/dev/snd")).exists())
            inotify_add_watch(s_inotifyFd, "/dev/snd",
                              IN_CREATE | IN_DELETE);

        if (QDir(QStringLiteral("/dev/dri")).exists())
            inotify_add_watch(s_inotifyFd, "/dev/dri",
                              IN_CREATE | IN_DELETE);

        s_inotifyNotifier = new QSocketNotifier(s_inotifyFd,
                                                 QSocketNotifier::Read);
        QObject::connect(s_inotifyNotifier, &QSocketNotifier::activated,
                         processInotifyEvents);
    } else {
        qWarning("udev-veridian: inotify_init1 failed: %s", strerror(errno));
    }

    /* Periodic poll for kernel hotplug events (USB, etc.) */
    s_pollTimer = new QTimer();
    s_pollTimer->setInterval(500);  /* 500 ms */
    QObject::connect(s_pollTimer, &QTimer::timeout, pollKernelHotplug);
    s_pollTimer->start();

    /* Register on D-Bus */
    QDBusConnection bus = QDBusConnection::systemBus();
    if (bus.isConnected()) {
        bus.registerService(QStringLiteral("org.freedesktop.UDev"));
        qDebug("udev-veridian: registered on D-Bus as org.freedesktop.UDev");
    } else {
        qWarning("udev-veridian: D-Bus system bus not available");
    }

    s_running = true;
    qDebug("udev-veridian: daemon started, monitoring %d existing devices, "
           "%d rules loaded",
           s_devices.size(), s_rules.size());

    return 0;
}

void udev_daemon_stop(void)
{
    QMutexLocker lock(&s_mutex);

    if (!s_running)
        return;

    qDebug("udev-veridian: stopping daemon");

    delete s_pollTimer;
    s_pollTimer = nullptr;

    delete s_inotifyNotifier;
    s_inotifyNotifier = nullptr;

    if (s_inotifyFd >= 0) {
        close(s_inotifyFd);
        s_inotifyFd = -1;
    }

    /* Clean up monitors */
    for (UdevMonitor *mon : s_monitors) {
        mon->active = false;
        close(mon->pipeFd[0]);
        close(mon->pipeFd[1]);
    }
    qDeleteAll(s_monitors);
    s_monitors.clear();

    s_devices.clear();
    s_rules.clear();
    s_seqnum = 0;
    s_running = false;

    QDBusConnection::systemBus().unregisterService(
        QStringLiteral("org.freedesktop.UDev"));

    qDebug("udev-veridian: daemon stopped");
}

/* ========================================================================= */
/* Monitor interface                                                         */
/* ========================================================================= */

UdevMonitor *udev_monitor_new(void)
{
    QMutexLocker lock(&s_mutex);

    auto *mon = new UdevMonitor();
    mon->active = true;
    mon->pipeFd[0] = -1;
    mon->pipeFd[1] = -1;

    if (pipe2(mon->pipeFd, O_NONBLOCK | O_CLOEXEC) < 0) {
        qWarning("udev-veridian: pipe2 failed: %s", strerror(errno));
        delete mon;
        return nullptr;
    }

    s_monitors.append(mon);
    return mon;
}

int udev_monitor_add_filter(UdevMonitor *monitor, const char *subsystem)
{
    if (!monitor || !subsystem)
        return -EINVAL;

    QMutexLocker lock(&s_mutex);
    monitor->filters.append(QString::fromUtf8(subsystem));
    return 0;
}

UdevEvent *udev_monitor_receive_event(UdevMonitor *monitor)
{
    if (!monitor || !monitor->active)
        return nullptr;

    /* Block-wait on the pipe for a notification byte */
    char byte;
    ssize_t n = read(monitor->pipeFd[0], &byte, 1);
    if (n <= 0)
        return nullptr;

    /* In a full implementation, the event data would be sent through
     * the pipe or a shared queue.  For now, return a placeholder. */
    auto *event = new UdevEvent();
    memset(event, 0, sizeof(*event));
    event->action = UDEV_ACTION_CHANGE;
    event->seqnum = s_seqnum;
    return event;
}

void udev_monitor_destroy(UdevMonitor *monitor)
{
    if (!monitor)
        return;

    QMutexLocker lock(&s_mutex);

    monitor->active = false;
    if (monitor->pipeFd[0] >= 0)
        close(monitor->pipeFd[0]);
    if (monitor->pipeFd[1] >= 0)
        close(monitor->pipeFd[1]);

    s_monitors.removeAll(monitor);
    delete monitor;
}

/* ========================================================================= */
/* Device enumeration                                                        */
/* ========================================================================= */

int udev_enumerate_devices(const char *subsystem, char results[][UDEV_MAX_PATH],
                           int max)
{
    QMutexLocker lock(&s_mutex);

    if (!subsystem || !results || max <= 0)
        return -EINVAL;

    QString sub = QString::fromUtf8(subsystem);
    int count = 0;

    for (auto it = s_devices.constBegin();
         it != s_devices.constEnd() && count < max; ++it) {
        if (it.value().subsystem == sub) {
            QByteArray path = it.key().toUtf8();
            int len = path.size();
            if (len >= UDEV_MAX_PATH)
                len = UDEV_MAX_PATH - 1;
            memcpy(results[count], path.constData(), len);
            results[count][len] = '\0';
            count++;
        }
    }

    return count;
}

const char *udev_device_get_property(const char *devpath, const char *key)
{
    QMutexLocker lock(&s_mutex);

    if (!devpath || !key)
        return nullptr;

    QString path = QString::fromUtf8(devpath);
    auto it = s_devices.constFind(path);
    if (it == s_devices.constEnd())
        return nullptr;

    QString k = QString::fromUtf8(key);
    auto propIt = it.value().properties.constFind(k);
    if (propIt == it.value().properties.constEnd())
        return nullptr;

    /* Return pointer to static thread-local buffer */
    static thread_local QByteArray s_propBuf;
    s_propBuf = propIt.value().toUtf8();
    return s_propBuf.constData();
}

void udev_event_free(UdevEvent *event)
{
    delete event;
}

/* ========================================================================= */
/* Rule loading                                                              */
/* ========================================================================= */

static void loadRules()
{
    s_rules.clear();

    QDir rulesDir(QStringLiteral("/etc/udev/rules.d"));
    if (!rulesDir.exists()) {
        qDebug("udev-veridian: /etc/udev/rules.d not found, no rules loaded");
        return;
    }

    QStringList entries = rulesDir.entryList(
        QStringList(QStringLiteral("*.rules")),
        QDir::Files, QDir::Name);

    for (const QString &filename : entries) {
        QFile file(rulesDir.filePath(filename));
        if (!file.open(QIODevice::ReadOnly | QIODevice::Text))
            continue;

        QTextStream stream(&file);
        while (!stream.atEnd()) {
            QString line = stream.readLine().trimmed();

            /* Skip comments and empty lines */
            if (line.isEmpty() || line.startsWith(QLatin1Char('#')))
                continue;

            /* Parse simple KEY==VALUE, KEY+=VALUE format */
            UdevRule rule;
            memset(&rule, 0, sizeof(rule));
            rule.num_matches = 0;

            QStringList parts = line.split(QLatin1Char(','));
            for (const QString &part : parts) {
                QString trimmed = part.trimmed();

                if (trimmed.contains(QStringLiteral("=="))) {
                    /* Match condition */
                    int pos = trimmed.indexOf(QStringLiteral("=="));
                    if (rule.num_matches < UDEV_MAX_MATCHES) {
                        QByteArray key = trimmed.left(pos).trimmed().toUtf8();
                        QByteArray val = trimmed.mid(pos + 2).trimmed()
                                             .remove(QLatin1Char('"')).toUtf8();
                        strncpy(rule.matches[rule.num_matches].key,
                                key.constData(), UDEV_MAX_NAME - 1);
                        strncpy(rule.matches[rule.num_matches].value,
                                val.constData(), UDEV_MAX_PATH - 1);
                        rule.num_matches++;
                    }
                } else if (trimmed.contains(QStringLiteral("+="))) {
                    /* Action */
                    int pos = trimmed.indexOf(QStringLiteral("+="));
                    QString actionKey = trimmed.left(pos).trimmed();
                    QByteArray actionVal = trimmed.mid(pos + 2).trimmed()
                                               .remove(QLatin1Char('"')).toUtf8();

                    if (actionKey == QStringLiteral("RUN")) {
                        rule.action_type = UDEV_RULE_RUN;
                    } else if (actionKey == QStringLiteral("SYMLINK")) {
                        rule.action_type = UDEV_RULE_SYMLINK;
                    } else if (actionKey.startsWith(QStringLiteral("ENV"))) {
                        rule.action_type = UDEV_RULE_ENV;
                    }

                    strncpy(rule.action_value, actionVal.constData(),
                            UDEV_MAX_PATH - 1);
                }
            }

            if (rule.num_matches > 0) {
                s_rules.append(rule);
            }
        }

        file.close();
    }

    qDebug("udev-veridian: loaded %d rules from %d files",
           s_rules.size(), entries.size());
}

/* ========================================================================= */
/* Device enumeration at startup                                             */
/* ========================================================================= */

static void enumerateExistingDevices()
{
    s_devices.clear();

    /* Scan /dev for block devices */
    QDir devDir(QStringLiteral("/dev"));
    if (devDir.exists()) {
        const QStringList blockFilters = {
            QStringLiteral("sd[a-z]*"),
            QStringLiteral("vd[a-z]*"),
            QStringLiteral("nvme*"),
        };
        for (const QString &filter : blockFilters) {
            const QStringList entries = devDir.entryList(
                QStringList(filter), QDir::System);
            for (const QString &entry : entries) {
                QString devpath = QStringLiteral("/sys/block/") + entry;
                QString devnode = QStringLiteral("/dev/") + entry;
                QMap<QString, QString> props;
                props[QStringLiteral("DEVNAME")] = devnode;
                props[QStringLiteral("SUBSYSTEM")] = QStringLiteral("block");
                addDeviceEntry(devpath, devnode, QStringLiteral("block"), props);
            }
        }
    }

    /* Scan /dev/input for input devices */
    QDir inputDir(QStringLiteral("/dev/input"));
    if (inputDir.exists()) {
        const QStringList entries = inputDir.entryList(
            QStringList(QStringLiteral("event*")), QDir::System);
        for (const QString &entry : entries) {
            QString devpath = QStringLiteral("/sys/class/input/") + entry;
            QString devnode = QStringLiteral("/dev/input/") + entry;
            QMap<QString, QString> props;
            props[QStringLiteral("DEVNAME")] = devnode;
            props[QStringLiteral("SUBSYSTEM")] = QStringLiteral("input");
            addDeviceEntry(devpath, devnode, QStringLiteral("input"), props);
        }
    }

    /* Scan /dev/dri for display devices */
    QDir driDir(QStringLiteral("/dev/dri"));
    if (driDir.exists()) {
        const QStringList entries = driDir.entryList(
            QStringList(QStringLiteral("card*")), QDir::System);
        for (const QString &entry : entries) {
            QString devpath = QStringLiteral("/sys/class/drm/") + entry;
            QString devnode = QStringLiteral("/dev/dri/") + entry;
            QMap<QString, QString> props;
            props[QStringLiteral("DEVNAME")] = devnode;
            props[QStringLiteral("SUBSYSTEM")] = QStringLiteral("drm");
            addDeviceEntry(devpath, devnode, QStringLiteral("drm"), props);
        }
    }

    /* Scan network interfaces via /proc/net/dev */
    QFile procNetDev(QStringLiteral("/proc/net/dev"));
    if (procNetDev.open(QIODevice::ReadOnly)) {
        QTextStream stream(&procNetDev);
        stream.readLine(); /* skip header */
        stream.readLine();
        while (!stream.atEnd()) {
            QString line = stream.readLine().trimmed();
            int colonPos = line.indexOf(QLatin1Char(':'));
            if (colonPos > 0) {
                QString ifName = line.left(colonPos).trimmed();
                QString devpath = QStringLiteral("/sys/class/net/") + ifName;
                QMap<QString, QString> props;
                props[QStringLiteral("INTERFACE")] = ifName;
                props[QStringLiteral("SUBSYSTEM")] = QStringLiteral("net");
                addDeviceEntry(devpath, QString(), QStringLiteral("net"), props);
            }
        }
        procNetDev.close();
    }
}

/* ========================================================================= */
/* Kernel event processing                                                   */
/* ========================================================================= */

static void processInotifyEvents()
{
    char buf[4096]
        __attribute__((aligned(__alignof__(struct inotify_event))));

    ssize_t len = read(s_inotifyFd, buf, sizeof(buf));
    if (len <= 0)
        return;

    const char *ptr = buf;
    while (ptr < buf + len) {
        const struct inotify_event *iev =
            reinterpret_cast<const struct inotify_event *>(ptr);

        if (iev->len > 0) {
            QString name = QString::fromUtf8(iev->name);
            QString devnode = QStringLiteral("/dev/") + name;
            QString devpath = QStringLiteral("/sys/dev/") + name;

            /* Determine subsystem from name patterns */
            QString subsystem;
            if (name.startsWith(QStringLiteral("sd")) ||
                name.startsWith(QStringLiteral("vd")) ||
                name.startsWith(QStringLiteral("nvme"))) {
                subsystem = QStringLiteral("block");
            } else if (name.startsWith(QStringLiteral("event"))) {
                subsystem = QStringLiteral("input");
            } else {
                subsystem = QStringLiteral("unknown");
            }

            UdevEvent event;
            memset(&event, 0, sizeof(event));
            s_seqnum++;
            event.seqnum = s_seqnum;

            QByteArray subBytes = subsystem.toUtf8();
            strncpy(event.subsystem, subBytes.constData(), UDEV_MAX_NAME - 1);
            QByteArray pathBytes = devpath.toUtf8();
            strncpy(event.devpath, pathBytes.constData(), UDEV_MAX_PATH - 1);
            QByteArray nodeBytes = devnode.toUtf8();
            strncpy(event.devnode, nodeBytes.constData(), UDEV_MAX_PATH - 1);
            event.num_properties = 0;

            if (iev->mask & IN_CREATE) {
                event.action = UDEV_ACTION_ADD;
                QMap<QString, QString> props;
                props[QStringLiteral("DEVNAME")] = devnode;
                props[QStringLiteral("SUBSYSTEM")] = subsystem;
                addDeviceEntry(devpath, devnode, subsystem, props);
            } else if (iev->mask & IN_DELETE) {
                event.action = UDEV_ACTION_REMOVE;
                removeDeviceEntry(devpath);
            } else {
                event.action = UDEV_ACTION_CHANGE;
            }

            emitEvent(event);
        }

        ptr += sizeof(struct inotify_event) + iev->len;
    }
}

/**
 * Poll kernel hotplug interface for USB and other device events.
 *
 * On VeridianOS, kernel USB hotplug events are exposed via
 * /dev/hotplug or a similar mechanism.  This polls that interface
 * periodically.
 */
static void pollKernelHotplug()
{
    /* Read from /dev/hotplug if available.  This is a VeridianOS-specific
     * interface backed by the kernel's UsbHotplugManager. */
    int fd = open("/dev/hotplug", O_RDONLY | O_NONBLOCK);
    if (fd < 0)
        return;

    /* Read event records (simplified binary format) */
    struct {
        uint8_t  type;       /* 0=attach, 1=detach */
        uint8_t  port;
        uint16_t vendor_id;
        uint16_t product_id;
        uint8_t  device_class;
        uint8_t  speed;
    } __attribute__((packed)) record;

    while (read(fd, &record, sizeof(record)) == sizeof(record)) {
        UdevEvent event;
        memset(&event, 0, sizeof(event));
        s_seqnum++;
        event.seqnum = s_seqnum;

        strncpy(event.subsystem, "usb", UDEV_MAX_NAME - 1);

        char pathBuf[UDEV_MAX_PATH];
        snprintf(pathBuf, sizeof(pathBuf), "/sys/bus/usb/devices/port%d",
                 record.port);
        strncpy(event.devpath, pathBuf, UDEV_MAX_PATH - 1);

        snprintf(pathBuf, sizeof(pathBuf), "/dev/bus/usb/001/%03d",
                 record.port);
        strncpy(event.devnode, pathBuf, UDEV_MAX_PATH - 1);

        /* Set properties */
        event.num_properties = 0;
        auto addProp = [&](const char *key, const char *val) {
            if (event.num_properties < UDEV_MAX_PROPS) {
                strncpy(event.properties[event.num_properties].key,
                        key, UDEV_MAX_NAME - 1);
                strncpy(event.properties[event.num_properties].value,
                        val, UDEV_MAX_PATH - 1);
                event.num_properties++;
            }
        };

        char valBuf[64];
        snprintf(valBuf, sizeof(valBuf), "%04x", record.vendor_id);
        addProp("ID_VENDOR_ID", valBuf);

        snprintf(valBuf, sizeof(valBuf), "%04x", record.product_id);
        addProp("ID_MODEL_ID", valBuf);

        snprintf(valBuf, sizeof(valBuf), "%02x", record.device_class);
        addProp("ID_USB_CLASS", valBuf);

        if (record.type == 0) {
            event.action = UDEV_ACTION_ADD;
            addProp("SUBSYSTEM", "usb");
            QMap<QString, QString> props;
            props[QStringLiteral("ID_VENDOR_ID")] =
                QString::fromUtf8(valBuf);
            props[QStringLiteral("SUBSYSTEM")] = QStringLiteral("usb");
            addDeviceEntry(QString::fromUtf8(event.devpath),
                           QString::fromUtf8(event.devnode),
                           QStringLiteral("usb"), props);
        } else {
            event.action = UDEV_ACTION_REMOVE;
            removeDeviceEntry(QString::fromUtf8(event.devpath));
        }

        emitEvent(event);
    }

    close(fd);
}

/* ========================================================================= */
/* Event processing pipeline                                                 */
/* ========================================================================= */

static void emitEvent(const UdevEvent &event)
{
    /* Apply rules */
    applyRules(event);

    /* Notify monitors */
    notifyMonitors(event);

    /* Send D-Bus signal */
    sendDbusSignal(event);

    const char *actionStr = "unknown";
    switch (event.action) {
    case UDEV_ACTION_ADD:    actionStr = "add";    break;
    case UDEV_ACTION_REMOVE: actionStr = "remove"; break;
    case UDEV_ACTION_CHANGE: actionStr = "change"; break;
    case UDEV_ACTION_BIND:   actionStr = "bind";   break;
    case UDEV_ACTION_UNBIND: actionStr = "unbind"; break;
    }

    qDebug("udev-veridian: [%llu] %s %s %s",
           static_cast<unsigned long long>(event.seqnum),
           actionStr, event.subsystem, event.devpath);
}

static void applyRules(const UdevEvent &event)
{
    for (const UdevRule &rule : s_rules) {
        bool match = true;

        for (int i = 0; i < rule.num_matches; i++) {
            const char *key = rule.matches[i].key;
            const char *expected = rule.matches[i].value;

            if (strcmp(key, "SUBSYSTEM") == 0) {
                if (strcmp(event.subsystem, expected) != 0)
                    match = false;
            } else if (strcmp(key, "ACTION") == 0) {
                const char *actionStr = "";
                switch (event.action) {
                case UDEV_ACTION_ADD:    actionStr = "add";    break;
                case UDEV_ACTION_REMOVE: actionStr = "remove"; break;
                case UDEV_ACTION_CHANGE: actionStr = "change"; break;
                default: break;
                }
                if (strcmp(actionStr, expected) != 0)
                    match = false;
            } else {
                /* Check device properties */
                bool found = false;
                for (int p = 0; p < event.num_properties; p++) {
                    if (strcmp(event.properties[p].key, key) == 0 &&
                        strcmp(event.properties[p].value, expected) == 0) {
                        found = true;
                        break;
                    }
                }
                if (!found)
                    match = false;
            }

            if (!match)
                break;
        }

        if (match) {
            qDebug("udev-veridian: rule matched, action=%d value=%s",
                   rule.action_type, rule.action_value);

            switch (rule.action_type) {
            case UDEV_RULE_RUN:
                /* Would fork+exec the program */
                qDebug("udev-veridian: RUN %s (stub)", rule.action_value);
                break;
            case UDEV_RULE_SYMLINK:
                /* Would create symlink in /dev */
                qDebug("udev-veridian: SYMLINK %s (stub)", rule.action_value);
                break;
            case UDEV_RULE_ENV:
            case UDEV_RULE_LABEL:
                /* Set environment or label */
                break;
            }
        }
    }
}

static void notifyMonitors(const UdevEvent &event)
{
    QString subsystem = QString::fromUtf8(event.subsystem);

    for (UdevMonitor *mon : s_monitors) {
        if (!mon->active)
            continue;

        /* Check filters */
        if (!mon->filters.isEmpty() && !mon->filters.contains(subsystem))
            continue;

        /* Wake the monitor via pipe */
        if (mon->pipeFd[1] >= 0) {
            char byte = 1;
            ssize_t n = write(mon->pipeFd[1], &byte, 1);
            (void)n;
        }
    }
}

static void sendDbusSignal(const UdevEvent &event)
{
    QDBusConnection bus = QDBusConnection::systemBus();
    if (!bus.isConnected())
        return;

    QString signalName;
    switch (event.action) {
    case UDEV_ACTION_ADD:
        signalName = QStringLiteral("DeviceAdded");
        break;
    case UDEV_ACTION_REMOVE:
        signalName = QStringLiteral("DeviceRemoved");
        break;
    default:
        signalName = QStringLiteral("DeviceChanged");
        break;
    }

    QDBusMessage msg = QDBusMessage::createSignal(
        QStringLiteral("/org/freedesktop/UDev"),
        QStringLiteral("org.freedesktop.UDev"),
        signalName);

    msg << QString::fromUtf8(event.devpath)
        << QString::fromUtf8(event.subsystem);

    bus.send(msg);
}

/* ========================================================================= */
/* Device database helpers                                                   */
/* ========================================================================= */

static void addDeviceEntry(const QString &devpath, const QString &devnode,
                           const QString &subsystem,
                           const QMap<QString, QString> &props)
{
    DeviceEntry entry;
    entry.devpath = devpath;
    entry.devnode = devnode;
    entry.subsystem = subsystem;
    entry.properties = props;
    s_devices.insert(devpath, entry);
}

static void removeDeviceEntry(const QString &devpath)
{
    s_devices.remove(devpath);
}

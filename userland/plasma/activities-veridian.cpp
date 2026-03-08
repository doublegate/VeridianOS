/*
 * VeridianOS -- activities-veridian.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * KDE Activities backend implementation for VeridianOS.
 *
 * Activities provide virtual workspace grouping.  Each activity has a
 * unique ID (timestamp-based), a display name, icon, and a set of
 * associated window IDs.
 *
 * Storage: activity configs are saved to /etc/veridian/activities/<id>.conf
 * as simple key=value files.
 *
 * D-Bus service: org.kde.ActivityManager
 *   - AddActivity(name) -> id
 *   - RemoveActivity(id)
 *   - SetCurrentActivity(id)
 *   - CurrentActivity() -> id
 *   - ListActivities(state) -> [id]
 */

#include "activities-veridian.h"

#include <QDebug>
#include <QDir>
#include <QFile>
#include <QTextStream>
#include <QDateTime>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QVector>
#include <QSet>
#include <QHash>

#include <string.h>
#include <stdlib.h>
#include <time.h>

namespace Activities {

/* ========================================================================= */
/* Configuration                                                             */
/* ========================================================================= */

static const int MAX_ACTIVITIES = 16;
static const char *CONFIG_DIR = "/etc/veridian/activities";
static const char *DEFAULT_ACTIVITY_NAME = "Default";
static const char *DEFAULT_ICON = "activities";

/* ========================================================================= */
/* Internal state                                                            */
/* ========================================================================= */

struct ActivityInternal {
    QString id;
    QString name;
    QString description;
    QString icon;
    ActivityState state;
    QSet<uint32_t> windows;
};

static QVector<ActivityInternal> s_activities;
static int s_currentIndex = -1;
static bool s_initialized = false;

/* ========================================================================= */
/* ID generation                                                             */
/* ========================================================================= */

static QString generateId()
{
    /* Timestamp-based UUID-style: "act-<timestamp>-<random>" */
    uint64_t ts = static_cast<uint64_t>(QDateTime::currentMSecsSinceEpoch());
    uint32_t rnd = static_cast<uint32_t>(rand()) & 0xFFFF;

    return QStringLiteral("act-%1-%2")
               .arg(ts, 0, 16)
               .arg(rnd, 4, 16, QLatin1Char('0'));
}

/* ========================================================================= */
/* Persistence                                                               */
/* ========================================================================= */

static void saveActivity(const ActivityInternal &act)
{
    QDir().mkpath(QString::fromUtf8(CONFIG_DIR));

    QString path = QStringLiteral("%1/%2.conf")
                       .arg(QString::fromUtf8(CONFIG_DIR))
                       .arg(act.id);

    QFile file(path);
    if (!file.open(QIODevice::WriteOnly | QIODevice::Text)) {
        qWarning("Activities: cannot save %s", qPrintable(path));
        return;
    }

    QTextStream out(&file);
    out << "id=" << act.id << "\n";
    out << "name=" << act.name << "\n";
    out << "description=" << act.description << "\n";
    out << "icon=" << act.icon << "\n";
}

static ActivityInternal loadActivity(const QString &filePath)
{
    ActivityInternal act;
    act.state = ACTIVITY_STOPPED;

    QFile file(filePath);
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text))
        return act;

    QTextStream in(&file);
    while (!in.atEnd()) {
        QString line = in.readLine().trimmed();
        if (line.isEmpty() || line.startsWith(QLatin1Char('#')))
            continue;

        int eqPos = line.indexOf(QLatin1Char('='));
        if (eqPos < 0)
            continue;

        QString key = line.left(eqPos);
        QString value = line.mid(eqPos + 1);

        if (key == QStringLiteral("id"))
            act.id = value;
        else if (key == QStringLiteral("name"))
            act.name = value;
        else if (key == QStringLiteral("description"))
            act.description = value;
        else if (key == QStringLiteral("icon"))
            act.icon = value;
    }

    return act;
}

static void deleteActivityFile(const QString &id)
{
    QString path = QStringLiteral("%1/%2.conf")
                       .arg(QString::fromUtf8(CONFIG_DIR))
                       .arg(id);
    QFile::remove(path);
}

/* ========================================================================= */
/* Lookup helpers                                                            */
/* ========================================================================= */

static int findById(const QString &id)
{
    for (int i = 0; i < s_activities.size(); ++i) {
        if (s_activities[i].id == id)
            return i;
    }
    return -1;
}

static void setString(char *dst, size_t dstSize, const QString &src)
{
    QByteArray utf8 = src.toUtf8();
    size_t len = static_cast<size_t>(utf8.size());
    if (len >= dstSize)
        len = dstSize - 1;
    memcpy(dst, utf8.constData(), len);
    dst[len] = '\0';
}

static void fillActivity(Activity *out, const ActivityInternal &act)
{
    memset(out, 0, sizeof(Activity));
    setString(out->id, sizeof(out->id), act.id);
    setString(out->name, sizeof(out->name), act.name);
    setString(out->description, sizeof(out->description), act.description);
    setString(out->icon, sizeof(out->icon), act.icon);
    out->state = act.state;
    out->is_current = (&act == &s_activities[s_currentIndex]) ? 1 : 0;
}

/* ========================================================================= */
/* Window visibility (D-Bus integration with KWin)                           */
/* ========================================================================= */

static void setWindowVisibility(uint32_t windowId, bool visible)
{
    /* Notify compositor to show/hide window via D-Bus.
     * KWin handles the actual surface visibility. */
    QDBusMessage msg = QDBusMessage::createMethodCall(
        QStringLiteral("org.kde.KWin"),
        QStringLiteral("/KWin"),
        QStringLiteral("org.kde.KWin"),
        visible ? QStringLiteral("showWindow")
                : QStringLiteral("hideWindow"));
    msg << static_cast<uint>(windowId);
    QDBusConnection::sessionBus().send(msg);
}

static void updateWindowVisibility()
{
    if (s_currentIndex < 0)
        return;

    const QSet<uint32_t> &currentWindows = s_activities[s_currentIndex].windows;

    /* Gather all windows across all activities */
    QSet<uint32_t> allWindows;
    for (const ActivityInternal &act : s_activities)
        allWindows.unite(act.windows);

    /* Show windows in current activity, hide others */
    for (uint32_t wid : allWindows) {
        bool show = currentWindows.contains(wid);
        setWindowVisibility(wid, show);
    }
}

/* ========================================================================= */
/* D-Bus registration                                                        */
/* ========================================================================= */

static bool registerDBus()
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    if (!bus.registerService(QStringLiteral("org.kde.ActivityManager"))) {
        qWarning("Activities: failed to register D-Bus service: %s",
                 qPrintable(bus.lastError().message()));
        return false;
    }

    qDebug("Activities: D-Bus service registered at org.kde.ActivityManager");
    return true;
}

} /* namespace Activities */

/* ========================================================================= */
/* C API implementation                                                      */
/* ========================================================================= */

extern "C" {

int activities_init(void)
{
    if (Activities::s_initialized)
        return 0;

    Activities::s_activities.clear();
    Activities::s_currentIndex = -1;

    srand(static_cast<unsigned int>(time(nullptr)));

    /* Load saved activities */
    QDir configDir(QString::fromUtf8(Activities::CONFIG_DIR));
    if (configDir.exists()) {
        QStringList filters = { QStringLiteral("*.conf") };
        QFileInfoList entries = configDir.entryInfoList(filters, QDir::Files);
        for (const QFileInfo &fi : entries) {
            Activities::ActivityInternal act =
                Activities::loadActivity(fi.absoluteFilePath());
            if (!act.id.isEmpty() && !act.name.isEmpty()) {
                act.state = ACTIVITY_RUNNING;
                Activities::s_activities.append(act);
            }
        }
    }

    /* Create default activity if none exist */
    if (Activities::s_activities.isEmpty()) {
        Activities::ActivityInternal def;
        def.id = Activities::generateId();
        def.name = QString::fromUtf8(Activities::DEFAULT_ACTIVITY_NAME);
        def.icon = QString::fromUtf8(Activities::DEFAULT_ICON);
        def.state = ACTIVITY_RUNNING;
        Activities::s_activities.append(def);
        Activities::saveActivity(def);

        qDebug("Activities: created default activity '%s'",
               qPrintable(def.id));
    }

    /* Set first activity as current */
    Activities::s_currentIndex = 0;
    Activities::s_initialized = true;

    /* Register D-Bus */
    Activities::registerDBus();

    qDebug("Activities: initialized with %d activity(ies)",
           Activities::s_activities.size());
    return 0;
}

void activities_destroy(void)
{
    if (!Activities::s_initialized)
        return;

    /* Save all activities */
    for (const Activities::ActivityInternal &act : Activities::s_activities)
        Activities::saveActivity(act);

    Activities::s_activities.clear();
    Activities::s_currentIndex = -1;
    Activities::s_initialized = false;

    QDBusConnection::sessionBus().unregisterService(
        QStringLiteral("org.kde.ActivityManager"));

    qDebug("Activities: destroyed");
}

const char *activities_create(const char *name,
                              const char *description,
                              const char *icon)
{
    if (!Activities::s_initialized || !name)
        return nullptr;

    if (Activities::s_activities.size() >= Activities::MAX_ACTIVITIES) {
        qWarning("Activities: maximum %d activities reached",
                 Activities::MAX_ACTIVITIES);
        return nullptr;
    }

    Activities::ActivityInternal act;
    act.id = Activities::generateId();
    act.name = QString::fromUtf8(name);
    act.description = description ? QString::fromUtf8(description) : QString();
    act.icon = (icon && icon[0]) ? QString::fromUtf8(icon)
                                 : QString::fromUtf8(Activities::DEFAULT_ICON);
    act.state = ACTIVITY_RUNNING;

    Activities::s_activities.append(act);
    Activities::saveActivity(act);

    qDebug("Activities: created '%s' (id=%s)",
           name, qPrintable(act.id));

    /* Return pointer to the stored id -- stable until vector reallocation */
    return Activities::s_activities.last().id.toUtf8().constData();
}

void activities_delete(const char *id)
{
    if (!Activities::s_initialized || !id)
        return;

    /* Cannot delete the last activity */
    if (Activities::s_activities.size() <= 1) {
        qWarning("Activities: cannot delete the last activity");
        return;
    }

    QString qid = QString::fromUtf8(id);
    int idx = Activities::findById(qid);
    if (idx < 0) {
        qWarning("Activities: activity '%s' not found", id);
        return;
    }

    /* Reassign windows to current activity */
    int currentIdx = Activities::s_currentIndex;
    if (idx == currentIdx) {
        /* Deleting current: switch to another first */
        currentIdx = (idx == 0) ? 1 : 0;
        Activities::s_currentIndex = currentIdx;
    }

    const QSet<uint32_t> &orphanWindows = Activities::s_activities[idx].windows;
    Activities::s_activities[currentIdx].windows.unite(orphanWindows);

    /* Remove config file */
    Activities::deleteActivityFile(qid);

    /* Remove from list */
    Activities::s_activities.removeAt(idx);

    /* Fix current index */
    if (Activities::s_currentIndex >= Activities::s_activities.size())
        Activities::s_currentIndex = Activities::s_activities.size() - 1;
    if (idx < Activities::s_currentIndex)
        Activities::s_currentIndex--;

    Activities::updateWindowVisibility();

    qDebug("Activities: deleted '%s'", id);
}

void activities_switch(const char *id)
{
    if (!Activities::s_initialized || !id)
        return;

    QString qid = QString::fromUtf8(id);
    int idx = Activities::findById(qid);
    if (idx < 0) {
        qWarning("Activities: activity '%s' not found for switch", id);
        return;
    }

    if (idx == Activities::s_currentIndex)
        return;  /* Already current */

    Activities::s_currentIndex = idx;
    Activities::s_activities[idx].state = ACTIVITY_RUNNING;

    /* Update window visibility */
    Activities::updateWindowVisibility();

    /* Notify via D-Bus */
    QDBusMessage signal = QDBusMessage::createSignal(
        QStringLiteral("/ActivityManager"),
        QStringLiteral("org.kde.ActivityManager"),
        QStringLiteral("CurrentActivityChanged"));
    signal << qid;
    QDBusConnection::sessionBus().send(signal);

    qDebug("Activities: switched to '%s' (%s)",
           qPrintable(Activities::s_activities[idx].name), id);
}

const char *activities_get_current(void)
{
    if (!Activities::s_initialized || Activities::s_currentIndex < 0)
        return nullptr;

    /* Return a stable pointer to the current activity's ID.
     * Thread safety note: callers must not modify activities
     * concurrently with this call. */
    static QByteArray s_currentIdCache;
    s_currentIdCache = Activities::s_activities[Activities::s_currentIndex]
                           .id.toUtf8();
    return s_currentIdCache.constData();
}

int activities_list(Activity *out, int max)
{
    if (!Activities::s_initialized || !out || max <= 0)
        return 0;

    int count = qMin(Activities::s_activities.size(), max);
    for (int i = 0; i < count; ++i)
        Activities::fillActivity(&out[i], Activities::s_activities[i]);

    return count;
}

void activities_set_name(const char *id, const char *name)
{
    if (!Activities::s_initialized || !id || !name)
        return;

    int idx = Activities::findById(QString::fromUtf8(id));
    if (idx < 0)
        return;

    Activities::s_activities[idx].name = QString::fromUtf8(name);
    Activities::saveActivity(Activities::s_activities[idx]);

    qDebug("Activities: renamed '%s' to '%s'", id, name);
}

void activities_set_icon(const char *id, const char *icon)
{
    if (!Activities::s_initialized || !id || !icon)
        return;

    int idx = Activities::findById(QString::fromUtf8(id));
    if (idx < 0)
        return;

    Activities::s_activities[idx].icon = QString::fromUtf8(icon);
    Activities::saveActivity(Activities::s_activities[idx]);

    qDebug("Activities: set icon for '%s' to '%s'", id, icon);
}

void activities_add_window(const char *id, uint32_t window_id)
{
    if (!Activities::s_initialized || !id)
        return;

    int idx = Activities::findById(QString::fromUtf8(id));
    if (idx < 0)
        return;

    Activities::s_activities[idx].windows.insert(window_id);
}

void activities_remove_window(const char *id, uint32_t window_id)
{
    if (!Activities::s_initialized || !id)
        return;

    int idx = Activities::findById(QString::fromUtf8(id));
    if (idx < 0)
        return;

    Activities::s_activities[idx].windows.remove(window_id);
}

int activities_get_windows(const char *id, uint32_t *out, int max)
{
    if (!Activities::s_initialized || !id || !out || max <= 0)
        return 0;

    int idx = Activities::findById(QString::fromUtf8(id));
    if (idx < 0)
        return 0;

    const QSet<uint32_t> &windows = Activities::s_activities[idx].windows;
    int count = 0;
    for (uint32_t wid : windows) {
        if (count >= max)
            break;
        out[count++] = wid;
    }

    return count;
}

} /* extern "C" */

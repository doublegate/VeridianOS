/*
 * VeridianOS -- qveridianeventdispatcher.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Event dispatcher for VeridianOS.  Uses epoll to multiplex the
 * Wayland display fd, timer fds, socket notifiers, and posted events
 * into a single event loop.
 */

#ifndef QVERIDIANEVENTDISPATCHER_H
#define QVERIDIANEVENTDISPATCHER_H

#include <QtCore/QAbstractEventDispatcher>
#include <QtCore/QList>
#include <QtCore/QHash>

QT_BEGIN_NAMESPACE

class QVeridianEventDispatcher : public QAbstractEventDispatcher
{
    Q_OBJECT

public:
    explicit QVeridianEventDispatcher(QObject *parent = nullptr);
    ~QVeridianEventDispatcher() override;

    bool processEvents(QEventLoop::ProcessEventsFlags flags) override;

    void registerSocketNotifier(QSocketNotifier *notifier) override;
    void unregisterSocketNotifier(QSocketNotifier *notifier) override;

    void registerTimer(int timerId, qint64 interval, Qt::TimerType timerType,
                       QObject *object) override;
    bool unregisterTimer(int timerId) override;
    bool unregisterTimers(QObject *object) override;
    QList<TimerInfo> registeredTimers(QObject *object) const override;
    int remainingTime(int timerId) override;

    void wakeUp() override;
    void interrupt() override;

private:
    struct TimerData {
        int     timerId;
        qint64  interval;
        Qt::TimerType timerType;
        QObject *object;
        int     timerFd;
    };

    void initEpoll();

    int  m_epollFd   = -1;
    int  m_wakeUpFd  = -1;  /* eventfd for cross-thread wakeup */
    bool m_interrupt  = false;

    QHash<int, TimerData>          m_timers;       /* timerId -> data */
    QHash<int, QSocketNotifier *>  m_socketNotifiers; /* fd -> notifier */
};

QT_END_NAMESPACE

#endif /* QVERIDIANEVENTDISPATCHER_H */

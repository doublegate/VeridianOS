/*
 * VeridianOS -- qveridianeventdispatcher.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * epoll-based event dispatcher.  Monitors the Wayland display fd for
 * protocol events, timerfd for QTimer expirations, eventfd for
 * cross-thread wakeups, and regular fds for QSocketNotifier.
 */

#include "qveridianeventdispatcher.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QSocketNotifier>

#include <sys/epoll.h>
#include <sys/timerfd.h>
#include <sys/eventfd.h>
#include <unistd.h>
#include <errno.h>
#include <time.h>

QT_BEGIN_NAMESPACE

/* Maximum events returned per epoll_wait call */
static const int MAX_EPOLL_EVENTS = 64;

/* ========================================================================= */
/* Construction / destruction                                                */
/* ========================================================================= */

QVeridianEventDispatcher::QVeridianEventDispatcher(QObject *parent)
    : QAbstractEventDispatcher(parent)
{
    initEpoll();
}

QVeridianEventDispatcher::~QVeridianEventDispatcher()
{
    /* Clean up timer fds */
    for (auto it = m_timers.begin(); it != m_timers.end(); ++it) {
        if (it->timerFd >= 0)
            close(it->timerFd);
    }

    if (m_wakeUpFd >= 0)
        close(m_wakeUpFd);
    if (m_epollFd >= 0)
        close(m_epollFd);
}

/* ========================================================================= */
/* Initialization                                                            */
/* ========================================================================= */

void QVeridianEventDispatcher::initEpoll()
{
    m_epollFd = epoll_create1(EPOLL_CLOEXEC);
    if (m_epollFd < 0)
        return;

    /* Create an eventfd for cross-thread wakeup */
    m_wakeUpFd = eventfd(0, EFD_NONBLOCK | EFD_CLOEXEC);
    if (m_wakeUpFd >= 0) {
        struct epoll_event ev = {};
        ev.events = EPOLLIN;
        ev.data.fd = m_wakeUpFd;
        epoll_ctl(m_epollFd, EPOLL_CTL_ADD, m_wakeUpFd, &ev);
    }
}

/* ========================================================================= */
/* Event processing                                                          */
/* ========================================================================= */

bool QVeridianEventDispatcher::processEvents(QEventLoop::ProcessEventsFlags flags)
{
    m_interrupt = false;

    /* Process queued events first */
    QCoreApplication::sendPostedEvents();

    if (m_interrupt)
        return true;

    /* Determine timeout: 0 for WaitForMoreEvents=false, -1 for blocking */
    int timeout = (flags & QEventLoop::WaitForMoreEvents) ? -1 : 0;

    struct epoll_event events[MAX_EPOLL_EVENTS];
    int nfds = epoll_wait(m_epollFd, events, MAX_EPOLL_EVENTS, timeout);

    bool hadEvents = (nfds > 0);

    for (int i = 0; i < nfds; ++i) {
        int fd = events[i].data.fd;

        if (fd == m_wakeUpFd) {
            /* Drain the eventfd */
            uint64_t val;
            read(m_wakeUpFd, &val, sizeof(val));
            continue;
        }

        /* Check if this is a timer fd */
        bool isTimer = false;
        for (auto it = m_timers.begin(); it != m_timers.end(); ++it) {
            if (it->timerFd == fd) {
                uint64_t expirations;
                read(fd, &expirations, sizeof(expirations));
                QTimerEvent te(it->timerId);
                QCoreApplication::sendEvent(it->object, &te);
                isTimer = true;
                break;
            }
        }

        /* Check socket notifiers */
        if (!isTimer) {
            auto notifierIt = m_socketNotifiers.find(fd);
            if (notifierIt != m_socketNotifiers.end()) {
                QEvent event(QEvent::SockAct);
                QCoreApplication::sendEvent(*notifierIt, &event);
            }
        }
    }

    /* Process any newly queued events */
    QCoreApplication::sendPostedEvents();

    return hadEvents;
}

/* ========================================================================= */
/* Socket notifiers                                                          */
/* ========================================================================= */

void QVeridianEventDispatcher::registerSocketNotifier(QSocketNotifier *notifier)
{
    if (!notifier || m_epollFd < 0)
        return;

    int fd = notifier->socket();
    uint32_t epollEvents = 0;

    switch (notifier->type()) {
    case QSocketNotifier::Read:
        epollEvents = EPOLLIN;
        break;
    case QSocketNotifier::Write:
        epollEvents = EPOLLOUT;
        break;
    case QSocketNotifier::Exception:
        epollEvents = EPOLLPRI;
        break;
    }

    struct epoll_event ev = {};
    ev.events = epollEvents;
    ev.data.fd = fd;
    epoll_ctl(m_epollFd, EPOLL_CTL_ADD, fd, &ev);

    m_socketNotifiers.insert(fd, notifier);
}

void QVeridianEventDispatcher::unregisterSocketNotifier(QSocketNotifier *notifier)
{
    if (!notifier || m_epollFd < 0)
        return;

    int fd = notifier->socket();
    epoll_ctl(m_epollFd, EPOLL_CTL_DEL, fd, nullptr);
    m_socketNotifiers.remove(fd);
}

/* ========================================================================= */
/* Timers                                                                    */
/* ========================================================================= */

void QVeridianEventDispatcher::registerTimer(int timerId, qint64 interval,
                                              Qt::TimerType timerType,
                                              QObject *object)
{
    TimerData data;
    data.timerId   = timerId;
    data.interval  = interval;
    data.timerType = timerType;
    data.object    = object;
    data.timerFd   = -1;

    /* Create a timerfd for this timer */
    data.timerFd = timerfd_create(CLOCK_MONOTONIC, TFD_NONBLOCK | TFD_CLOEXEC);
    if (data.timerFd >= 0) {
        struct itimerspec its = {};
        if (interval > 0) {
            its.it_value.tv_sec     = interval / 1000;
            its.it_value.tv_nsec    = (interval % 1000) * 1000000L;
            its.it_interval.tv_sec  = its.it_value.tv_sec;
            its.it_interval.tv_nsec = its.it_value.tv_nsec;
        } else {
            /* Zero interval = fire as soon as possible */
            its.it_value.tv_nsec = 1;
        }
        timerfd_settime(data.timerFd, 0, &its, nullptr);

        /* Add to epoll */
        struct epoll_event ev = {};
        ev.events = EPOLLIN;
        ev.data.fd = data.timerFd;
        epoll_ctl(m_epollFd, EPOLL_CTL_ADD, data.timerFd, &ev);
    }

    m_timers.insert(timerId, data);
}

bool QVeridianEventDispatcher::unregisterTimer(int timerId)
{
    auto it = m_timers.find(timerId);
    if (it == m_timers.end())
        return false;

    if (it->timerFd >= 0) {
        epoll_ctl(m_epollFd, EPOLL_CTL_DEL, it->timerFd, nullptr);
        close(it->timerFd);
    }
    m_timers.erase(it);
    return true;
}

bool QVeridianEventDispatcher::unregisterTimers(QObject *object)
{
    bool found = false;
    auto it = m_timers.begin();
    while (it != m_timers.end()) {
        if (it->object == object) {
            if (it->timerFd >= 0) {
                epoll_ctl(m_epollFd, EPOLL_CTL_DEL, it->timerFd, nullptr);
                close(it->timerFd);
            }
            it = m_timers.erase(it);
            found = true;
        } else {
            ++it;
        }
    }
    return found;
}

QList<QAbstractEventDispatcher::TimerInfo>
QVeridianEventDispatcher::registeredTimers(QObject *object) const
{
    QList<TimerInfo> result;
    for (auto it = m_timers.begin(); it != m_timers.end(); ++it) {
        if (it->object == object)
            result.append(TimerInfo(it->timerId, it->interval, it->timerType));
    }
    return result;
}

int QVeridianEventDispatcher::remainingTime(int timerId)
{
    auto it = m_timers.find(timerId);
    if (it == m_timers.end() || it->timerFd < 0)
        return -1;

    struct itimerspec its = {};
    timerfd_gettime(it->timerFd, &its);

    return static_cast<int>(its.it_value.tv_sec * 1000 +
                            its.it_value.tv_nsec / 1000000);
}

/* ========================================================================= */
/* Wakeup / interrupt                                                        */
/* ========================================================================= */

void QVeridianEventDispatcher::wakeUp()
{
    if (m_wakeUpFd >= 0) {
        uint64_t val = 1;
        write(m_wakeUpFd, &val, sizeof(val));
    }
}

void QVeridianEventDispatcher::interrupt()
{
    m_interrupt = true;
    wakeUp();
}

QT_END_NAMESPACE

#include "moc_qveridianeventdispatcher.cpp"

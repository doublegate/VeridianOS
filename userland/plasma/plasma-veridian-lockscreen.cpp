/*
 * VeridianOS -- plasma-veridian-lockscreen.cpp
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Lock screen implementation for KDE Plasma on VeridianOS.
 *
 * Implements session locking using ext-session-lock-v1 Wayland protocol,
 * password verification against the VeridianOS UserDatabase, and the
 * lock screen UI rendering.
 */

#include "plasma-veridian-lockscreen.h"

#include <QDebug>
#include <QFile>
#include <QDir>
#include <QCryptographicHash>
#include <QPainter>
#include <QPainterPath>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QTextOption>
#include <QFontMetrics>
#include <QGuiApplication>
#include <QScreen>

#include <unistd.h>
#include <pwd.h>
#include <string.h>
#include <time.h>

namespace Plasma {

/* ========================================================================= */
/* VeridianAuthenticator                                                     */
/* ========================================================================= */

VeridianAuthenticator::VeridianAuthenticator(QObject *parent)
    : QObject(parent)
    , m_userId(-1)
    , m_failedAttempts(0)
    , m_maxAttempts(5)
    , m_lockoutDuration(60)
{
    /* Determine current user */
    uid_t uid = getuid();
    struct passwd *pw = getpwuid(uid);
    if (pw) {
        m_currentUser = QString::fromUtf8(pw->pw_name);
        m_displayName = m_currentUser;
        m_userId = static_cast<int>(uid);

        /* Check for user avatar */
        m_avatarPath = QStringLiteral("/var/lib/AccountsService/icons/%1")
                           .arg(m_currentUser);
        if (!QFile::exists(m_avatarPath))
            m_avatarPath.clear();
    }
}

VeridianAuthenticator::~VeridianAuthenticator()
{
}

bool VeridianAuthenticator::authenticate(const QString &username,
                                          const QString &password)
{
    /* Check lockout */
    if (isLockedOut()) {
        qWarning("LockScreen: authentication blocked -- locked out for %d more seconds",
                 lockoutRemainingSeconds());
        return false;
    }

    /* Read shadow entry for the user */
    QString storedHash;
    QByteArray salt;
    if (!readShadowEntry(username, storedHash, salt)) {
        qWarning("LockScreen: failed to read shadow entry for %s",
                 qPrintable(username));
        ++m_failedAttempts;
        Q_EMIT authenticationFailed(m_failedAttempts);
        return false;
    }

    /* Verify password */
    if (!verifyPasswordHash(password, storedHash, salt)) {
        ++m_failedAttempts;
        Q_EMIT authenticationFailed(m_failedAttempts);

        /* Lockout after too many failures */
        if (m_maxAttempts > 0 && m_failedAttempts >= m_maxAttempts) {
            m_lockoutUntil = QDateTime::currentDateTime().addSecs(m_lockoutDuration);
            Q_EMIT lockedOut(m_lockoutDuration);
        }

        return false;
    }

    /* Success */
    m_failedAttempts = 0;
    Q_EMIT authenticationSucceeded();
    return true;
}

bool VeridianAuthenticator::isLocked() const
{
    return false; /* This is about the authenticator, not the lock screen */
}

int VeridianAuthenticator::failedAttempts() const
{
    return m_failedAttempts;
}

void VeridianAuthenticator::resetFailedAttempts()
{
    m_failedAttempts = 0;
    m_lockoutUntil = QDateTime();
}

QString VeridianAuthenticator::currentUser() const { return m_currentUser; }
QString VeridianAuthenticator::userDisplayName() const { return m_displayName; }
QString VeridianAuthenticator::userAvatarPath() const { return m_avatarPath; }
int VeridianAuthenticator::userId() const { return m_userId; }

bool VeridianAuthenticator::isLockedOut() const
{
    if (!m_lockoutUntil.isValid())
        return false;
    return QDateTime::currentDateTime() < m_lockoutUntil;
}

int VeridianAuthenticator::lockoutRemainingSeconds() const
{
    if (!isLockedOut())
        return 0;
    return static_cast<int>(QDateTime::currentDateTime().secsTo(m_lockoutUntil));
}

bool VeridianAuthenticator::verifyPasswordHash(const QString &password,
                                                const QString &storedHash,
                                                const QByteArray &salt) const
{
    /* Compute SHA-256 hash of (salt + password) to match VeridianOS
     * UserDatabase format (Hash256 + salt, v0.20.2). */
    QCryptographicHash hasher(QCryptographicHash::Sha256);
    hasher.addData(salt);
    hasher.addData(password.toUtf8());
    QByteArray computed = hasher.result().toHex();

    /* Constant-time comparison to prevent timing attacks */
    QByteArray stored = storedHash.toUtf8();
    if (computed.size() != stored.size())
        return false;

    volatile int result = 0;
    for (int i = 0; i < computed.size(); ++i)
        result |= (computed[i] ^ stored[i]);

    return (result == 0);
}

bool VeridianAuthenticator::readShadowEntry(const QString &username,
                                             QString &hash,
                                             QByteArray &salt) const
{
    /* Read /etc/shadow for the user's password hash.
     * Format: username:$type$salt$hash:last:min:max:warn:inactive:expire: */
    QFile shadowFile(QStringLiteral("/etc/shadow"));
    if (!shadowFile.open(QIODevice::ReadOnly | QIODevice::Text))
        return false;

    while (!shadowFile.atEnd()) {
        QByteArray line = shadowFile.readLine().trimmed();
        if (line.isEmpty() || line.startsWith('#'))
            continue;

        int colonPos = line.indexOf(':');
        if (colonPos < 0)
            continue;

        QString user = QString::fromUtf8(line.left(colonPos));
        if (user != username)
            continue;

        /* Extract hash field (between first and second colon) */
        int secondColon = line.indexOf(':', colonPos + 1);
        if (secondColon < 0)
            secondColon = line.size();

        QByteArray hashField = line.mid(colonPos + 1, secondColon - colonPos - 1);

        /* Parse hash format: $type$salt$hash or salt:hash */
        if (hashField.startsWith('$')) {
            /* Crypt-style: $type$salt$hash */
            QList<QByteArray> parts = hashField.split('$');
            if (parts.size() >= 4) {
                salt = parts[2];
                hash = QString::fromUtf8(parts[3]);
                return true;
            }
        } else if (hashField.contains(':')) {
            /* VeridianOS format: salt:hash */
            int sepPos = hashField.indexOf(':');
            salt = hashField.left(sepPos);
            hash = QString::fromUtf8(hashField.mid(sepPos + 1));
            return true;
        } else {
            /* Plain hash, no salt */
            salt.clear();
            hash = QString::fromUtf8(hashField);
            return true;
        }
    }

    return false;
}

/* ========================================================================= */
/* VeridianLockSurface                                                       */
/* ========================================================================= */

VeridianLockSurface::VeridianLockSurface(struct wl_surface *surface,
                                          struct ext_session_lock_surface_v1 *lockSurface,
                                          int width, int height,
                                          const LockScreenConfig &config,
                                          QObject *parent)
    : QObject(parent)
    , m_wlSurface(surface)
    , m_lockSurface(lockSurface)
    , m_width(width)
    , m_height(height)
    , m_config(config)
    , m_statusIsError(false)
{
}

VeridianLockSurface::~VeridianLockSurface()
{
}

void VeridianLockSurface::setUserInfo(const QString &userName,
                                       const QString &avatarPath)
{
    m_userName = userName;
    m_avatarPath = avatarPath;

    if (!avatarPath.isEmpty())
        m_avatarImage = QImage(avatarPath);
}

void VeridianLockSurface::setPasswordText(const QString &masked)
{
    m_passwordMasked = masked;
}

void VeridianLockSurface::setStatusMessage(const QString &message, bool isError)
{
    m_statusMessage = message;
    m_statusIsError = isError;
}

void VeridianLockSurface::updateClock()
{
    Q_EMIT needsRedraw();
}

void VeridianLockSurface::resize(int width, int height)
{
    m_width = width;
    m_height = height;
    Q_EMIT needsRedraw();
}

void VeridianLockSurface::render()
{
    QImage image(m_width, m_height, QImage::Format_ARGB32_Premultiplied);

    paintBackground(image);
    paintClock(image);
    paintUserInfo(image);
    paintPasswordField(image);
    paintStatusMessage(image);

    submitBuffer(image);
}

void VeridianLockSurface::paintBackground(QImage &image)
{
    /* Try wallpaper image first, then solid color */
    if (!m_wallpaperImage.isNull()) {
        QPainter painter(&image);
        QImage scaled = m_wallpaperImage.scaled(m_width, m_height,
                                                 Qt::KeepAspectRatioByExpanding,
                                                 Qt::SmoothTransformation);
        int xOffset = (scaled.width() - m_width) / 2;
        int yOffset = (scaled.height() - m_height) / 2;
        painter.drawImage(-xOffset, -yOffset, scaled);

        /* Semi-transparent dark overlay for readability */
        painter.fillRect(image.rect(), QColor(0, 0, 0, 128));
    } else if (!m_config.wallpaperPath.isEmpty() && m_wallpaperImage.isNull()) {
        m_wallpaperImage = QImage(m_config.wallpaperPath);
        if (!m_wallpaperImage.isNull()) {
            paintBackground(image);
            return;
        }
        image.fill(m_config.backgroundColor);
    } else {
        image.fill(m_config.backgroundColor);
    }
}

void VeridianLockSurface::paintClock(QImage &image)
{
    if (!m_config.showClock)
        return;

    QPainter painter(&image);
    painter.setRenderHint(QPainter::TextAntialiasing, true);

    /* Clock */
    QDateTime now = QDateTime::currentDateTime();
    QString timeStr = now.toString(m_config.clockFormat);
    QString dateStr = now.toString(m_config.dateFormat);

    /* Time -- large, centered, upper third */
    painter.setFont(m_config.clockFont);
    painter.setPen(m_config.clockColor);
    QRectF clockRect(0, m_height * 0.15, m_width, m_height * 0.15);
    painter.drawText(clockRect, Qt::AlignHCenter | Qt::AlignBottom, timeStr);

    /* Date -- below clock */
    if (m_config.showDate) {
        painter.setFont(m_config.dateFont);
        painter.setPen(m_config.dateColor);
        QRectF dateRect(0, clockRect.bottom() + 8, m_width, 40);
        painter.drawText(dateRect, Qt::AlignHCenter | Qt::AlignTop, dateStr);
    }
}

void VeridianLockSurface::paintUserInfo(QImage &image)
{
    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing, true);
    painter.setRenderHint(QPainter::TextAntialiasing, true);

    qreal centerY = m_height * 0.45;

    /* User avatar (circular clip) */
    if (m_config.showUserAvatar && !m_avatarImage.isNull()) {
        int avatarSize = 80;
        QRectF avatarRect((m_width - avatarSize) / 2.0,
                          centerY - avatarSize - 10,
                          avatarSize, avatarSize);

        QPainterPath clipPath;
        clipPath.addEllipse(avatarRect);
        painter.setClipPath(clipPath);
        painter.drawImage(avatarRect, m_avatarImage);
        painter.setClipping(false);

        /* Avatar border */
        painter.setPen(QPen(QColor(255, 255, 255, 180), 2.0));
        painter.setBrush(Qt::NoBrush);
        painter.drawEllipse(avatarRect);
    }

    /* User name */
    if (m_config.showUserName && !m_userName.isEmpty()) {
        painter.setFont(m_config.userNameFont);
        painter.setPen(m_config.userNameColor);
        QRectF nameRect(0, centerY, m_width, 30);
        painter.drawText(nameRect, Qt::AlignHCenter | Qt::AlignTop, m_userName);
    }
}

void VeridianLockSurface::paintPasswordField(QImage &image)
{
    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing, true);
    painter.setRenderHint(QPainter::TextAntialiasing, true);

    int fieldW = m_config.inputFieldWidth;
    int fieldH = m_config.inputFieldHeight;
    int fieldX = (m_width - fieldW) / 2;
    int fieldY = static_cast<int>(m_height * 0.55);

    QRectF fieldRect(fieldX, fieldY, fieldW, fieldH);

    /* Field background */
    painter.setPen(QPen(m_config.inputFieldBorder, 1.5));
    painter.setBrush(m_config.inputFieldBackground);
    painter.drawRoundedRect(fieldRect, m_config.inputFieldRadius,
                             m_config.inputFieldRadius);

    /* Password text or placeholder */
    painter.setFont(m_config.passwordFont);
    QRectF textRect = fieldRect.adjusted(12, 0, -12, 0);

    if (m_passwordMasked.isEmpty()) {
        painter.setPen(m_config.inputFieldPlaceholder);
        painter.drawText(textRect, Qt::AlignVCenter | Qt::AlignLeft,
                         QStringLiteral("Enter password..."));
    } else {
        painter.setPen(m_config.inputFieldText);
        painter.drawText(textRect, Qt::AlignVCenter | Qt::AlignLeft,
                         m_passwordMasked);
    }
}

void VeridianLockSurface::paintStatusMessage(QImage &image)
{
    if (m_statusMessage.isEmpty())
        return;

    QPainter painter(&image);
    painter.setRenderHint(QPainter::TextAntialiasing, true);

    painter.setFont(m_config.messageFont);
    painter.setPen(m_statusIsError ? m_config.errorColor : m_config.clockColor);

    int msgY = static_cast<int>(m_height * 0.55) + m_config.inputFieldHeight + 16;
    QRectF msgRect(0, msgY, m_width, 30);
    painter.drawText(msgRect, Qt::AlignHCenter | Qt::AlignTop, m_statusMessage);
}

void VeridianLockSurface::submitBuffer(const QImage &image)
{
    /* In a full implementation, this would:
     * 1. Create a wl_shm_pool from a shared memory fd
     * 2. Create a wl_buffer from the pool
     * 3. Copy the QImage data into the shared memory
     * 4. wl_surface_attach(m_wlSurface, buffer, 0, 0)
     * 5. wl_surface_damage_buffer(m_wlSurface, 0, 0, m_width, m_height)
     * 6. wl_surface_commit(m_wlSurface)
     *
     * The VeridianOS Wayland shim (Sprint 9.1) provides these interfaces. */
    Q_UNUSED(image);
}

/* ========================================================================= */
/* VeridianLockScreen                                                        */
/* ========================================================================= */

VeridianLockScreen::VeridianLockScreen(QObject *parent)
    : QObject(parent)
    , m_locked(false)
    , m_visible(false)
    , m_authenticator(new VeridianAuthenticator(this))
    , m_idleTimer(new QTimer(this))
    , m_clockTimer(new QTimer(this))
    , m_wlDisplay(nullptr)
    , m_lockManager(nullptr)
    , m_sessionLock(nullptr)
{
    loadConfig();

    connect(m_authenticator, &VeridianAuthenticator::authenticationSucceeded,
            this, &VeridianLockScreen::onAuthSuccess);
    connect(m_authenticator, &VeridianAuthenticator::authenticationFailed,
            this, &VeridianLockScreen::onAuthFailed);

    connect(m_idleTimer, &QTimer::timeout,
            this, &VeridianLockScreen::onIdleTimeout);
    connect(m_clockTimer, &QTimer::timeout,
            this, &VeridianLockScreen::onClockTick);

    /* Register D-Bus interface */
    registerDBus();
}

VeridianLockScreen::~VeridianLockScreen()
{
    if (m_locked)
        releaseSessionLock();
    destroyLockSurfaces();
}

void VeridianLockScreen::loadConfig()
{
    m_config.wallpaperPath.clear();
    m_config.backgroundColor = QColor(26, 26, 46);  /* Dark navy */
    m_config.showClock = true;
    m_config.showDate = true;
    m_config.showUserAvatar = true;
    m_config.showUserName = true;
    m_config.clockFormat = QStringLiteral("HH:mm");
    m_config.dateFormat = QStringLiteral("dddd, MMMM d");

    /* Fonts */
    m_config.clockFont = QFont(QStringLiteral("Noto Sans"), 64);
    m_config.clockFont.setWeight(QFont::Thin);
    m_config.dateFont = QFont(QStringLiteral("Noto Sans"), 16);
    m_config.userNameFont = QFont(QStringLiteral("Noto Sans"), 14);
    m_config.userNameFont.setWeight(QFont::DemiBold);
    m_config.passwordFont = QFont(QStringLiteral("Noto Sans"), 12);
    m_config.messageFont = QFont(QStringLiteral("Noto Sans"), 11);

    /* Colors */
    m_config.clockColor = QColor(239, 240, 241);
    m_config.dateColor = QColor(189, 195, 199);
    m_config.userNameColor = QColor(239, 240, 241);
    m_config.inputFieldBackground = QColor(255, 255, 255, 30);
    m_config.inputFieldBorder = QColor(255, 255, 255, 80);
    m_config.inputFieldText = QColor(239, 240, 241);
    m_config.inputFieldPlaceholder = QColor(127, 140, 141);
    m_config.errorColor = QColor(218, 68, 83);

    /* Behavior */
    m_config.lockAfterIdleSeconds = 300;     /* 5 minutes */
    m_config.gracePeriodSeconds = 5;
    m_config.lockOnSuspend = true;
    m_config.lockOnSwitchUser = true;
    m_config.maxPasswordAttempts = 5;
    m_config.lockoutDurationSeconds = 60;

    /* Input field */
    m_config.inputFieldWidth = 300;
    m_config.inputFieldHeight = 40;
    m_config.inputFieldRadius = 20;
}

bool VeridianLockScreen::lock()
{
    if (m_locked)
        return true;

    /* Check grace period */
    if (m_lastUnlockTime.isValid()) {
        int elapsed = static_cast<int>(m_lastUnlockTime.secsTo(
            QDateTime::currentDateTime()));
        if (elapsed < m_config.gracePeriodSeconds) {
            qDebug("LockScreen: within grace period (%d/%d s), skipping lock",
                   elapsed, m_config.gracePeriodSeconds);
            return false;
        }
    }

    if (!acquireSessionLock()) {
        qWarning("LockScreen: failed to acquire session lock");
        return false;
    }

    createLockSurfaces();

    m_locked = true;
    m_visible = true;
    m_authenticator->resetFailedAttempts();
    clearPassword();

    /* Start clock update timer */
    m_clockTimer->start(1000);  /* 1 second */

    Q_EMIT locked();
    Q_EMIT lockStateChanged(true);

    qDebug("LockScreen: session locked");
    return true;
}

bool VeridianLockScreen::unlock(const QString &password)
{
    if (!m_locked)
        return true;

    return m_authenticator->authenticate(
        m_authenticator->currentUser(), password);
}

bool VeridianLockScreen::isLocked() const
{
    return m_locked;
}

bool VeridianLockScreen::isVisible() const
{
    return m_visible;
}

void VeridianLockScreen::setIdleLockEnabled(bool enabled)
{
    if (enabled && m_config.lockAfterIdleSeconds > 0)
        m_idleTimer->start(m_config.lockAfterIdleSeconds * 1000);
    else
        m_idleTimer->stop();
}

bool VeridianLockScreen::isIdleLockEnabled() const
{
    return m_idleTimer->isActive();
}

void VeridianLockScreen::resetIdleTimer()
{
    if (m_idleTimer->isActive())
        m_idleTimer->start();  /* restart with same interval */
}

bool VeridianLockScreen::registerDBus()
{
    QDBusConnection bus = QDBusConnection::sessionBus();
    return bus.registerService(QStringLiteral("org.kde.screensaver")) &&
           bus.registerObject(QStringLiteral("/ScreenSaver"), this);
}

void VeridianLockScreen::handleKeyPress(uint32_t key, const QString &text)
{
    Q_UNUSED(key);

    if (!m_locked)
        return;

    if (key == 0x0D || key == 0xFF0D) {  /* Return/Enter */
        submitPassword();
    } else if (key == 0x08 || key == 0xFF08) {  /* Backspace */
        deletePasswordChar();
    } else if (key == 0x1B || key == 0xFF1B) {  /* Escape */
        clearPassword();
    } else if (!text.isEmpty() && text[0].isPrint()) {
        appendPasswordChar(text);
    }
}

void VeridianLockScreen::handleKeyRelease(uint32_t key)
{
    Q_UNUSED(key);
}

/* ========================================================================= */
/* Internal helpers                                                          */
/* ========================================================================= */

void VeridianLockScreen::appendPasswordChar(const QString &ch)
{
    m_passwordBuffer += ch;
    updatePasswordDisplay();
}

void VeridianLockScreen::deletePasswordChar()
{
    if (!m_passwordBuffer.isEmpty()) {
        m_passwordBuffer.chop(1);
        updatePasswordDisplay();
    }
}

void VeridianLockScreen::submitPassword()
{
    if (m_passwordBuffer.isEmpty())
        return;

    /* Show "Unlocking..." status */
    for (auto *surface : m_surfaces) {
        surface->setStatusMessage(QStringLiteral("Unlocking..."), false);
        surface->render();
    }

    unlock(m_passwordBuffer);
}

void VeridianLockScreen::clearPassword()
{
    m_passwordBuffer.clear();
    updatePasswordDisplay();
}

void VeridianLockScreen::updatePasswordDisplay()
{
    /* Show masked dots for each character */
    QString masked = QString(m_passwordBuffer.length(), QChar(0x2022));  /* bullet */

    for (auto *surface : m_surfaces) {
        surface->setPasswordText(masked);
        surface->setStatusMessage(QString(), false);
        surface->render();
    }
}

bool VeridianLockScreen::acquireSessionLock()
{
    /* In a full implementation, this would:
     * 1. Connect to the Wayland display
     * 2. Bind ext_session_lock_manager_v1
     * 3. Call ext_session_lock_manager_v1_lock()
     * 4. Wait for the lock event
     *
     * The VeridianOS KWin implementation (Sprint 9.8) provides
     * ext-session-lock-v1 protocol support. */
    return true;
}

void VeridianLockScreen::releaseSessionLock()
{
    /* ext_session_lock_v1_unlock_and_destroy() */
    m_sessionLock = nullptr;
}

void VeridianLockScreen::createLockSurfaces()
{
    /* Create one lock surface per output */
    QList<QScreen *> screens = QGuiApplication::screens();
    for (QScreen *screen : screens) {
        QSize size = screen->size();

        /* In the full implementation, these would be actual Wayland objects */
        auto *surface = new VeridianLockSurface(
            nullptr, nullptr,
            size.width(), size.height(),
            m_config, this);

        surface->setUserInfo(m_authenticator->userDisplayName(),
                             m_authenticator->userAvatarPath());

        connect(surface, &VeridianLockSurface::needsRedraw,
                surface, &VeridianLockSurface::render);

        surface->render();
        m_surfaces.append(surface);
    }
}

void VeridianLockScreen::destroyLockSurfaces()
{
    qDeleteAll(m_surfaces);
    m_surfaces.clear();
}

void VeridianLockScreen::onIdleTimeout()
{
    if (!m_locked)
        lock();
}

void VeridianLockScreen::onClockTick()
{
    for (auto *surface : m_surfaces)
        surface->updateClock();
}

void VeridianLockScreen::onAuthSuccess()
{
    m_clockTimer->stop();
    m_locked = false;
    m_visible = false;
    m_lastUnlockTime = QDateTime::currentDateTime();

    destroyLockSurfaces();
    releaseSessionLock();
    clearPassword();

    Q_EMIT unlocked();
    Q_EMIT lockStateChanged(false);

    qDebug("LockScreen: session unlocked");
}

void VeridianLockScreen::onAuthFailed(int attempt)
{
    QString message;
    if (m_authenticator->isLockedOut()) {
        message = QStringLiteral("Too many attempts. Try again in %1 seconds.")
                      .arg(m_authenticator->lockoutRemainingSeconds());
    } else {
        message = QStringLiteral("Incorrect password (attempt %1/%2)")
                      .arg(attempt)
                      .arg(m_config.maxPasswordAttempts);
    }

    clearPassword();

    for (auto *surface : m_surfaces) {
        surface->setStatusMessage(message, true);
        surface->render();
    }

    qDebug("LockScreen: authentication failed (attempt %d)", attempt);
}

} /* namespace Plasma */

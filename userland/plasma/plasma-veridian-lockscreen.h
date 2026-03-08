/*
 * VeridianOS -- plasma-veridian-lockscreen.h
 *
 * Copyright (c) 2025-2026 VeridianOS Contributors
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Lock screen integration for KDE Plasma on VeridianOS.  Provides
 * PAM-like authentication against the VeridianOS UserDatabase, renders
 * the lock screen overlay via ext-session-lock-v1 Wayland protocol,
 * and handles the unlock UI (password field, user avatar, clock).
 *
 * Responsibilities:
 *   - Session locking via ext-session-lock-v1 protocol
 *   - Password verification against UserDatabase (/etc/passwd + /etc/shadow)
 *   - Lock screen rendering (wallpaper, clock, user info, password field)
 *   - Keyboard input capture in locked state
 *   - Idle-triggered automatic locking
 *   - D-Bus interface for lock/unlock control
 *   - Grace period for immediate re-lock prevention
 *
 * This module is loaded by plasma-workspace's ksmserver for screen
 * locking functionality.
 */

#ifndef PLASMA_VERIDIAN_LOCKSCREEN_H
#define PLASMA_VERIDIAN_LOCKSCREEN_H

#include <QObject>
#include <QString>
#include <QTimer>
#include <QDateTime>
#include <QImage>
#include <QColor>
#include <QFont>
#include <QDBusInterface>

/* Wayland protocol headers */
struct wl_display;
struct wl_surface;
struct wl_output;
struct ext_session_lock_manager_v1;
struct ext_session_lock_v1;
struct ext_session_lock_surface_v1;

namespace Plasma {

/* ========================================================================= */
/* LockScreenConfig -- lock screen appearance settings                       */
/* ========================================================================= */

struct LockScreenConfig
{
    /* Appearance */
    QString wallpaperPath;      /* background image or empty for solid color */
    QColor backgroundColor;     /* fallback background color */
    bool showClock;
    bool showDate;
    bool showUserAvatar;
    bool showUserName;
    QString clockFormat;        /* "HH:mm" or "hh:mm AP" */
    QString dateFormat;         /* "dddd, MMMM d" */

    /* Fonts */
    QFont clockFont;
    QFont dateFont;
    QFont userNameFont;
    QFont passwordFont;
    QFont messageFont;

    /* Colors */
    QColor clockColor;
    QColor dateColor;
    QColor userNameColor;
    QColor inputFieldBackground;
    QColor inputFieldBorder;
    QColor inputFieldText;
    QColor inputFieldPlaceholder;
    QColor errorColor;

    /* Behavior */
    int lockAfterIdleSeconds;   /* 0 = disabled */
    int gracePeriodSeconds;     /* time after unlock before re-lock allowed */
    bool lockOnSuspend;
    bool lockOnSwitchUser;
    int maxPasswordAttempts;    /* 0 = unlimited */
    int lockoutDurationSeconds; /* after max attempts exceeded */

    /* Input field dimensions */
    int inputFieldWidth;
    int inputFieldHeight;
    int inputFieldRadius;       /* corner radius */
};

/* ========================================================================= */
/* VeridianAuthenticator -- password verification                            */
/* ========================================================================= */

/**
 * Verifies user passwords against the VeridianOS UserDatabase.
 *
 * On VeridianOS, user accounts are stored in:
 *   /etc/passwd (username, UID, GID, home, shell)
 *   /etc/shadow (hashed password with salt)
 *
 * Password verification uses the same Hash256 + salt scheme as the
 * kernel's UserDatabase (v0.20.2), with constant-time comparison
 * to prevent timing attacks.
 */
class VeridianAuthenticator : public QObject
{
    Q_OBJECT

public:
    explicit VeridianAuthenticator(QObject *parent = nullptr);
    ~VeridianAuthenticator() override;

    /* ----- Authentication ----- */
    bool authenticate(const QString &username, const QString &password);
    bool isLocked() const;
    int failedAttempts() const;
    void resetFailedAttempts();

    /* ----- User info ----- */
    QString currentUser() const;
    QString userDisplayName() const;
    QString userAvatarPath() const;
    int userId() const;

    /* ----- Lockout ----- */
    bool isLockedOut() const;
    int lockoutRemainingSeconds() const;

Q_SIGNALS:
    void authenticationSucceeded();
    void authenticationFailed(int attemptNumber);
    void lockedOut(int durationSeconds);

private:
    bool verifyPasswordHash(const QString &password,
                            const QString &storedHash,
                            const QByteArray &salt) const;
    bool readShadowEntry(const QString &username,
                         QString &hash, QByteArray &salt) const;

    QString m_currentUser;
    QString m_displayName;
    QString m_avatarPath;
    int m_userId;
    int m_failedAttempts;
    int m_maxAttempts;
    QDateTime m_lockoutUntil;
    int m_lockoutDuration;
};

/* ========================================================================= */
/* VeridianLockSurface -- lock screen rendering surface                      */
/* ========================================================================= */

/**
 * Renders the lock screen UI on a Wayland surface obtained via the
 * ext-session-lock-v1 protocol.  One surface is created per output
 * (multi-monitor support).
 *
 * The lock screen layout (top to bottom):
 *   - Clock (centered, large font)
 *   - Date (centered, below clock)
 *   - User avatar (centered circle, optional)
 *   - User name (centered, below avatar)
 *   - Password input field (centered, below name)
 *   - Status message (centered, below field -- errors, "Unlocking...")
 */
class VeridianLockSurface : public QObject
{
    Q_OBJECT

public:
    explicit VeridianLockSurface(struct wl_surface *surface,
                                  struct ext_session_lock_surface_v1 *lockSurface,
                                  int width, int height,
                                  const LockScreenConfig &config,
                                  QObject *parent = nullptr);
    ~VeridianLockSurface() override;

    /* ----- Rendering ----- */
    void render();
    void setUserInfo(const QString &userName, const QString &avatarPath);
    void setPasswordText(const QString &masked);
    void setStatusMessage(const QString &message, bool isError);
    void updateClock();

    /* ----- Geometry ----- */
    int width() const { return m_width; }
    int height() const { return m_height; }
    void resize(int width, int height);

    /* ----- Wayland surface ----- */
    struct wl_surface *waylandSurface() const { return m_wlSurface; }

Q_SIGNALS:
    void needsRedraw();

private:
    void paintBackground(QImage &image);
    void paintClock(QImage &image);
    void paintUserInfo(QImage &image);
    void paintPasswordField(QImage &image);
    void paintStatusMessage(QImage &image);
    void submitBuffer(const QImage &image);

    struct wl_surface *m_wlSurface;
    struct ext_session_lock_surface_v1 *m_lockSurface;
    int m_width;
    int m_height;
    LockScreenConfig m_config;

    /* Rendering state */
    QString m_userName;
    QString m_avatarPath;
    QImage m_avatarImage;
    QString m_passwordMasked;
    QString m_statusMessage;
    bool m_statusIsError;
    QImage m_wallpaperImage;
};

/* ========================================================================= */
/* VeridianLockScreen -- main lock screen controller                         */
/* ========================================================================= */

/**
 * Controls the lock screen lifecycle:
 *   1. Acquires ext-session-lock-v1 from compositor
 *   2. Creates lock surfaces for each output
 *   3. Captures keyboard input
 *   4. Delegates authentication to VeridianAuthenticator
 *   5. Releases lock on successful authentication
 *
 * D-Bus interface: org.kde.screensaver
 *   - Lock()           lock the screen
 *   - SimulateUserActivity()  reset idle timer
 *   - GetActive()      returns true if locked
 *   - SetActive(bool)  lock or unlock
 */
class VeridianLockScreen : public QObject
{
    Q_OBJECT

public:
    explicit VeridianLockScreen(QObject *parent = nullptr);
    ~VeridianLockScreen() override;

    /* ----- Lock control ----- */
    bool lock();
    bool unlock(const QString &password);
    bool isLocked() const;
    bool isVisible() const;

    /* ----- Configuration ----- */
    void loadConfig();
    LockScreenConfig config() const { return m_config; }

    /* ----- Idle management ----- */
    void setIdleLockEnabled(bool enabled);
    bool isIdleLockEnabled() const;
    void resetIdleTimer();

    /* ----- D-Bus interface ----- */
    bool registerDBus();

    /* ----- Input handling ----- */
    void handleKeyPress(uint32_t key, const QString &text);
    void handleKeyRelease(uint32_t key);

Q_SIGNALS:
    void locked();
    void unlocked();
    void lockStateChanged(bool locked);
    void passwordRequired();

private Q_SLOTS:
    void onIdleTimeout();
    void onClockTick();
    void onAuthSuccess();
    void onAuthFailed(int attempt);

private:
    /* ----- Wayland protocol ----- */
    bool acquireSessionLock();
    void releaseSessionLock();
    void createLockSurfaces();
    void destroyLockSurfaces();

    /* ----- Input processing ----- */
    void appendPasswordChar(const QString &ch);
    void deletePasswordChar();
    void submitPassword();
    void clearPassword();
    void updatePasswordDisplay();

    /* ----- State ----- */
    bool m_locked;
    bool m_visible;
    QString m_passwordBuffer;
    LockScreenConfig m_config;

    /* ----- Components ----- */
    VeridianAuthenticator *m_authenticator;
    QVector<VeridianLockSurface *> m_surfaces;
    QTimer *m_idleTimer;
    QTimer *m_clockTimer;

    /* ----- Wayland objects ----- */
    struct wl_display *m_wlDisplay;
    struct ext_session_lock_manager_v1 *m_lockManager;
    struct ext_session_lock_v1 *m_sessionLock;

    /* ----- Grace period ----- */
    QDateTime m_lastUnlockTime;
};

} /* namespace Plasma */

#endif /* PLASMA_VERIDIAN_LOCKSCREEN_H */
